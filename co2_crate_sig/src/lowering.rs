use std::{cell::RefCell, collections::HashMap, rc::Rc};

use co2_ast::{
    Declaration, DeclarationSpecifier, DoTransform as _, InitDeclarator, StatelessResolver,
    StorageClassSpecifier, StructOrUnionKind, TranslationUnit, TypeResolver,
};
use co2_parser::parse_compound_statement;
use rustc_public_generative::{
    DefData, FileId, ForeignModItem, FunctionAbi, FunctionSignature, HirAdtKind, HirGenericArg,
    HirImplItem, HirImplItemKind, HirLifetime, HirModule, HirModuleItem, HirSelfKind, HirStructure,
    HirStructureCtx, HirTy, HirTyConst,
    rustc_public::{
        DefId,
        ty::{AdtDef, FnDef, IntTy},
    },
};

use crate::{
    CrateSigCtx, LocalResolver, LocalResolverBase, MirOwnerInfo,
    ast_resolver::StructAndEnumData,
    resolver::Resolver,
    struct_manager::{PendingEnum, StructData, StructManager},
    ty::CTy,
};

#[derive(Clone, Copy)]
pub struct WellknownDefs {
    pub maybe_uninit: AdtDef,
    pub valist: AdtDef,
    pub valist_fn_arg: FnDef,
    pub zeroed: FnDef,
}

fn deduplicate_tu_items(
    mut tu: TranslationUnit<StatelessResolver>,
) -> TranslationUnit<StatelessResolver> {
    let mut tu_item_id: usize = 0;
    let mut name_to_important_def = HashMap::new();

    for (item, _) in &tu.items {
        match item {
            Declaration::FunctionDefinition { declarator, .. } => {
                let name = declarator.0.ident().unwrap();
                name_to_important_def.insert(name, (tu_item_id, 3));
                tu_item_id += 1;
            }
            Declaration::Declaration {
                declaration_specifiers,
                declarators,
            } => {
                let is_extern = declaration_specifiers.iter().any(|x| x.0.is_extern());
                for decl in declarators {
                    let prio = if decl.0.initializer.is_some() {
                        2
                    } else if is_extern {
                        0
                    } else {
                        1
                    };
                    let name = decl.0.declarator.0.ident().unwrap();
                    match name_to_important_def.entry(name) {
                        std::collections::hash_map::Entry::Occupied(mut occupied_entry) => {
                            if occupied_entry.get().1 < prio {
                                *occupied_entry.get_mut() = (tu_item_id, prio);
                            }
                        }
                        std::collections::hash_map::Entry::Vacant(vacant_entry) => {
                            vacant_entry.insert((tu_item_id, prio));
                        }
                    }
                    tu_item_id += 1;
                }
            }
        }
    }

    tu_item_id = 0;

    tu.items.retain_mut(|item| match &mut item.0 {
        Declaration::FunctionDefinition { declarator, .. } => {
            let name = declarator.0.ident().unwrap();
            let is_needed = name_to_important_def[&name].0 == tu_item_id;
            tu_item_id += 1;
            is_needed
        }
        Declaration::Declaration {
            declaration_specifiers: _,
            declarators,
        } => {
            declarators.retain(|decl| {
                let name = decl.0.declarator.0.ident().unwrap();
                let is_needed = name_to_important_def[&name].0 == tu_item_id;
                tu_item_id += 1;
                is_needed
            });
            true
        }
    });

    tu
}

pub fn lower_crate_sig(
    ctx: HirStructureCtx<'_>,
    source_name: String,
    src_static: &'static str,
    file_id: FileId,
    no_main: bool,
) -> (HirStructure, HashMap<DefId, MirOwnerInfo>, WellknownDefs) {
    let span = ctx.span_in_file(file_id, 0, 0);
    let deps = ctx.dependencies();

    let tu = co2_parser::parse_translation_unit(source_name.clone(), src_static, StatelessResolver)
        .expect("failed to parse co2 source")
        .0;

    let tu = deduplicate_tu_items(tu);

    let foreign_mod = ctx.allocate_def_id(ctx.root_crate_def_id(), DefData::ForeignMod);
    let mut foreign_items = Vec::new();

    let ctx = &*Box::leak(Box::new(ctx));

    let mut ctx = CrateSigCtx {
        resolver: Rc::new(RefCell::new(LocalResolverBase {
            resolver: Resolver::new(&ctx, deps, &tu, foreign_mod),
            local_counter: 0,
            fake_defs_counter: 0,
            pending_typedefs: vec![],
            pending_static: vec![],
            hir_ctx: unsafe { std::mem::transmute(ctx) },
            file_id,
            source_name: source_name.clone(),
            source: src_static,
            struct_manager: StructManager::default(),
            unrepresentable_typedefs: HashMap::new(),
            global_struct_tags: Rc::new(RefCell::new(StructAndEnumData::default())),
            global_locals: Rc::new(RefCell::new(im::HashMap::new())),
        })),
        hir_ctx: ctx,
        source_name,
        source: src_static,
        file_id,
        mir_owners: HashMap::new(),
        hir_items: vec![],
    };

    {
        let name = "__builtin_va_list";
        let id = ctx.resolve(name).unwrap().0;
        let adt = ctx.resolve("std::ffi::VaList").unwrap().0;
        let ty = HirTy::adt(
            adt,
            vec![HirGenericArg::Lifetime(HirLifetime::Static)],
            span,
        );
        ctx.hir_items.push(HirModuleItem::TypeDef {
            name: name.to_owned(),
            id,
            span,
            ty,
        });
    }

    for (item, parser_span) in tu.items {
        let span = ctx.co2_span_to_rustc(parser_span);
        match item {
            Declaration::FunctionDefinition {
                declaration_specifiers,
                declarator,
                body,
            } => {
                let mut resolver = LocalResolver::new(ctx.resolver.clone());
                let base =
                    ctx.base_ty_of_decl(declaration_specifiers.transform(&resolver), parser_span);
                let (name, mut sig, param_names) = ctx
                    .lower_function_signature(base, declarator.transform(&resolver))
                    .expect("failed to lower function signature");

                let id = ctx.resolve_in_current([&*name]).unwrap().0;
                if name == "main" && !no_main {
                    sig.abi = FunctionAbi::Rust;
                }
                let id = FnDef(id);
                ctx.hir_items.push(HirModuleItem::Function {
                    name,
                    id,
                    sig,
                    no_mangle: true,
                    span,
                });
                resolver = resolver.start_new_scope();
                let param_names = param_names
                    .into_iter()
                    .map(|name| {
                        let id = resolver.add_local(name.clone());
                        (id, name)
                    })
                    .collect();
                let body = parse_compound_statement(
                    &body.0.tokens.0,
                    ctx.source_name.clone(),
                    ctx.source,
                    resolver,
                );

                ctx.mir_owners.insert(
                    id.0,
                    MirOwnerInfo::Fn {
                        def: id,
                        param_names,
                        body,
                    },
                );
            }
            Declaration::Declaration {
                declaration_specifiers,
                declarators,
            } => {
                let mut is_typedef = false;
                let mut is_extern = false;
                let mut cleaned_specs = Vec::new();
                for (spec, sp) in declaration_specifiers {
                    match spec {
                        DeclarationSpecifier::StorageSpecifier((
                            StorageClassSpecifier::Typedef,
                            _,
                        )) => {
                            is_typedef = true;
                        }
                        DeclarationSpecifier::StorageSpecifier((
                            StorageClassSpecifier::Extern,
                            _,
                        )) => {
                            is_extern = true;
                        }
                        _ => cleaned_specs.push((spec, sp)),
                    }
                }

                let resolver = LocalResolver::new(ctx.resolver.clone());
                let base = ctx.base_ty_of_decl(cleaned_specs.transform(&resolver), parser_span);

                for init in declarators {
                    let InitDeclarator {
                        declarator,
                        initializer,
                    } = init.0;

                    let (name, ty) =
                        ctx.lower_value_decl_ctype(base.clone(), declarator.transform(&resolver));

                    ctx.resolver
                        .borrow()
                        .global_locals
                        .borrow_mut()
                        .remove(&name);

                    if is_typedef {
                        let ty = match ty {
                            CTy::Ty(ty) => ty,
                            _ => {
                                ctx.resolver
                                    .borrow_mut()
                                    .unrepresentable_typedefs
                                    .insert(name, ty);
                                continue;
                            }
                        };
                        let type_def = ctx.resolve_in_current([&*name]).unwrap().0;
                        ctx.hir_items.push(HirModuleItem::TypeDef {
                            name,
                            id: type_def,
                            span,
                            ty,
                        });
                        continue;
                    }

                    match ty {
                        CTy::Ty(ty) => {
                            let (id, _) = ctx.resolve_in_current([&*name]).unwrap();
                            if let Some((initializer, span)) = initializer {
                                let initializer = initializer.transform(&resolver);
                                let initializer = (initializer, span);
                                ctx.mir_owners
                                    .insert(id, MirOwnerInfo::Static { initializer });
                            } else {
                                ctx.mir_owners.insert(id, MirOwnerInfo::StaticZeroed);
                            }
                            if is_extern {
                                foreign_items.push(ForeignModItem::ForeignStatic {
                                    name,
                                    id,
                                    ty,
                                    mutable: true,
                                    span,
                                });
                            } else {
                                ctx.hir_items.push(HirModuleItem::Static {
                                    name,
                                    id,
                                    ty,
                                    mutable: true,
                                    span,
                                });
                            }
                        }
                        CTy::UnsizedArray(elem_ty) => {
                            let (id, _) = ctx.resolve_in_current([&*name]).unwrap();
                            let (len_id, len_name) =
                                ctx.resolver.borrow_mut().emit_fake_def(DefData::ValueNs);
                            let len_rhs = ctx.allocate_def_id(len_id, DefData::AnonConst);
                            if let Some((initializer, span)) = initializer {
                                let initializer = initializer.transform(&resolver);
                                let initializer = (initializer, span);
                                ctx.mir_owners
                                    .insert(id, MirOwnerInfo::Static { initializer });
                            } else {
                                ctx.terminate_with_error(
                                    parser_span,
                                    "static with unsized array type should have initializer",
                                );
                            }
                            let ty = HirTy::new_array(elem_ty, HirTyConst::ConstDef(len_id), span);
                            ctx.hir_items.push(HirModuleItem::Static {
                                name,
                                id,
                                ty,
                                mutable: true,
                                span,
                            });
                            ctx.hir_items.push(HirModuleItem::Const {
                                name: len_name,
                                id: len_id,
                                ty: HirTy::usize_ty(span),
                                rhs: len_rhs,
                                span,
                            });
                            ctx.mir_owners
                                .insert(len_rhs, MirOwnerInfo::StaticArraySizeInference { span });
                        }
                        CTy::Function(sig) => {
                            let def_id = ctx.resolve_in_current([&*name]).unwrap().0;
                            let span = ctx.co2_span_to_rustc(init.1);
                            foreign_items.push(ForeignModItem::ForeignFunction {
                                name,
                                id: FnDef(def_id),
                                sig,
                                span,
                            });
                        }
                    }
                }
            }
        }
    }

    ctx.hir_items.push(HirModuleItem::ForeignMod {
        id: foreign_mod,
        items: foreign_items,
    });

    let clone_trait = ctx.resolve("core::clone::Clone").unwrap().0;
    let copy_trait = ctx.resolve("core::marker::Copy").unwrap().0;
    let clone_trait_fn = ctx.resolve("core::clone::Clone::clone").unwrap().0;

    let pending_typedefs = std::mem::take(&mut ctx.resolver.borrow_mut().pending_typedefs);
    for (id, name, specifiers, declarator, parser_span) in pending_typedefs {
        let span = ctx.co2_span_to_rustc(parser_span);
        let ty = ctx.base_ty_of_decl(specifiers, parser_span);
        let (_, ty) = ctx.lower_value_decl_type(ty, (declarator, parser_span));
        ctx.hir_items
            .push(HirModuleItem::TypeDef { name, id, ty, span });
    }

    let pending_static = std::mem::take(&mut ctx.resolver.borrow_mut().pending_static);
    for (id, name, specifiers, declarator, parser_span) in pending_static {
        let span = ctx.co2_span_to_rustc(parser_span);
        let ty = ctx.base_ty_of_decl(specifiers, parser_span);
        let (_, ty) = ctx.lower_value_decl_type(ty, declarator.declarator);
        ctx.hir_items.push(HirModuleItem::Static {
            name,
            id,
            ty,
            span,
            mutable: true,
        });
        if let Some(initializer) = declarator.initializer {
            ctx.mir_owners
                .insert(id, MirOwnerInfo::Static { initializer });
        } else {
            ctx.mir_owners.insert(id, MirOwnerInfo::StaticZeroed);
        }
    }

    let structs = ctx.resolver.borrow_mut().emit_structs().collect::<Vec<_>>();
    for StructData {
        def_id: def,
        name,
        kind,
        fields,
        span,
    } in structs
    {
        let Some(fields) = fields else {
            // TODO: lower to extern types
            ctx.hir_items.push(HirModuleItem::Adt {
                name,
                id: AdtDef(def),
                kind: HirAdtKind::Struct { fields: vec![] },
                span,
            });
            continue;
        };
        let kind = match kind {
            StructOrUnionKind::Struct => HirAdtKind::Struct { fields },
            StructOrUnionKind::Union => HirAdtKind::Union { fields },
        };

        ctx.hir_items.push(HirModuleItem::Adt {
            name,
            id: AdtDef(def),
            kind,
            span,
        });

        let self_ty_hir = HirTy::adt(def, vec![], span);

        let root_crate = ctx.root_crate_def_id();
        let clone_impl_def = ctx.allocate_def_id(root_crate, DefData::Impl);
        let clone_method_def =
            ctx.allocate_def_id(clone_impl_def, DefData::ValueNs("clone".to_owned()));
        let clone_self_lifetime =
            ctx.allocate_def_id(clone_method_def, DefData::LifetimeNs("a".to_owned()));
        let clone_sig = FunctionSignature {
            lifetimes: vec![clone_self_lifetime],
            inputs: Vec::new(),
            output: self_ty_hir.clone(),
            abi: FunctionAbi::Rust,
            is_unsafe: false,
            c_variadic: false,
        };
        ctx.hir_items.push(HirModuleItem::Impl {
            id: clone_impl_def,
            self_ty: self_ty_hir.clone(),
            trait_def: Some(clone_trait),
            items: vec![HirImplItem {
                name: "clone".to_owned(),
                id: clone_method_def,
                kind: HirImplItemKind::Fn {
                    sig: clone_sig,
                    self_kind: HirSelfKind::RefImm(HirLifetime::Param(clone_self_lifetime)),
                    trait_item_def_id: Some(clone_trait_fn),
                },
                span,
            }],
            span,
        });
        ctx.mir_owners
            .insert(clone_method_def, MirOwnerInfo::CloneMethod(AdtDef(def)));

        let copy_impl_def = ctx.allocate_def_id(root_crate, DefData::Impl);
        ctx.hir_items.push(HirModuleItem::Impl {
            id: copy_impl_def,
            self_ty: self_ty_hir.clone(),
            trait_def: Some(copy_trait),
            items: Vec::new(),
            span,
        });
    }

    let enums = ctx.resolver.borrow_mut().emit_enums().collect::<Vec<_>>();
    for PendingEnum {
        name,
        def_id,
        mir_info,
    } in enums
    {
        ctx.hir_items
            .push(rustc_public_generative::HirModuleItem::Static {
                name: name.clone(),
                id: def_id,
                ty: HirTy::signed_ty(IntTy::I32, span),
                mutable: false,
                span,
            });
        ctx.mir_owners.insert(def_id, mir_info);
    }

    let defs = WellknownDefs {
        maybe_uninit: AdtDef(ctx.resolve("core::mem::MaybeUninit").unwrap().0),
        valist: AdtDef(ctx.resolve("core::ffi::VaList").unwrap().0),
        valist_fn_arg: FnDef(ctx.resolve("core::ffi::VaList::<'f>::arg").unwrap().0),
        zeroed: FnDef(ctx.resolve("core::mem::zeroed").unwrap().0),
    };
    (
        HirStructure {
            root: HirModule {
                span,
                items: ctx.hir_items,
            },
            no_main,
        },
        ctx.mir_owners,
        defs,
    )
}
