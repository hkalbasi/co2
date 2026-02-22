#![feature(rustc_private)]

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use co2_hir::{HirCtx, ResolvedValue};
use co2_parser::{
    BinOp as ParsedBinOp, Declaration, DeclarationSpecifier, Declarator, EnumSpecifier, Expression,
    InitDeclarator, Initializer, StorageClassSpecifier, StructDeclarator, StructOrUnionField,
    StructOrUnionSpecifier, TypeQueryResult, TypeResolver, TypeSpecifier,
    UnaryOp as ParsedUnaryOp,
};
use rustc_public_generative::rustc_public::{
    CrateItem,
    DefId,
    mir::{
        BasicBlock, Body, CastKind, ConstOperand, LocalDecl, Mutability, Operand, Rvalue,
        Statement, StatementKind, Terminator, TerminatorKind,
    },
    ty::{AdtDef, FnDef, GenericArgKind, GenericArgs, MirConst, RigidTy, Ty, UintTy},
};
use rustc_public_generative::{self as rustc_gen, FunctionSignature};

mod hir_ty;
mod span;
mod types;

pub use types::CompileMode;

use crate::hir_ty::{lower_function_signature, lower_value_decl_type};
use crate::span::{FILE_ID};

struct PendingCompile {
    mode: CompileMode,
    source_path: PathBuf,
    source: String,
}

fn pending_compile_cell() -> &'static Mutex<Option<PendingCompile>> {
    static CELL: OnceLock<Mutex<Option<PendingCompile>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(None))
}

struct PendingFunction {
    name: String,
    def: FnDef,
    sig: FunctionSignature,
    param_names: Vec<String>,
    body_tokens: Vec<co2_parser::Spanned<co2_parser::Token>>,
}

struct PendingStatic {
    name: String,
    def: DefId,
    ty: rustc_gen::HirTy,
    init_value: i64,
}

struct Co2GeneratorState {
    deps: rustc_gen::DependencyInfo,
    file_id: rustc_gen::FileId,
    mode: CompileMode,
    pending_functions: Vec<PendingFunction>,
    pending_statics: Vec<PendingStatic>,
    typedefs: HashMap<String, DefId>,
    typedef_type_defs: HashMap<String, DefId>,
    local_value_map: HashMap<String, ResolvedValue>,
    uses: Vec<String>,
    global_prelude_decls: Vec<Declaration>,
    source_name: String,
    src_static: &'static str,
}

unsafe impl Send for Co2GeneratorState {}
unsafe impl Sync for Co2GeneratorState {}

struct DriverResolver<'a> {
    typedefs: &'a HashMap<String, DefId>,
    typedef_type_defs: &'a HashMap<String, DefId>,
    values: &'a HashMap<String, ResolvedValue>,
    deps: &'a rustc_gen::DependencyInfo,
    uses: &'a [String],
}

impl DriverResolver<'_> {
    fn resolve_value(&self, path: &str) -> Option<ResolvedValue> {
        for candidate in resolve_candidates(path, self.uses) {
            if let Some(v) = self.values.get(&candidate) {
                return Some(v.clone());
            }
            if let Some(last) = candidate.rsplit("::").next()
                && let Some(v) = self.values.get(last)
            {
                return Some(v.clone());
            }
            if let Some(fn_def) = find_dep_fn(self.deps, &candidate) {
                return Some(ResolvedValue::Fn(fn_def));
            }
        }
        None
    }

    fn resolve_type(&self, path: &str) -> Option<Ty> {
        for candidate in resolve_candidates(path, self.uses) {
            if let Some(type_def) = self.typedef_type_defs.get(&candidate) {
                return Some(CrateItem(*type_def).ty());
            }
            if let Some(last) = candidate.rsplit("::").next()
                && let Some(type_def) = self.typedef_type_defs.get(last)
            {
                return Some(CrateItem(*type_def).ty());
            }
            if let Some(ty) = self.typedefs.get(&candidate) {
                return Some(CrateItem(*ty).ty());
            }
            if let Some(last) = candidate.rsplit("::").next()
                && let Some(ty) = self.typedefs.get(last)
            {
                return Some(CrateItem(*ty).ty());
            }
            if let Some(ty) = resolve_ty_candidate(&candidate, self.typedefs, self.deps, self.uses)
            {
                return Some(ty);
            }
        }
        None
    }
}

impl rustc_gen::CrateGeneratorState for Co2GeneratorState {
    fn hir_structure(ctx: rustc_gen::HirStructureCtx) -> (Self, rustc_gen::HirStructure) {
        let pending = pending_compile_cell()
            .lock()
            .unwrap()
            .take()
            .expect("missing pending compile input");

        let file_id = ctx.add_custom_file(&pending.source_path, pending.source.clone());
        FILE_ID.set(file_id).unwrap();
        let span = ctx.span_in_file(file_id, 0, 0);
        let deps = ctx.dependencies();
        let source_name = pending.source_path.to_string_lossy().into_owned();
        let src_static: &'static str = Box::leak(pending.source.into_boxed_str());
        struct TranslationUnitParseResolver;
        impl TypeResolver for TranslationUnitParseResolver {
            fn classify_path(&self, path: &co2_parser::RustPath) -> TypeQueryResult {
                let _ = path;
                TypeQueryResult::Unsure
            }
        }

        let parse_resolver = TranslationUnitParseResolver;
        let tu = co2_parser::parse_translation_unit(source_name.clone(), src_static, &parse_resolver)
            .expect("failed to parse co2 source")
            .0;
        let items = tu.items;
        let mut global_prelude_decls = items
            .iter()
            .filter_map(|(item, _)| match item {
                Declaration::Declaration {
                    declaration_specifiers,
                    declarators,
                } => {
                    let is_typedef = declaration_specifiers.iter().any(|(spec, _)| {
                        matches!(
                            spec,
                            DeclarationSpecifier::StorageSpecifier((StorageClassSpecifier::Typedef, _))
                        )
                    });
                    let has_initializer = declarators
                        .iter()
                        .any(|d| d.0.initializer.is_some());
                    if !is_typedef && has_initializer {
                        Some(item.clone())
                    } else {
                        None
                    }
                }
                Declaration::FunctionDefinition { .. } => None,
            })
            .collect::<Vec<_>>();

        let uses = tu
            .rust_use_items
            .into_iter()
            .map(|u| {
                u.0.path
                    .into_iter()
                    .map(|(part, _)| part)
                    .collect::<Vec<_>>()
                    .join("::")
            })
            .collect::<Vec<_>>();

        let root_crate = ctx.root_crate_def_id();

        let mut typedefs: HashMap<String, DefId> = HashMap::new();
        let mut typedef_hir_tys: HashMap<String, rustc_gen::HirTy> = HashMap::new();

        let mut pending_functions = Vec::new();
        let mut pending_statics = Vec::new();
        let mut externs: HashMap<String, FunctionSignature> = HashMap::new();
        let mut hir_items = Vec::new();
        let mut enum_values: HashMap<String, i64> = HashMap::new();
        let mut typedef_type_defs: HashMap<String, DefId> = HashMap::new();

        struct PendingStructDef {
            key: String,
            fields: Vec<co2_parser::Spanned<StructOrUnionField>>,
        }

        let mut pending_structs_by_key: HashMap<String, PendingStructDef> = HashMap::new();
        let mut typedef_struct_aliases: Vec<(String, String)> = Vec::new();
        let mut struct_tag_aliases: Vec<(String, String)> = Vec::new();
        for (item, _) in &items {
            let Declaration::Declaration {
                declaration_specifiers,
                declarators,
            } = item
            else {
                continue;
            };

            let is_typedef = declaration_specifiers.iter().any(|(spec, _)| {
                matches!(
                    spec,
                    DeclarationSpecifier::StorageSpecifier((StorageClassSpecifier::Typedef, _))
                )
            });
            let struct_spec = declaration_specifiers
                .iter()
                .find_map(|(spec, _)| match spec {
                    DeclarationSpecifier::TypeSpecifier((
                        TypeSpecifier::StructOrUnion { specifier, .. },
                        _,
                    )) => Some(specifier.clone()),
                    _ => None,
                });

            let Some(struct_spec) = struct_spec else {
                continue;
            };

            let key = match struct_spec.canonical_field_set_key() {
                Some(key) => key,
                None => continue,
            };

            let fields = match struct_spec {
                StructOrUnionSpecifier::Defined { ident, fields } => {
                    struct_tag_aliases.push((ident.0.clone(), key.clone()));
                    fields
                }
                StructOrUnionSpecifier::Anonymous { fields } => {
                    fields
                }
                StructOrUnionSpecifier::Declared { .. } => continue,
            };

            pending_structs_by_key
                .entry(key.clone())
                .or_insert_with(|| PendingStructDef {
                    key: key.clone(),
                    fields: fields.clone(),
                });

            if is_typedef {
                for init in declarators {
                    if let Some(alias) = declarator_name(&init.0.declarator.0) {
                        typedef_struct_aliases.push((alias, key.clone()));
                    }
                }
            }
        }

        let mut struct_keys = pending_structs_by_key.keys().cloned().collect::<Vec<_>>();
        struct_keys.sort();

        let mut adt_by_name: HashMap<String, AdtDef> = HashMap::new();
        let mut adt_public_name_by_key: HashMap<String, String> = HashMap::new();
        for (idx, struct_key) in struct_keys.iter().enumerate() {
            let synthetic_name = format!("__anon_struct_{idx}");
            let adt = AdtDef(ctx.allocate_def_id(
                root_crate,
                rustc_gen::DefData::TypeNs(synthetic_name.clone()),
            ));
            adt_by_name.insert(struct_key.clone(), adt);
            adt_public_name_by_key.insert(struct_key.clone(), synthetic_name);
            typedefs.insert(struct_key.clone(), adt.0);
        }

        for (alias, key) in typedef_struct_aliases {
            if let Some(adt) = adt_by_name.get(&key) {
                typedefs.insert(alias, adt.0);
            }
        }
        for (tag_name, key) in struct_tag_aliases {
            if let Some(adt) = adt_by_name.get(&key) {
                typedefs.insert(tag_name, adt.0);
            }
        }

        for struct_key in struct_keys {
            let pending_struct = pending_structs_by_key
                .remove(&struct_key)
                .expect("missing pending struct for key");
            let adt = adt_by_name[&pending_struct.key];
            let mut hir_fields = Vec::new();
            let mut skip_struct = false;

            for (field, field_span) in pending_struct.fields {
                for (decl, _) in field.declarators {
                    let field_name = struct_declarator_name(&decl).unwrap_or_else(|| {
                        panic!("anonymous struct field in {}", pending_struct.key)
                    });
                    let specs = field
                        .specifiers
                        .iter()
                        .cloned()
                        .map(DeclarationSpecifier::TypeSpecifier)
                        .map(|spec| (spec, field_span))
                        .collect::<Vec<_>>();
                    let Ok((_, field_ty)) = lower_value_decl_type(
                        &ctx,
                        specs,
                        decl.declarator.clone(),
                        &typedefs,
                        &typedef_hir_tys,
                    ) else {
                        skip_struct = true;
                        break;
                    };

                    let field_def =
                        ctx.allocate_def_id(adt.0, rustc_gen::DefData::ValueNs(field_name.clone()));
                    hir_fields.push(rustc_gen::StructField {
                        id: field_def,
                        name: field_name,
                        ty: field_ty,
                    });
                }
                if skip_struct {
                    break;
                }
            }
            hir_items.push(rustc_gen::HirModuleItem::Adt {
                name: adt_public_name_by_key[&pending_struct.key].clone(),
                id: adt,
                kind: rustc_gen::HirAdtKind::Struct {
                    // Keep a placeholder ADT even when we cannot lower all fields.
                    // This preserves DefId consistency for header references.
                    fields: if skip_struct { Vec::new() } else { hir_fields },
                },
                span,
            });
        }

        for (item, _) in items {
            match item {
                Declaration::FunctionDefinition {
                    declaration_specifiers,
                    declarator,
                    body,
                } => {
                    if has_static_storage(&declaration_specifiers) {
                        continue;
                    }
                    let (name, sig, param_names) = lower_function_signature(
                        &ctx,
                        declaration_specifiers,
                        declarator,
                        &typedefs,
                        &typedef_hir_tys,
                    )
                    .expect("failed to lower function signature");
                    pending_functions.push(PendingFunction {
                        name,
                        sig,
                        param_names,
                        def: FnDef(ctx.root_crate_def_id()),
                        body_tokens: body.0.tokens.0,
                    });
                }
                Declaration::Declaration {
                    declaration_specifiers,
                    declarators,
                } => {
                    collect_enum_constants(&declaration_specifiers, &mut enum_values)
                        .expect("failed to evaluate enum constants");
                    let mut is_typedef = false;
                    let mut cleaned_specs = Vec::new();
                    for (spec, sp) in declaration_specifiers {
                        match spec {
                            DeclarationSpecifier::StorageSpecifier((
                                StorageClassSpecifier::Typedef,
                                _,
                            )) => {
                                is_typedef = true;
                            }
                            _ => cleaned_specs.push((spec, sp)),
                        }
                    }

                    for init in declarators {
                        let InitDeclarator {
                            declarator,
                            initializer,
                        } = init.0;
                        if let Some((initializer, _)) = &initializer
                            && let Initializer::Expr((expr, _)) = initializer
                            && !declarator_is_function(&declarator.0)
                            && let Ok((name, hir_ty)) = lower_value_decl_type(
                                &ctx,
                                cleaned_specs.clone(),
                                declarator.clone(),
                                &typedefs,
                                &typedef_hir_tys,
                            )
                            && let Ok(init_value) = eval_enum_const_expr(expr, &enum_values)
                        {
                            let static_def_id = ctx.allocate_def_id(
                                root_crate,
                                rustc_gen::DefData::ValueNs(name.clone()),
                            );
                            pending_statics.push(PendingStatic {
                                name,
                                def: static_def_id,
                                ty: hir_ty,
                                init_value,
                            });
                            continue;
                        }

                        if is_typedef {
                            if let Ok((name, ty)) = lower_value_decl_type(
                                &ctx,
                                cleaned_specs.clone(),
                                declarator.clone(),
                                &typedefs,
                                &typedef_hir_tys,
                            ) {
                                typedef_hir_tys.insert(name.clone(), ty.clone());
                                if let rustc_gen::HirTyKind::Adt(adt, _) = &ty.kind {
                                    typedefs.insert(name.clone(), adt.0);
                                    let type_def = ctx.allocate_def_id(
                                        root_crate,
                                        rustc_gen::DefData::TypeNs(name.clone()),
                                    );
                                    typedef_type_defs.insert(name.clone(), type_def);
                                    hir_items.push(rustc_gen::HirModuleItem::TypeDef {
                                        name,
                                        id: type_def,
                                        span,
                                        ty,
                                    });
                                } else if !matches!(ty.kind, rustc_gen::HirTyKind::FnPtr(_)) {
                                    let type_def =
                                        ctx.allocate_def_id(root_crate, rustc_gen::DefData::TypeNs(name.clone()));
                                    typedef_type_defs.insert(name.clone(), type_def);
                                    hir_items.push(rustc_gen::HirModuleItem::TypeDef {
                                        name,
                                        id: type_def,
                                        span,
                                        ty,
                                    });
                                }
                            }
                            continue;
                        }

                        if let Ok((name, sig, _param_names)) = lower_function_signature(
                            &ctx,
                            cleaned_specs.clone(),
                            declarator.clone(),
                            &typedefs,
                            &typedef_hir_tys,
                        ) {
                            externs.insert(name, sig);
                        }
                    }
                }
            }
        }

        let mut local_value_map = HashMap::new();
        let mut fn_defs = Vec::new();

        for (name, value) in &enum_values {
            local_value_map.insert(name.clone(), ResolvedValue::ConstInt(*value));
        }

        for func in &mut pending_functions {
            let is_c_entry_main = pending.mode.no_main && func.name == "main";
            let fn_def = FnDef(
                ctx.allocate_def_id(root_crate, rustc_gen::DefData::ValueNs(func.name.clone())),
            );
            func.def = fn_def;

            local_value_map.insert(func.name.clone(), ResolvedValue::Fn(fn_def));
            fn_defs.push(fn_def);

            let mut sig = func.sig.clone();

            if is_c_entry_main {
                sig.output = rustc_gen::HirTy::new_tuple(vec![], span);
            }

            hir_items.push(rustc_gen::HirModuleItem::Function {
                name: func.name.clone(),
                id: fn_def,
                sig,
                no_mangle: if is_c_entry_main {
                    false
                } else {
                    pending.mode.function_no_mangle
                },
                span,
            });
        }

        for pending_static in &pending_statics {
            hir_items.push(rustc_gen::HirModuleItem::Static {
                name: pending_static.name.clone(),
                id: pending_static.def,
                ty: pending_static.ty.clone(),
                mutable: true,
                span,
            });
            local_value_map.insert(
                pending_static.name.clone(),
                ResolvedValue::ConstInt(pending_static.init_value),
            );
        }
        let static_names = pending_statics
            .iter()
            .map(|s| s.name.clone())
            .collect::<HashSet<_>>();
        global_prelude_decls.retain(|decl| !decl_all_declarators_in_set(decl, &static_names));

        let referenced_body_idents = pending_functions
            .iter()
            .flat_map(|func| func.body_tokens.iter())
            .filter_map(|(tok, _)| match tok {
                co2_parser::Token::Ident(name) => Some(name.clone()),
                _ => None,
            })
            .collect::<HashSet<_>>();

        let foreign_mod = ctx.allocate_def_id(root_crate, rustc_gen::DefData::ForeignMod);
        let mut foreign_items = Vec::new();

        for (name, sig) in externs {
            if local_value_map.contains_key(&name) {
                continue;
            }
            if !referenced_body_idents.contains(&name) {
                continue;
            }

            let fn_def =
                FnDef(ctx.allocate_def_id(foreign_mod, rustc_gen::DefData::ValueNs(name.clone())));

            local_value_map.insert(name.clone(), ResolvedValue::Fn(fn_def));

            foreign_items.push(rustc_gen::ForeignModItem::ForeignFunction {
                name,
                id: fn_def,
                sig,
                span,
            });
        }

        hir_items.push(rustc_gen::HirModuleItem::ForeignMod {
            id: foreign_mod,
            items: foreign_items,
        });

        (
            Co2GeneratorState {
                deps,
                file_id,
                mode: pending.mode,
                pending_functions,
                pending_statics,
                typedefs,
                typedef_type_defs,
                local_value_map,
                uses,
                global_prelude_decls,
                source_name,
                src_static,
            },
            rustc_gen::HirStructure {
                root: rustc_gen::HirModule {
                    span,
                    items: hir_items,
                },
            },
        )
    }

    fn emit_mir(&mut self, ctx: rustc_gen::HirStructureCtx, def: DefId) -> Body {
        if let Some(pending_static) = self.pending_statics.iter().find(|s| s.def == def) {
            return build_static_initializer_body(
                CrateItem(pending_static.def).ty(),
                pending_static.init_value,
                ctx.span_in_file(self.file_id, 0, 0),
            );
        }
        let func = self
            .pending_functions
            .iter()
            .find(|f| f.def.0 == def)
            .unwrap_or_else(|| panic!("missing function/static for def {def:?}"));

        let resolver = DriverResolver {
            typedefs: &self.typedefs,
            typedef_type_defs: &self.typedef_type_defs,
            values: &self.local_value_map,
            deps: &self.deps,
            uses: &self.uses,
        };
        let span_converter = |span: co2_parser::Span| {
            ctx.span_in_file(self.file_id, span.start as u32, span.end as u32)
        };
        let hir_ctx = HirCtx::new(
            &resolver,
            DriverResolver::resolve_value,
            DriverResolver::resolve_type,
            &span_converter,
        );
        let body_identifiers = func
            .body_tokens
            .iter()
            .filter_map(|(tok, _)| match tok {
                co2_parser::Token::Ident(name) => Some(name.as_str()),
                _ => None,
            })
            .collect::<HashSet<_>>();
        let filtered_prelude_decls = self
            .global_prelude_decls
            .iter()
            .filter(|decl| {
                prelude_decl_names(decl)
                    .into_iter()
                    .any(|name| body_identifiers.contains(name.as_str()))
            })
            .cloned()
            .collect::<Vec<_>>();

        let hir = co2_hir::lower_function_body(
            &func.body_tokens,
            &self.source_name,
            &self.src_static,
            func.def,
            &func.param_names,
            &filtered_prelude_decls,
            &hir_ctx,
        )
        .unwrap();

        co2_mir::build_mir_for_body(
            &hir,
            &self.deps,
            &ctx,
            self.file_id,
            self.mode.no_main && func.name == "main",
        )
    }
}

fn resolve_candidates(path: &str, uses: &[String]) -> Vec<String> {
    let mut out = Vec::new();

    let mut push = |candidate: String| {
        if !out.iter().any(|c| c == &candidate) {
            out.push(candidate);
        }
    };

    push(path.to_owned());

    if !path.contains("::") {
        for use_path in uses {
            if use_path.rsplit("::").next() == Some(path) {
                push(use_path.clone());
            }
        }
    }

    let first_segment = path.split("::").next().unwrap_or(path);
    for use_path in uses {
        let Some(last) = use_path.rsplit("::").next() else {
            continue;
        };
        if last != first_segment {
            continue;
        }

        let prefix = if let Some(idx) = use_path.rfind("::") {
            &use_path[..idx]
        } else {
            continue;
        };

        if path == first_segment {
            push(use_path.clone());
            continue;
        }

        let suffix = &path[first_segment.len() + 2..];
        push(format!("{prefix}::{suffix}"));
    }

    out
}

fn resolve_ty_candidate(
    path: &str,
    typedefs: &HashMap<String, DefId>,
    deps: &rustc_gen::DependencyInfo,
    uses: &[String],
) -> Option<Ty> {
    let normalized = path.replace("::<", "<");
    let path = normalized.as_str();

    if let Some(prim) = co2_hir::primitive_type(path) {
        return Some(prim);
    }

    if let Some(ty) = typedefs.get(path) {
        return Some(Ty::from_rigid_kind(RigidTy::Adt(
            AdtDef(*ty),
            GenericArgs(vec![]),
        )));
    }

    if let Some(last) = path.rsplit("::").next()
        && let Some(ty) = typedefs.get(last)
    {
        return Some(Ty::from_rigid_kind(RigidTy::Adt(
            AdtDef(*ty),
            GenericArgs(vec![]),
        )));
    }

    let (base, generic_args_src) = split_type_path(path);
    let adt = find_dep_adt(deps, base)?;

    let mut generic_args = Vec::new();
    for arg in generic_args_src {
        let mut resolved = None;
        for candidate in resolve_candidates(arg, uses) {
            if let Some(ty) = resolve_ty_candidate(&candidate, typedefs, deps, uses) {
                resolved = Some(ty);
                break;
            }
        }
        let ty = resolved?;
        generic_args.push(GenericArgKind::Type(ty));
    }

    if (base == "std::vec::Vec" || base == "alloc::vec::Vec" || base.ends_with("::Vec"))
        && generic_args.len() == 1
        && let Some(global) =
            find_dep_adt_any(deps, &["alloc::alloc::Global", "std::alloc::Global"])
    {
        generic_args.push(GenericArgKind::Type(Ty::from_rigid_kind(RigidTy::Adt(
            global,
            GenericArgs(vec![]),
        ))));
    }

    Some(Ty::from_rigid_kind(RigidTy::Adt(
        adt,
        GenericArgs(generic_args),
    )))
}

fn collect_enum_constants(
    declaration_specifiers: &[co2_parser::Spanned<DeclarationSpecifier>],
    enum_values: &mut HashMap<String, i64>,
) -> Result<(), String> {
    for (spec, _) in declaration_specifiers {
        let DeclarationSpecifier::TypeSpecifier((type_specifier, _)) = spec else {
            continue;
        };
        let TypeSpecifier::Enum(enum_spec) = type_specifier else {
            continue;
        };
        let enumerators = match enum_spec {
            EnumSpecifier::Defined { enumerators, .. } | EnumSpecifier::Anonymous { enumerators } => {
                enumerators
            }
            EnumSpecifier::Declared { .. } => continue,
        };
        let mut next = 0i64;
        for (enumerator, _) in enumerators {
            let value = if let Some((expr, _)) = &enumerator.value {
                eval_enum_const_expr(expr, enum_values)?
            } else {
                next
            };
            enum_values.insert(enumerator.ident.0.clone(), value);
            next = value.saturating_add(1);
        }
    }
    Ok(())
}

fn eval_enum_const_expr(expr: &Expression, enum_values: &HashMap<String, i64>) -> Result<i64, String> {
    match expr {
        Expression::Constant(co2_parser::Constant::Int(v, _)) => Ok(*v),
        Expression::Constant(co2_parser::Constant::Float(v)) => Ok(v.trunc() as i64),
        Expression::Constant(co2_parser::Constant::Char(ch)) => Ok(*ch as i64),
        Expression::Identifier(path) => {
            let pretty = path.0.to_pretty();
            if let Some(v) = enum_values.get(&pretty) {
                return Ok(*v);
            }
            if let Some(last) = pretty.rsplit("::").next()
                && let Some(v) = enum_values.get(last)
            {
                return Ok(*v);
            }
            Err(format!("unknown enum constant in enumerator value: {pretty}"))
        }
        Expression::UnaryOp(op, inner) => {
            let v = eval_enum_const_expr(&inner.0, enum_values)?;
            match op {
                ParsedUnaryOp::Plus => Ok(v),
                ParsedUnaryOp::Minus => Ok(-v),
                ParsedUnaryOp::Com => Ok(!v),
                ParsedUnaryOp::Not => Ok((v == 0) as i64),
                ParsedUnaryOp::AddrOf | ParsedUnaryOp::Deref => {
                    Err("invalid unary operator in enum constant expression".to_owned())
                }
            }
        }
        Expression::Cast { expr, .. } => eval_enum_const_expr(&expr.0, enum_values),
        Expression::Conditional {
            cond,
            then_expr,
            else_expr,
        } => {
            if eval_enum_const_expr(&cond.0, enum_values)? != 0 {
                eval_enum_const_expr(&then_expr.0, enum_values)
            } else {
                eval_enum_const_expr(&else_expr.0, enum_values)
            }
        }
        Expression::BinOp(lhs, op, rhs) => {
            let l = eval_enum_const_expr(&lhs.0, enum_values)?;
            let r = eval_enum_const_expr(&rhs.0, enum_values)?;
            match op {
                ParsedBinOp::Assign => Err("assignment not allowed in enum constant expression".to_owned()),
                ParsedBinOp::Add => Ok(l.wrapping_add(r)),
                ParsedBinOp::Sub => Ok(l.wrapping_sub(r)),
                ParsedBinOp::Mul => Ok(l.wrapping_mul(r)),
                ParsedBinOp::Div => Ok(l / r),
                ParsedBinOp::Rem => Ok(l % r),
                ParsedBinOp::BitOr => Ok(l | r),
                ParsedBinOp::BitXor => Ok(l ^ r),
                ParsedBinOp::BitAnd => Ok(l & r),
                ParsedBinOp::Eq => Ok((l == r) as i64),
                ParsedBinOp::Lt => Ok((l < r) as i64),
                ParsedBinOp::Le => Ok((l <= r) as i64),
                ParsedBinOp::Ne => Ok((l != r) as i64),
                ParsedBinOp::Ge => Ok((l >= r) as i64),
                ParsedBinOp::Gt => Ok((l > r) as i64),
                ParsedBinOp::Shl => Ok(l << r),
                ParsedBinOp::Shr => Ok(l >> r),
                ParsedBinOp::And => Ok(((l != 0) && (r != 0)) as i64),
                ParsedBinOp::Or => Ok(((l != 0) || (r != 0)) as i64),
            }
        }
        _ => Err("unsupported enum constant expression".to_owned()),
    }
}

fn split_type_path(path: &str) -> (&str, Vec<&str>) {
    let Some(start) = path.find('<') else {
        return (path, Vec::new());
    };

    let mut depth = 0usize;
    let mut end = None;
    for (idx, ch) in path.char_indices() {
        match ch {
            '<' => depth += 1,
            '>' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(idx);
                    break;
                }
            }
            _ => {}
        }
    }

    let Some(end) = end else {
        return (path, Vec::new());
    };

    let args_src = &path[start + 1..end];
    let mut args = Vec::new();
    let mut seg_start = 0usize;
    let mut seg_depth = 0usize;

    for (idx, ch) in args_src.char_indices() {
        match ch {
            '<' => seg_depth += 1,
            '>' => seg_depth -= 1,
            ',' if seg_depth == 0 => {
                args.push(args_src[seg_start..idx].trim());
                seg_start = idx + 1;
            }
            _ => {}
        }
    }

    let tail = args_src[seg_start..].trim();
    if !tail.is_empty() {
        args.push(tail);
    }

    (path[..start].trim(), args)
}
fn find_dep_adt(deps: &rustc_gen::DependencyInfo, path: &str) -> Option<AdtDef> {
    if let Some(found) = deps.types.iter().find(|t| t.path == path).map(|t| t.adt) {
        return Some(found);
    }

    if let Some(found) = deps
        .types
        .iter()
        .find(|t| t.path.ends_with(path))
        .map(|t| t.adt)
    {
        return Some(found);
    }

    if let Some(last) = path.rsplit("::").next()
        && let Some(found) = deps
            .types
            .iter()
            .find(|t| t.path.ends_with(&format!("::{last}")))
            .map(|t| t.adt)
    {
        return Some(found);
    }

    None
}

fn find_dep_adt_any(deps: &rustc_gen::DependencyInfo, paths: &[&str]) -> Option<AdtDef> {
    for path in paths {
        if let Some(found) = deps.types.iter().find(|t| t.path == *path).map(|t| t.adt) {
            return Some(found);
        }
        if let Some(found) = deps
            .types
            .iter()
            .find(|t| t.path.ends_with(path))
            .map(|t| t.adt)
        {
            return Some(found);
        }
    }
    None
}

fn find_dep_fn(deps: &rustc_gen::DependencyInfo, path: &str) -> Option<FnDef> {
    let normalized_path = normalize_dep_path(path);

    if let Some(found) = deps
        .functions
        .iter()
        .find(|f| normalize_dep_path(&f.path) == normalized_path && f.fn_def.is_some())
        .and_then(|f| f.fn_def)
    {
        return Some(found);
    }

    if let Some(found) = deps
        .functions
        .iter()
        .find(|f| {
            let normalized = normalize_dep_path(&f.path);
            (if path.contains("::") {
                normalized.ends_with(&normalized_path)
            } else {
                normalized.ends_with(&format!("::{}", normalized_path))
            }) && f.fn_def.is_some()
                && !f.path.contains("::{closure")
                && !f.path.contains("{{")
        })
        .and_then(|f| f.fn_def)
    {
        return Some(found);
    }

    if let Some(last) = path.rsplit("::").next() {
        let required_segments = normalized_path
            .split("::")
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();
        let required_tail = required_segments
            .iter()
            .rev()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>();
        if let Some(found) = deps
            .functions
            .iter()
            .find(|f| {
                let normalized = normalize_dep_path(&f.path);
                normalized.ends_with(&format!("::{last}"))
                    && f.fn_def.is_some()
                    && !f.path.contains("::{closure")
                    && !f.path.contains("{{")
                    && required_tail.iter().all(|seg| normalized.contains(seg))
            })
            .and_then(|f| f.fn_def)
        {
            return Some(found);
        }
        if !path.contains("::")
            && let Some(found) = deps
                .functions
                .iter()
                .find(|f| {
                    f.path.ends_with(&format!("::{last}"))
                        && f.fn_def.is_some()
                        && !f.path.contains("::{closure")
                        && !f.path.contains("{{")
                })
                .and_then(|f| f.fn_def)
        {
            return Some(found);
        }
    }

    None
}

fn normalize_dep_path(path: &str) -> String {
    let mut no_generics = String::with_capacity(path.len());
    let mut depth = 0usize;
    for ch in path.chars() {
        match ch {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            _ if depth == 0 => no_generics.push(ch),
            _ => {}
        }
    }
    no_generics
        .split("::")
        .filter(|seg| !seg.is_empty() && !seg.starts_with('{') && !seg.ends_with('}'))
        .collect::<Vec<_>>()
        .join("::")
}

fn declarator_name(decl: &Declarator) -> Option<String> {
    match decl {
        Declarator::Identifier((name, _)) => Some(name.clone()),
        Declarator::FunctionDeclarator { declarator, .. }
        | Declarator::PointerDeclarator { declarator, .. }
        | Declarator::ArrayDeclarator { declarator, .. } => declarator_name(&declarator.0),
        Declarator::Abstract => None,
    }
}

fn struct_declarator_name(decl: &StructDeclarator) -> Option<String> {
    declarator_name(&decl.declarator.0)
}

fn prelude_decl_names(decl: &Declaration) -> Vec<String> {
    match decl {
        Declaration::Declaration { declarators, .. } => declarators
            .iter()
            .filter_map(|d| declarator_name(&d.0.declarator.0))
            .collect(),
        Declaration::FunctionDefinition { .. } => Vec::new(),
    }
}

fn has_static_storage(specifiers: &[co2_parser::Spanned<DeclarationSpecifier>]) -> bool {
    specifiers.iter().any(|(spec, _)| {
        matches!(
            spec,
            DeclarationSpecifier::StorageSpecifier((StorageClassSpecifier::Static, _))
        )
    })
}

fn declarator_is_function(decl: &Declarator) -> bool {
    match decl {
        Declarator::FunctionDeclarator { .. } => true,
        Declarator::Identifier(_) | Declarator::Abstract => false,
        Declarator::PointerDeclarator { declarator, .. }
        | Declarator::ArrayDeclarator { declarator, .. } => declarator_is_function(&declarator.0),
    }
}

fn decl_all_declarators_in_set(decl: &Declaration, names: &HashSet<String>) -> bool {
    let Declaration::Declaration { declarators, .. } = decl else {
        return false;
    };
    if declarators.is_empty() {
        return false;
    }
    declarators.iter().all(|d| {
        declarator_name(&d.0.declarator.0)
            .as_ref()
            .is_some_and(|n| names.contains(n))
    })
}

fn build_static_initializer_body(ty: Ty, init_value: i64, span: rustc_public_generative::rustc_public::ty::Span) -> Body {
    let locals = vec![
        LocalDecl {
            ty,
            span,
            mutability: Mutability::Mut,
        },
        LocalDecl {
            ty: Ty::unsigned_ty(UintTy::U64),
            span,
            mutability: Mutability::Mut,
        },
    ];
    let const_u64 = MirConst::try_from_uint(init_value as u128, UintTy::U64)
        .expect("failed to build static initializer const");
    let mut statements = vec![Statement {
        kind: StatementKind::Assign(
            rustc_public_generative::rustc_public::mir::Place {
                local: 1,
                projection: vec![],
            },
            Rvalue::Use(Operand::Constant(ConstOperand {
                span,
                user_ty: None,
                const_: const_u64,
            })),
        ),
        span,
    }];
    statements.push(Statement {
        kind: StatementKind::Assign(
            rustc_public_generative::rustc_public::mir::Place {
                local: 0,
                projection: vec![],
            },
            Rvalue::Cast(
                CastKind::IntToInt,
                Operand::Copy(rustc_public_generative::rustc_public::mir::Place {
                    local: 1,
                    projection: vec![],
                }),
                ty,
            ),
        ),
        span,
    });

    Body::new(
        vec![BasicBlock {
            statements,
            terminator: Terminator {
                kind: TerminatorKind::Return,
                span,
            },
        }],
        locals,
        0,
        vec![],
        None,
        span,
    )
}

pub fn compile_co2_file(mode: CompileMode, co2_file: &Path) {
    let src = std::fs::read_to_string(co2_file).expect("failed to read co2 file");
    compile_co2_source(
        mode,
        co2_file.to_path_buf(),
        src,
        std::env::args().collect(),
    );
}

pub fn compile_co2_source(
    mode: CompileMode,
    source_path: PathBuf,
    source: String,
    rustc_args: Vec<String>,
) {
    *pending_compile_cell().lock().unwrap() = Some(PendingCompile {
        mode,
        source_path,
        source,
    });

    rustc_gen::generate_with_args::<Co2GeneratorState>(rustc_args);
}
