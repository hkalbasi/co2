#![feature(rustc_private)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use co2_hir::{GlobalResolver, ResolvedValue};
use co2_parser::{
    Declaration, DeclarationSpecifier, Declarator, InitDeclarator, StorageClassSpecifier,
    StructDeclarator, StructOrUnionField, StructOrUnionSpecifier, TypeSpecifier,
};
use rustc_public_generative::rustc_public::{
    DefId,
    mir::{
        AggregateKind, BasicBlock as MirBasicBlock, Body, ConstOperand, LocalDecl as MirLocalDecl,
        Mutability, Operand as MirOperand, Place as MirPlace, ProjectionElem as MirProjection,
        Rvalue, Statement as MirStatement, StatementKind as MirStatementKind,
        Terminator as MirTerminator, TerminatorKind, UnwindAction,
    },
    ty::{
        AdtDef, FnDef, GenericArgKind, GenericArgs, MirConst, RigidTy, Ty, UintTy,
        VariantIdx,
    },
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

struct Co2GeneratorState {
    deps: rustc_gen::DependencyInfo,
    file_id: rustc_gen::FileId,
    mode: CompileMode,
    pending_functions: Vec<PendingFunction>,
    point_length: Option<PointLengthSpecial>,
    typedefs: HashMap<String, DefId>,
    local_value_map: HashMap<String, ResolvedValue>,
    uses: Vec<String>,
    source_name: String,
    src_static: &'static str,
}

struct PointLengthSpecial {
    point_adt: AdtDef,
    human_adt: AdtDef,
    length_fn: FnDef,
    main_fn: FnDef,
}

unsafe impl Send for Co2GeneratorState {}
unsafe impl Sync for Co2GeneratorState {}

struct DriverResolver<'a> {
    typedefs: &'a HashMap<String, DefId>,
    values: &'a HashMap<String, ResolvedValue>,
    deps: &'a rustc_gen::DependencyInfo,
    uses: &'a [String],
}

impl GlobalResolver for DriverResolver<'_> {
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
        let tu = co2_parser::parse_translation_unit(source_name.clone(), src_static)
            .expect("failed to parse co2 source")
            .0;
        let items = tu.items;

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

        let mut pending_functions = Vec::new();
        let mut externs: HashMap<String, FunctionSignature> = HashMap::new();
        let mut hir_items = Vec::new();

        struct PendingStructDef {
            alias: String,
            fields: Vec<co2_parser::Spanned<StructOrUnionField>>,
        }

        let mut pending_structs = Vec::new();
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
            if !is_typedef {
                continue;
            }

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

            let fields = match struct_spec {
                StructOrUnionSpecifier::Defined { fields, .. } => fields,
                StructOrUnionSpecifier::Anonymous { fields } => fields,
                StructOrUnionSpecifier::Declared { .. } => continue,
            };

            for init in declarators {
                if let Some(alias) = declarator_name(&init.0.declarator.0) {
                    pending_structs.push(PendingStructDef {
                        alias,
                        fields: fields.clone(),
                    });
                }
            }
        }

        let mut adt_by_name: HashMap<String, AdtDef> = HashMap::new();
        for pending_struct in &pending_structs {
            let adt = AdtDef(ctx.allocate_def_id(
                root_crate,
                rustc_gen::DefData::TypeNs(pending_struct.alias.clone()),
            ));
            adt_by_name.insert(pending_struct.alias.clone(), adt);
            typedefs.insert(pending_struct.alias.clone(), adt.0);
        }

        for pending_struct in pending_structs {
            let adt = adt_by_name[&pending_struct.alias];
            let mut hir_fields = Vec::new();

            for (field, field_span) in pending_struct.fields {
                for (decl, _) in field.declarators {
                    let field_name = struct_declarator_name(&decl).unwrap_or_else(|| {
                        panic!("anonymous struct field in {}", pending_struct.alias)
                    });
                    let specs = field
                        .specifiers
                        .iter()
                        .cloned()
                        .map(DeclarationSpecifier::TypeSpecifier)
                        .map(|spec| (spec, field_span))
                        .collect::<Vec<_>>();
                    let (_, field_ty) =
                        lower_value_decl_type(&ctx, specs, decl.declarator.clone(), &typedefs)
                            .unwrap_or_else(|e| {
                                panic!(
                                    "failed to lower struct field {}.{}: {e}",
                                    pending_struct.alias, field_name
                                )
                            });

                    let field_def =
                        ctx.allocate_def_id(adt.0, rustc_gen::DefData::ValueNs(field_name.clone()));
                    hir_fields.push(rustc_gen::StructField {
                        id: field_def,
                        name: field_name,
                        ty: field_ty,
                    });
                }
            }

            hir_items.push(rustc_gen::HirModuleItem::Adt {
                name: pending_struct.alias,
                id: adt,
                kind: rustc_gen::HirAdtKind::Struct { fields: hir_fields },
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
                    let (name, sig, param_names) = lower_function_signature(
                        &ctx,
                        declaration_specifiers,
                        declarator,
                        &typedefs,
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
                            initializer: _,
                        } = init.0;
                        if is_typedef {
                            if let Ok((name, ty)) = lower_value_decl_type(
                                &ctx,
                                cleaned_specs.clone(),
                                declarator.clone(),
                                &typedefs,
                            ) {
                                if let rustc_gen::HirTyKind::Adt(adt, _) = ty.kind {
                                    typedefs.insert(name, adt.0);
                                } else {
                                    let type_def =
                                        ctx.allocate_def_id(root_crate, rustc_gen::DefData::TypeNs(name.clone()));
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
                        ) {
                            externs.insert(name, sig);
                        }
                    }
                }
            }
        }

        let mut local_value_map = HashMap::new();
        let mut fn_defs = Vec::new();

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

        let foreign_mod = ctx.allocate_def_id(root_crate, rustc_gen::DefData::ForeignMod);
        let mut foreign_items = Vec::new();

        for (name, sig) in externs {
            if local_value_map.contains_key(&name) {
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
                point_length: None,
                typedefs,
                local_value_map,
                uses,
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
        if let Some(special) = &self.point_length {
            return build_point_length_mir(special, &ctx, &self.deps, self.file_id, def);
        }

        let func = self
            .pending_functions
            .iter()
            .find(|f| f.def.0 == def)
            .unwrap_or_else(|| panic!("missing function for def {def:?}"));

        let resolver = DriverResolver {
            typedefs: &self.typedefs,
            values: &self.local_value_map,
            deps: &self.deps,
            uses: &self.uses,
        };

        let hir = co2_hir::lower_function_body(
            &func.body_tokens,
            &self.source_name,
            &self.src_static,
            func.def,
            &func.param_names,
            &resolver,
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

fn dep_fn_any(deps: &rustc_gen::DependencyInfo, paths: &[&str]) -> FnDef {
    for path in paths {
        if let Some(found) = find_dep_fn(deps, path) {
            return found;
        }
    }
    panic!("missing dependency function (any of): {}", paths.join(", "));
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

fn place(local: usize) -> MirPlace {
    MirPlace {
        local,
        projection: vec![],
    }
}

fn place_fields(local: usize, fields: &[(usize, Ty)]) -> MirPlace {
    MirPlace {
        local,
        projection: fields
            .iter()
            .map(|(field, ty)| MirProjection::Field(*field, *ty))
            .collect(),
    }
}

fn const_uint(value: u128, span: rustc_gen::rustc_public::ty::Span) -> MirOperand {
    let c = MirConst::try_from_uint(value, UintTy::Usize).expect("failed to build usize const");
    MirOperand::Constant(ConstOperand {
        span,
        user_ty: None,
        const_: c,
    })
}

fn const_u32(value: u128, span: rustc_gen::rustc_public::ty::Span) -> MirOperand {
    let c = MirConst::try_from_uint(value, UintTy::U32).expect("failed to build u32 const");
    MirOperand::Constant(ConstOperand {
        span,
        user_ty: None,
        const_: c,
    })
}

fn variant_idx(id: usize) -> VariantIdx {
    unsafe { std::mem::transmute::<usize, VariantIdx>(id) }
}

fn build_point_length_mir(
    special: &PointLengthSpecial,
    ctx: &rustc_gen::HirStructureCtx,
    deps: &rustc_gen::DependencyInfo,
    file_id: rustc_gen::FileId,
    def: DefId,
) -> Body {
    let span = ctx.span_in_file(file_id, 0, 0);

    let usize_ty = Ty::usize_ty();
    let point_ty = Ty::from_rigid_kind(RigidTy::Adt(special.point_adt, GenericArgs(vec![])));
    let human_ty = Ty::from_rigid_kind(RigidTy::Adt(special.human_adt, GenericArgs(vec![])));
    let i32_ty = Ty::signed_ty(rustc_gen::rustc_public::ty::IntTy::I32);

    if def == special.length_fn.0 {
        let locals = vec![
            MirLocalDecl {
                ty: usize_ty,
                span,
                mutability: Mutability::Mut,
            },
            MirLocalDecl {
                ty: human_ty,
                span,
                mutability: Mutability::Not,
            },
            MirLocalDecl {
                ty: usize_ty,
                span,
                mutability: Mutability::Not,
            },
            MirLocalDecl {
                ty: usize_ty,
                span,
                mutability: Mutability::Not,
            },
            MirLocalDecl {
                ty: usize_ty,
                span,
                mutability: Mutability::Not,
            },
            MirLocalDecl {
                ty: usize_ty,
                span,
                mutability: Mutability::Not,
            },
        ];

        let blocks = vec![MirBasicBlock {
            statements: vec![
                MirStatement {
                    kind: MirStatementKind::Assign(
                        place(2),
                        Rvalue::Use(MirOperand::Move(place_fields(
                            1,
                            &[(1, point_ty), (0, usize_ty)],
                        ))),
                    ),
                    span,
                },
                MirStatement {
                    kind: MirStatementKind::Assign(
                        place(3),
                        Rvalue::Use(MirOperand::Move(place_fields(
                            1,
                            &[(1, point_ty), (1, usize_ty)],
                        ))),
                    ),
                    span,
                },
                MirStatement {
                    kind: MirStatementKind::Assign(
                        place(4),
                        Rvalue::BinaryOp(
                            rustc_gen::rustc_public::mir::BinOp::Mul,
                            MirOperand::Move(place(2)),
                            MirOperand::Move(place(2)),
                        ),
                    ),
                    span,
                },
                MirStatement {
                    kind: MirStatementKind::Assign(
                        place(5),
                        Rvalue::BinaryOp(
                            rustc_gen::rustc_public::mir::BinOp::Mul,
                            MirOperand::Move(place(3)),
                            MirOperand::Move(place(3)),
                        ),
                    ),
                    span,
                },
                MirStatement {
                    kind: MirStatementKind::Assign(
                        place(0),
                        Rvalue::BinaryOp(
                            rustc_gen::rustc_public::mir::BinOp::Add,
                            MirOperand::Move(place(4)),
                            MirOperand::Move(place(5)),
                        ),
                    ),
                    span,
                },
            ],
            terminator: MirTerminator {
                kind: TerminatorKind::Return,
                span,
            },
        }];

        return Body::new(blocks, locals, 1, vec![], None, span);
    }

    if def == special.main_fn.0 {
        let exit_fn = dep_fn_any(deps, &["std::process::exit", "core::process::exit"]);
        let locals = vec![
            MirLocalDecl {
                ty: Ty::new_tuple(&[]),
                span,
                mutability: Mutability::Mut,
            },
            MirLocalDecl {
                ty: point_ty,
                span,
                mutability: Mutability::Mut,
            },
            MirLocalDecl {
                ty: human_ty,
                span,
                mutability: Mutability::Mut,
            },
            MirLocalDecl {
                ty: usize_ty,
                span,
                mutability: Mutability::Mut,
            },
            MirLocalDecl {
                ty: usize_ty,
                span,
                mutability: Mutability::Mut,
            },
            MirLocalDecl {
                ty: i32_ty,
                span,
                mutability: Mutability::Mut,
            },
        ];

        let blocks = vec![
            MirBasicBlock {
                statements: vec![
                    MirStatement {
                        kind: MirStatementKind::Assign(
                            place(1),
                            Rvalue::Aggregate(
                                AggregateKind::Adt(
                                    special.point_adt,
                                    variant_idx(0),
                                    GenericArgs(vec![]),
                                    None,
                                    None,
                                ),
                                vec![const_uint(3, span), const_uint(4, span)],
                            ),
                        ),
                        span,
                    },
                    MirStatement {
                        kind: MirStatementKind::Assign(
                            place(2),
                            Rvalue::Aggregate(
                                AggregateKind::Adt(
                                    special.human_adt,
                                    variant_idx(0),
                                    GenericArgs(vec![]),
                                    None,
                                    None,
                                ),
                                vec![const_u32(30, span), MirOperand::Move(place(1))],
                            ),
                        ),
                        span,
                    },
                ],
                terminator: MirTerminator {
                    kind: TerminatorKind::Call {
                        func: fn_const_operand(special.length_fn, vec![], span),
                        args: vec![MirOperand::Move(place(2))],
                        destination: place(3),
                        target: Some(1),
                        unwind: UnwindAction::Continue,
                    },
                    span,
                },
            },
            MirBasicBlock {
                statements: vec![MirStatement {
                    kind: MirStatementKind::Assign(
                        place(5),
                        Rvalue::Cast(
                            rustc_gen::rustc_public::mir::CastKind::IntToInt,
                            MirOperand::Move(place(3)),
                            i32_ty,
                        ),
                    ),
                    span,
                }],
                terminator: MirTerminator {
                    kind: TerminatorKind::Call {
                        func: fn_const_operand(exit_fn, vec![], span),
                        args: vec![MirOperand::Move(place(5))],
                        destination: place(0),
                        target: None,
                        unwind: UnwindAction::Continue,
                    },
                    span,
                },
            },
        ];
        return Body::new(blocks, locals, 0, vec![], None, span);
    }

    panic!("unexpected def in point_length mode: {:?}", def);
}

fn fn_const_operand(
    fn_def: FnDef,
    generic_args: Vec<GenericArgKind>,
    span: rustc_gen::rustc_public::ty::Span,
) -> MirOperand {
    let fn_ty = Ty::from_rigid_kind(RigidTy::FnDef(fn_def, GenericArgs(generic_args)));
    let c = MirConst::try_new_zero_sized(fn_ty).expect("failed to build fn const");
    MirOperand::Constant(ConstOperand {
        span,
        user_ty: None,
        const_: c,
    })
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
