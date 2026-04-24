use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    panic::AssertUnwindSafe,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
};

use co2_ast::{
    Declaration, DeclarationSpecifier, Designator, DoTransform as _, FunctionDefinitionSignature,
    InitDeclarator, StatelessResolver, StorageClassSpecifier, StructOrUnionKind, TranslationUnit,
    TypeQualifier, TypeResolver,
};
use co2_parser::{parse_compound_statement, parse_translation_unit};
use co2_preprocessor::PreprocessedSource;
use rustc_public_generative::{
    AdtRepr, DefData, FileId, ForeignModItem, FunctionAbi, FunctionSignature, HirAdtKind,
    HirGenericArg, HirImplItem, HirImplItemKind, HirLifetime, HirModule, HirModuleItem,
    HirSelfKind, HirStructure, HirStructureCtx, HirTy, HirTyConst,
    rustc_public::{
        DefId,
        ty::{AdtDef, FnDef, IntTy},
    },
};

use crate::{
    CrateSigCtx, LocalResolver, LocalResolverBase, MirOwnerInfo,
    ast_resolver::StructAndEnumData,
    resolver::{ModuleData, Resolver},
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

#[derive(Clone)]
struct LoadedModule {
    name: String,
    def_id: DefId,
    decl_span: co2_ast::Span,
    source_name: String,
    source: &'static str,
    tu: TranslationUnit<StatelessResolver>,
    children: Vec<LoadedModule>,
}

struct SourceMapSnapshot {
    files: Arc<HashMap<co2_ast::FileId, (String, Arc<str>)>>,
}

impl co2_ast::SourceMap for SourceMapSnapshot {
    fn get_file_info(&self, id: co2_ast::FileId) -> Option<(String, Arc<str>)> {
        self.files.get(&id).cloned()
    }
}

fn root_module_dir(source_path: &Path) -> PathBuf {
    source_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}

fn child_module_dir(source_path: &Path) -> PathBuf {
    if source_path.file_stem().and_then(|stem| stem.to_str()) == Some("mod") {
        source_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()
    } else {
        source_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(source_path.file_stem().unwrap_or_default())
    }
}

fn resolve_module_source(module_dir: &Path, module_name: &str) -> PathBuf {
    let direct = module_dir.join(format!("{module_name}.co2"));
    if direct.is_file() {
        return direct;
    }

    let nested = module_dir.join(module_name).join("mod.co2");
    if nested.is_file() {
        return nested;
    }

    panic!(
        "failed to resolve module `{module_name}` in {}",
        module_dir.display()
    );
}

fn register_preprocessed_files(
    ctx: &HirStructureCtx<'_>,
    preprocessed: &PreprocessedSource,
    rustc_file_ids: &mut HashMap<co2_ast::FileId, FileId>,
    source_files: &mut HashMap<co2_ast::FileId, (String, Arc<str>)>,
) {
    for (file_id, file) in preprocessed.files() {
        source_files
            .entry(*file_id)
            .or_insert_with(|| (file.path.display().to_string(), file.source.clone()));
        rustc_file_ids
            .entry(*file_id)
            .or_insert_with(|| ctx.add_custom_file(&file.path, file.source.as_ref()));
    }
}

fn load_modules(
    ctx: &HirStructureCtx<'_>,
    parent_def: DefId,
    module_dir: &Path,
    rust_mod_items: &[co2_ast::Spanned<co2_ast::ModItem>],
    rustc_file_ids: &mut HashMap<co2_ast::FileId, FileId>,
    source_files: &mut HashMap<co2_ast::FileId, (String, Arc<str>)>,
    loaded_paths: &mut HashSet<PathBuf>,
) -> Vec<LoadedModule> {
    let mut modules = Vec::with_capacity(rust_mod_items.len());

    for (mod_item, mod_span) in rust_mod_items {
        let module_path = resolve_module_source(module_dir, &mod_item.name.0);
        if !loaded_paths.insert(module_path.clone()) {
            panic!("module loaded multiple times: {}", module_path.display());
        }
        let def_id = ctx.allocate_def_id(parent_def, DefData::Module(mod_item.name.0.clone()));

        let preprocessed = co2_preprocessor::preprocess(&module_path, &Vec::new());
        register_preprocessed_files(ctx, &preprocessed, rustc_file_ids, source_files);

        let source_name = module_path.to_string_lossy().into_owned();
        let source: &'static str = Box::leak(preprocessed.normalized.to_string().into_boxed_str());
        let tu = parse_translation_unit(
            source_name.clone(),
            source,
            Some(&preprocessed),
            StatelessResolver::new(),
        )
        .expect("failed to parse co2 module")
        .0;
        let tu = deduplicate_tu_items(tu);
        let children = load_modules(
            ctx,
            def_id,
            &child_module_dir(&module_path),
            &tu.rust_mod_items,
            rustc_file_ids,
            source_files,
            loaded_paths,
        );

        modules.push(LoadedModule {
            name: mod_item.name.0.clone(),
            def_id,
            decl_span: *mod_span,
            source_name,
            source,
            tu,
            children,
        });
    }

    modules
}

fn build_module_data_tree(
    ctx: &HirStructureCtx<'_>,
    module: &LoadedModule,
    foreign_mod: DefId,
) -> ModuleData {
    let mut data =
        ModuleData::forward_pass_parsed_module(ctx, &module.tu, module.def_id, foreign_mod, false);
    for child in &module.children {
        data.insert_alias(&child.name, build_module_data_tree(ctx, child, foreign_mod));
    }
    data
}

fn import_module_use_items(
    resolver: &mut Resolver,
    module_path: &[String],
    modules: &[LoadedModule],
) {
    for module in modules {
        let mut child_path = module_path.to_vec();
        child_path.push(module.name.clone());
        resolver.import_use_items(&child_path, &module.tu);
        import_module_use_items(resolver, &child_path, &module.children);
    }
}

fn resolve_in_module<'a>(
    ctx: &CrateSigCtx<'_>,
    module_path: &'a [String],
    name: &'a str,
) -> (DefId, co2_ast::TypeQueryResult) {
    ctx.resolve_in_current(
        module_path
            .iter()
            .map(String::as_str)
            .chain(std::iter::once(name)),
    )
    .unwrap()
}

fn lower_translation_unit_items(
    ctx: &mut CrateSigCtx<'_>,
    tu: &TranslationUnit<StatelessResolver>,
    modules: &[LoadedModule],
    module_path: &[String],
    source_name: &str,
    source: &'static str,
    foreign_mod: DefId,
    foreign_items: &mut Vec<ForeignModItem>,
) -> Vec<HirModuleItem> {
    let mut hir_items = Vec::new();
    for (item, parser_span) in tu.items.clone() {
        let span = ctx.co2_span_to_rustc(parser_span);
        let mut resolver =
            LocalResolver::new(ctx.resolver.clone()).with_module_path(module_path.to_vec());
        let item = item.transform(&resolver);
        match item {
            Declaration::FunctionDefinition { signature, body } => {
                let (name, sig, param_names, no_mangle) = match signature {
                    FunctionDefinitionSignature::C {
                        declaration_specifiers,
                        declarator,
                    } => {
                        let is_static =
                            declaration_specifiers.iter().any(|spec| spec.0.is_static());
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

                let id = resolve_in_module(ctx, module_path, &name).0;
                let function_name = name.clone();
                let param_tys = sig.inputs.clone();
                let id = FnDef(id);
                hir_items.push(HirModuleItem::Function {
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
                        source_name.to_owned(),
                        source,
                        body.0.tokens.1,
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
                        let type_def = resolve_in_module(ctx, module_path, &name).0;
                        ctx.resolver
                            .borrow_mut()
                            .typedef_tys
                            .insert(type_def, ty.clone());
                        hir_items.push(HirModuleItem::TypeDef {
                            name,
                            id: type_def,
                            span,
                            ty,
                        });
                        continue;
                    }

                    if let CTy::Ty(ty) = &ty {
                        let id = resolve_in_module(ctx, module_path, &name).0;
                        ctx.resolver
                            .borrow_mut()
                            .global_value_tys
                            .insert(id, ty.clone());
                    }

                    match ty {
                        CTy::Ty(ty) => {
                            let (id, _) = resolve_in_module(ctx, module_path, &name);
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
                                hir_items.push(HirModuleItem::Static {
                                    name,
                                    id,
                                    ty,
                                    mutable: true,
                                    span,
                                });
                            }
                        }
                        CTy::UnsizedArray(elem_ty) => {
                            let (id, _) = resolve_in_module(ctx, module_path, &name);
                            if let Some(initializer) = initializer {
                                ctx.mir_owners.insert(
                                    id,
                                    MirOwnerInfo::Static {
                                        initializer: initializer.clone(),
                                    },
                                );
                                let len =
                                    infer_unsized_array_len(&initializer.0, &resolver, &elem_ty)
                                        .unwrap_or_else(|err| {
                                            ctx.terminate_with_error(parser_span, &err)
                                        });
                                let ty = HirTy::new_array(elem_ty, HirTyConst::Literal(len), span);
                                ctx.resolver
                                    .borrow_mut()
                                    .global_value_tys
                                    .insert(id, ty.clone());
                                hir_items.push(HirModuleItem::Static {
                                    name,
                                    id,
                                    ty,
                                    mutable: true,
                                    span,
                                });
                            } else if is_extern {
                                let ty = HirTy::new_array(elem_ty, HirTyConst::Literal(0), span);
                                ctx.resolver
                                    .borrow_mut()
                                    .global_value_tys
                                    .insert(id, ty.clone());
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
                            let def_id = resolve_in_module(ctx, module_path, &name).0;
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

    for module in modules {
        let mut child_path = module_path.to_vec();
        child_path.push(module.name.clone());
        let items = lower_translation_unit_items(
            ctx,
            &module.tu,
            &module.children,
            &child_path,
            &module.source_name,
            module.source,
            foreign_mod,
            foreign_items,
        );
        let span = ctx.co2_span_to_rustc(module.decl_span);
        hir_items.push(HirModuleItem::Module {
            name: module.name.clone(),
            id: module.def_id,
            module: HirModule { span, items },
            span,
        });
    }
    hir_items
}

pub fn lower_crate_sig(
    ctx: HirStructureCtx<'_>,
    source_path: PathBuf,
    source_name: String,
    src_static: &'static str,
    file_id: FileId,
    preprocessed: Arc<PreprocessedSource>,
    file_ids: &mut HashMap<co2_ast::FileId, FileId>,
    source_files: &mut HashMap<co2_ast::FileId, (String, Arc<str>)>,
    no_main: bool,
) -> (HirStructure, HashMap<DefId, MirOwnerInfo>, WellknownDefs) {
    let span = ctx.span_in_file(file_id, 0, 0);
    let deps = ctx.dependencies();

    let tu = co2_parser::parse_translation_unit(
        source_name.clone(),
        src_static,
        Some(&preprocessed),
        StatelessResolver::new(),
    )
    .expect("failed to parse co2 source")
    .0;

    let tu = deduplicate_tu_items(tu);
    let mut loaded_paths = HashSet::new();
    let loaded_modules = load_modules(
        &ctx,
        ctx.root_crate_def_id(),
        &root_module_dir(&source_path),
        &tu.rust_mod_items,
        file_ids,
        source_files,
        &mut loaded_paths,
    );
    co2_ast::set_source_map(Arc::new(SourceMapSnapshot {
        files: Arc::new(source_files.clone()),
    }));

    let foreign_mod = ctx.allocate_def_id(ctx.root_crate_def_id(), DefData::ForeignMod);
    let mut foreign_items = Vec::new();

    let ctx = &*Box::leak(Box::new(ctx));
    let mut resolver = Resolver::new(&ctx, deps, &tu, foreign_mod);
    for module in &loaded_modules {
        resolver.insert_module_data(
            &[],
            &module.name,
            build_module_data_tree(ctx, module, foreign_mod),
        );
    }
    resolver.import_use_items(&[], &tu);
    import_module_use_items(&mut resolver, &[], &loaded_modules);
    resolver.rebuild_method_receivers();
    let file_ids = Arc::new(file_ids.clone());

    let mut ctx = CrateSigCtx {
        resolver: Rc::new(RefCell::new(LocalResolverBase {
            resolver,
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
            struct_manager: StructManager::default(),
            unrepresentable_typedefs: HashMap::new(),
            typedef_tys: HashMap::new(),
            global_value_tys: HashMap::new(),
            global_struct_tags: Rc::new(RefCell::new(StructAndEnumData::default())),
            global_locals: Rc::new(RefCell::new(im::HashMap::new())),
        })),
        hir_ctx: ctx,
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

    let root_items = lower_translation_unit_items(
        &mut ctx,
        &tu,
        &loaded_modules,
        &[],
        &source_name,
        src_static,
        foreign_mod,
        &mut foreign_items,
    );
    ctx.hir_items.extend(root_items);

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
            ctx.resolver
                .borrow_mut()
                .global_value_tys
                .insert(id, ty.clone());
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
        maybe_uninit_uninit: FnDef(
            ctx.resolve("core::mem::MaybeUninit::<T>::uninit")
                .unwrap()
                .0,
        ),
        valist: AdtDef(ctx.resolve("core::ffi::VaList").unwrap().0),
        valist_fn_arg: FnDef(ctx.resolve("core::ffi::VaList::<'f>::arg").unwrap().0),
        clone: FnDef(ctx.resolve("core::clone::Clone::clone").unwrap().0),
        zeroed: FnDef(ctx.resolve("core::mem::zeroed").unwrap().0),
        transmute: FnDef(ctx.resolve("core::intrinsics::transmute").unwrap().0),
        transmute_copy: FnDef(ctx.resolve("core::mem::transmute_copy").unwrap().0),
        str_as_ptr: FnDef(ctx.resolve("core::str::<impl str>::as_ptr").unwrap().0),
        offset_mut: FnDef(
            ctx.resolve("core::ptr::mut_ptr::<impl *mut T>::offset")
                .unwrap()
                .0,
        ),
        offset_const: FnDef(
            ctx.resolve("core::ptr::const_ptr::<impl *const T>::offset")
                .unwrap()
                .0,
        ),
        offset_from: FnDef(
            ctx.resolve("core::ptr::const_ptr::<impl *const T>::offset_from")
                .unwrap()
                .0,
        ),
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
        co2_ast::Initializer::Expr((
            co2_ast::Expression::Constant(co2_ast::Constant::String(s)),
            _,
        )) => Ok(s.chars().count() + 1),
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
                let element_advance = if consumed_slots == 0 {
                    1
                } else {
                    consumed_slots
                };
                let total_slots = used_slots_in_current + element_advance;
                let fully_covered = total_slots.div_ceil(slots_per_elem);
                max_len = max_len.max(index + fully_covered);
                next_index = index + total_slots / slots_per_elem;
                used_slots_in_current = total_slots % slots_per_elem;
            }
            Ok(max_len)
        }
        _ => {
            Err("static with unsized array type should have list or string initializer".to_owned())
        }
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

fn flattened_scalar_slots(ty: &HirTy, resolver: &LocalResolver) -> Result<usize, String> {
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
                    StructOrUnionKind::Struct => fields.iter().try_fold(0usize, |acc, field| {
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
