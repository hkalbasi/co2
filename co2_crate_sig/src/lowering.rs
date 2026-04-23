use std::{cell::RefCell, collections::HashMap, panic::AssertUnwindSafe, rc::Rc, sync::Arc};

use co2_ast::{
    Declaration, DeclarationSpecifier, Designator, DoTransform as _, FunctionDefinitionSignature,
    InitDeclarator, StatelessResolver, StorageClassSpecifier, StructOrUnionKind, TranslationUnit,
    TypeQualifier, TypeResolver,
};
use co2_parser::parse_compound_statement;
use co2_preprocessor::PreprocessedSource;
use rustc_public_generative::{
    AdtRepr, DefData, FileId, ForeignModItem, FunctionAbi, FunctionSignature, HirAdtKind, HirGenericArg, HirImplItem, HirImplItemKind, HirLifetime, HirModule, HirModuleItem, HirSelfKind, HirStructure, HirStructureCtx, HirTy, HirTyConst, rustc_public::{
        DefId,
        ty::{AdtDef, FnDef, IntTy},
    }
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
    pub maybe_uninit_uninit: FnDef,
    pub valist: AdtDef,
    pub valist_fn_arg: FnDef,
    pub clone: FnDef,
    pub transmute: FnDef,
    pub transmute_copy: FnDef,
    pub offset_mut: FnDef,
    pub offset_const: FnDef,
    pub offset_from: FnDef,
    pub zeroed: FnDef,
    pub str_as_ptr: FnDef,
}

fn has_const_qualifier_in_decl_specs(
    specs: &[co2_ast::Spanned<DeclarationSpecifier<LocalResolver>>],
) -> bool {
    specs.iter().any(|(spec, _)| {
        matches!(
            spec,
            DeclarationSpecifier::TypeQualifier((TypeQualifier::Const, _))
        )
    })
}

fn deduplicate_tu_items(
    mut tu: TranslationUnit<StatelessResolver>,
) -> TranslationUnit<StatelessResolver> {
    let mut tu_item_id: usize = 0;
    let mut name_to_important_def = HashMap::new();

    for (item, _) in &tu.items {
        match item {
            Declaration::FunctionDefinition { signature, .. } => {
                let name = signature.ident().unwrap();
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
        Declaration::FunctionDefinition { signature, .. } => {
            let name = signature.ident().unwrap();
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
    preprocessed: Arc<PreprocessedSource>,
    file_ids: Arc<Vec<FileId>>,
    no_main: bool,
) -> (HirStructure, HashMap<DefId, MirOwnerInfo>, WellknownDefs) {
    let span = ctx.span_in_file(file_id, 0, 0);
    let deps = ctx.dependencies();

    let tu =
        co2_parser::parse_translation_unit(source_name.clone(), src_static, StatelessResolver::new())
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
            local_tys: HashMap::new(),
            pending_typedefs: vec![],
            pending_static: vec![],
            array_len_consts: HashMap::new(),
            array_len_const_exprs: HashMap::new(),
            hir_ctx: unsafe { std::mem::transmute(ctx) },
            file_id,
            preprocessed: preprocessed.clone(),
            file_ids: file_ids.clone(),
            source_name: source_name.clone(),
            source: src_static,
            struct_manager: StructManager::default(),
            unrepresentable_typedefs: HashMap::new(),
            typedef_tys: HashMap::new(),
            global_value_tys: HashMap::new(),
            global_struct_tags: Rc::new(RefCell::new(StructAndEnumData::default())),
            global_locals: Rc::new(RefCell::new(im::HashMap::new())),
        })),
        hir_ctx: ctx,
        source_name,
        source: src_static,
        file_id,
        preprocessed,
        file_ids,
        mir_owners: HashMap::new(),
        hir_items: vec![],
    };

    {
        let adt = ctx.resolve("core::ffi::VaList").unwrap().0;
        let ty = HirTy::adt(
            adt,
            vec![HirGenericArg::Lifetime(HirLifetime::Static)],
            span,
        );
        for name in ["__builtin_va_list", "__gnuc_va_list"] {
            let Ok((id, _)) = ctx.resolve(name) else {
                continue;
            };
            ctx.resolver.borrow_mut().typedef_tys.insert(id, ty.clone());
            ctx.hir_items.push(HirModuleItem::TypeDef {
                name: name.to_owned(),
                id,
                span,
                ty: ty.clone(),
            });
        }
    }

    for (item, parser_span) in tu.items {
        let span = ctx.co2_span_to_rustc(parser_span);
        let mut resolver = LocalResolver::new(ctx.resolver.clone());
        let item = item.transform(&resolver);
        match item {
            Declaration::FunctionDefinition {
                signature,
                body,
            } => {
                let (name, sig, param_names, no_mangle) = match signature {
                    FunctionDefinitionSignature::C {
                        declaration_specifiers,
                        declarator,
                    } => {
                        let is_static = declaration_specifiers.iter().any(|spec| spec.0.is_static());
                        let transformed_specs = declaration_specifiers;
                        let base_const = has_const_qualifier_in_decl_specs(&transformed_specs);
                        let base = ctx.base_ty_of_decl(transformed_specs, parser_span);
                        let (name, sig, param_names) = ctx
                            .lower_function_signature(base, base_const, declarator)
                            .expect("failed to lower function signature");
                        (name, sig, param_names, !is_static)
                    }
                    FunctionDefinitionSignature::Rust(sig) => {
                        let (name, lower_sig, param_names) = ctx.lower_rust_function_signature(sig);
                        (name, lower_sig, param_names, false)
                    }
                };

                let id = ctx.resolve_in_current([&*name]).unwrap().0;
                let function_name = name.clone();
                let param_tys = sig.inputs.clone();
                let id = FnDef(id);
                ctx.hir_items.push(HirModuleItem::Function {
                    name,
                    id,
                    sig,
                    no_mangle,
                    span,
                });
                resolver = resolver.start_new_scope().with_owner(id.0);
                let param_names = param_names
                    .into_iter()
                    .zip(param_tys.into_iter())
                    .map(|(name, ty)| {
                        let id = resolver.add_local(name.clone());
                        resolver.base.borrow_mut().set_local_ty(id as u32, ty);
                        (id, name)
                    })
                    .collect();
                let parsed_body = std::panic::catch_unwind(AssertUnwindSafe(|| {
                    parse_compound_statement(
                        &body.0.tokens.0,
                        ctx.source_name.clone(),
                        ctx.source,
                        resolver.clone(),
                    )
                }));

                let mir_owner = match parsed_body {
                    Ok(body) => MirOwnerInfo::Fn {
                        def: id,
                        function_name,
                        param_names,
                        resolver: resolver.clone(),
                        body,
                    },
                    Err(payload) => {
                        if co2_ast::is_diagnostic_abort(payload.as_ref()) {
                            MirOwnerInfo::FnBodyError {
                                def: id,
                                body_span: body.1,
                            }
                        } else {
                            std::panic::resume_unwind(payload);
                        }
                    }
                };

                ctx.mir_owners.insert(id.0, mir_owner);
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

                let transformed_specs = cleaned_specs;
                let base_const = has_const_qualifier_in_decl_specs(&transformed_specs);
                let base = ctx.base_ty_of_decl(transformed_specs, parser_span);

                for init in declarators {
                    let InitDeclarator {
                        declarator,
                        initializer,
                    } = init.0;

                    let (name, ty, array_len) =
                        ctx.lower_value_decl_ctype(base.clone(), base_const, declarator, &resolver);

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
                        ctx.resolver
                            .borrow_mut()
                            .typedef_tys
                            .insert(type_def, ty.clone());
                        ctx.hir_items.push(HirModuleItem::TypeDef {
                            name,
                            id: type_def,
                            span,
                            ty,
                        });
                        continue;
                    }

                    if let CTy::Ty(ty) = &ty {
                        let id = ctx.resolve_in_current([&*name]).unwrap().0;
                        ctx.resolver
                            .borrow_mut()
                            .global_value_tys
                            .insert(id, ty.clone());
                    }

                    match ty {
                        CTy::Ty(ty) => {
                            let (id, _) = ctx.resolve_in_current([&*name]).unwrap();
                            if let Some(initializer) = initializer {
                                ctx.mir_owners.insert(
                                    id,
                                    match array_len {
                                        Some(array_len) => MirOwnerInfo::StaticWithArrayLen {
                                            initializer,
                                            array_len,
                                        },
                                        None => MirOwnerInfo::Static { initializer },
                                    },
                                );
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
                            if let Some(initializer) = initializer {
                                ctx.mir_owners.insert(
                                    id,
                                    MirOwnerInfo::Static {
                                        initializer: initializer.clone(),
                                    },
                                );
                                let len = infer_unsized_array_len(&initializer.0, &resolver, &elem_ty)
                                    .unwrap_or_else(|err| {
                                        ctx.terminate_with_error(parser_span, &err)
                                    });
                                let ty = HirTy::new_array(elem_ty, HirTyConst::Literal(len), span);
                                ctx.resolver.borrow_mut().global_value_tys.insert(id, ty.clone());
                                ctx.hir_items.push(HirModuleItem::Static {
                                    name,
                                    id,
                                    ty,
                                    mutable: true,
                                    span,
                                });
                            } else if is_extern {
                                let ty = HirTy::new_array(elem_ty, HirTyConst::Literal(0), span);
                                ctx.resolver.borrow_mut().global_value_tys.insert(id, ty.clone());
                                foreign_items.push(ForeignModItem::ForeignStatic {
                                    name,
                                    id,
                                    ty,
                                    mutable: true,
                                    span,
                                });
                            } else {
                                ctx.terminate_with_error(
                                    parser_span,
                                    "static with unsized array type should have initializer",
                                );
                            }
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
        let base_const = has_const_qualifier_in_decl_specs(&specifiers);
        let ty = ctx.base_ty_of_decl(specifiers, parser_span);
        let resolver = LocalResolver::new(ctx.resolver.clone());
        let (_, ty, _) =
            ctx.lower_value_decl_ctype(ty, base_const, (declarator, parser_span), &resolver);
        let CTy::Ty(ty) = ty else {
            ctx.terminate_with_error(parser_span, "typedef did not lower to a first-class type");
        };
        ctx.resolver.borrow_mut().typedef_tys.insert(id, ty.clone());
        ctx.hir_items
            .push(HirModuleItem::TypeDef { name, id, ty, span });
    }

    let pending_static = std::mem::take(&mut ctx.resolver.borrow_mut().pending_static);
    for (id, name, specifiers, declarator, parser_span) in pending_static {
        let span = ctx.co2_span_to_rustc(parser_span);
        let base_const = has_const_qualifier_in_decl_specs(&specifiers);
        let base_ty = ctx.base_ty_of_decl(specifiers, parser_span);
        let resolver = LocalResolver::new(ctx.resolver.clone());
        let (_, ty, _) =
            ctx.lower_value_decl_ctype(base_ty, base_const, declarator.declarator, &resolver);
        if let CTy::Ty(ty) = &ty {
            ctx.resolver.borrow_mut().global_value_tys.insert(id, ty.clone());
        }
        match ty {
            CTy::Ty(ty) => {
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
            CTy::UnsizedArray(elem_ty) => {
                let initializer = if let Some((initializer, init_span)) = declarator.initializer {
                    (initializer, init_span)
                } else {
                    ctx.terminate_with_error(
                        parser_span,
                        "local static with unsized array type must have an initializer",
                    );
                };
                let len = infer_unsized_array_len(&initializer.0, &resolver, &elem_ty)
                    .unwrap_or_else(|err| ctx.terminate_with_error(parser_span, &err));
                let sized_ty = HirTy::new_array(elem_ty, HirTyConst::Literal(len), span);
                ctx.resolver
                    .borrow_mut()
                    .global_value_tys
                    .insert(id, sized_ty.clone());
                ctx.hir_items.push(HirModuleItem::Static {
                    name,
                    id,
                    ty: sized_ty,
                    span,
                    mutable: true,
                });
                ctx.mir_owners
                    .insert(id, MirOwnerInfo::Static { initializer });
            }
            _ => {
                ctx.terminate_with_error(parser_span, "static did not lower to a first-class type");
            }
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
                repr: AdtRepr::C,
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
            repr: AdtRepr::C,
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
        maybe_uninit_uninit: FnDef(ctx.resolve("core::mem::MaybeUninit::<T>::uninit").unwrap().0),
        valist: AdtDef(ctx.resolve("core::ffi::VaList").unwrap().0),
        valist_fn_arg: FnDef(ctx.resolve("core::ffi::VaList::<'f>::arg").unwrap().0),
        clone: FnDef(ctx.resolve("core::clone::Clone::clone").unwrap().0),
        zeroed: FnDef(ctx.resolve("core::mem::zeroed").unwrap().0),
        transmute: FnDef(ctx.resolve("core::intrinsics::transmute").unwrap().0),
        transmute_copy: FnDef(ctx.resolve("core::mem::transmute_copy").unwrap().0),
        str_as_ptr: FnDef(ctx.resolve("core::str::<impl str>::as_ptr").unwrap().0),
        offset_mut: FnDef(ctx.resolve("core::ptr::mut_ptr::<impl *mut T>::offset").unwrap().0),
        offset_const: FnDef(ctx.resolve("core::ptr::const_ptr::<impl *const T>::offset").unwrap().0),
        offset_from: FnDef(ctx.resolve("core::ptr::const_ptr::<impl *const T>::offset_from").unwrap().0),
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

// TODO: this function is AI garbage and is duplicate logic from what is in co2_hir
fn infer_unsized_array_len(
    initializer: &co2_ast::Initializer<LocalResolver>,
    resolver: &LocalResolver,
    elem_ty: &HirTy,
) -> Result<usize, String> {
    match initializer {
        co2_ast::Initializer::Expr((co2_ast::Expression::Constant(co2_ast::Constant::String(s)), _)) => {
            Ok(s.chars().count() + 1)
        }
        co2_ast::Initializer::List(items) => {
            let slots_per_elem = flattened_scalar_slots(elem_ty, resolver)?;
            let mut next_index = 0usize;
            let mut max_len = 0usize;
            let mut used_slots_in_current = 0usize;
            for (item, _) in items {
                let index = match &item.designators {
                    None => next_index,
                    Some(designators) => match designators.first() {
                        None => next_index,
                        Some((first, _)) => match first {
                            Designator::Subscript(expr) => {
                                let value = {
                                    let mut base = resolver.base.borrow_mut();
                                    let value = base.eval_const_expr(expr)?;
                                    usize::try_from(value).map_err(|_| {
                                        format!("array designator index must be non-negative, got {value}")
                                    })?
                                };
                                value
                            }
                            Designator::Field(_) => {
                                return Err("field designator is invalid for unsized array length inference".to_owned());
                            }
                        },
                    },
                };
                if index != next_index {
                    used_slots_in_current = 0;
                }
                let consumed_slots =
                    consumed_initializer_slots(&item.initializer.0, elem_ty, resolver)?;
                let element_advance = if consumed_slots == 0 { 1 } else { consumed_slots };
                let total_slots = used_slots_in_current + element_advance;
                let fully_covered = total_slots.div_ceil(slots_per_elem);
                max_len = max_len.max(index + fully_covered);
                next_index = index + total_slots / slots_per_elem;
                used_slots_in_current = total_slots % slots_per_elem;
            }
            Ok(max_len)
        }
        _ => Err("static with unsized array type should have list or string initializer".to_owned()),
    }
}

fn consumed_initializer_slots(
    initializer: &co2_ast::Initializer<LocalResolver>,
    target_ty: &HirTy,
    resolver: &LocalResolver,
) -> Result<usize, String> {
    match initializer {
        co2_ast::Initializer::Expr(_) => Ok(1),
        co2_ast::Initializer::List(_) => flattened_scalar_slots(target_ty, resolver),
    }
}

fn flattened_scalar_slots(
    ty: &HirTy,
    resolver: &LocalResolver,
) -> Result<usize, String> {
    match &ty.kind {
        rustc_public_generative::HirTyKind::Bool
        | rustc_public_generative::HirTyKind::Char
        | rustc_public_generative::HirTyKind::Int(_)
        | rustc_public_generative::HirTyKind::Uint(_)
        | rustc_public_generative::HirTyKind::Float(_)
        | rustc_public_generative::HirTyKind::RawPtr(_, _)
        | rustc_public_generative::HirTyKind::Ref(_, _, _)
        | rustc_public_generative::HirTyKind::FnPtr(_) => Ok(1),
        rustc_public_generative::HirTyKind::Array(HirTyConst::Literal(len), inner) => {
            Ok(len * flattened_scalar_slots(inner, resolver)?)
        }
        rustc_public_generative::HirTyKind::Adt(def, _) => {
            let base = resolver.base.borrow();
            if let Some((kind, fields)) = base.adt_layout_info(*def) {
                match kind {
                    StructOrUnionKind::Struct => fields
                        .iter()
                        .try_fold(0usize, |acc, field| {
                            Ok(acc + flattened_scalar_slots(field, resolver)?)
                        }),
                    StructOrUnionKind::Union => fields
                        .first()
                        .map(|field| flattened_scalar_slots(field, resolver))
                        .unwrap_or(Ok(1)),
                }
            } else if let Some(aliased) = base.typedef_tys.get(def) {
                flattened_scalar_slots(aliased, resolver)
            } else {
                // Unknown ADT (e.g. MaybeUninit<fn(...)> for function pointers, or
                // other Rust stdlib types): opaque to our layout model, counts as one
                // initializer slot just like any scalar.
                Ok(1)
            }
        }
        // Tuple (including unit `()`) and any other opaque types: treat as one slot.
        _ => Ok(1),
    }
}
