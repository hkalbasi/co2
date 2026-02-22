use std::collections::{BTreeMap, HashMap};

use co2_hir::{HirBody, LabelId, LocalId};
use rustc_public_generative as rustc_gen;
use rustc_public_generative::rustc_public::{
    mir::{
        Body, CastKind, ConstOperand, LocalDecl as MirLocalDecl, Mutability, Operand as MirOperand,
        Rvalue, Statement as MirStatement, StatementKind as MirStatementKind,
    },
    ty::{
        FnDef, GenericArgKind, GenericArgs, IntTy, MirConst, RigidTy, Span as RustSpan, Ty,
        TyKind, UintTy, VariantIdx,
    },
};

use crate::place::place;

pub fn build_mir_for_body(
    body: &HirBody,
    deps: &rustc_gen::DependencyInfo,
    ctx: &rustc_gen::HirStructureCtx,
    file_id: rustc_gen::FileId,
    is_rust_entry_main: bool,
) -> Body {
    let span = ctx.span_in_file(file_id, 0, 0);
    let exit_fn = if is_rust_entry_main {
        Some(dep_fn_any(
            deps,
            &["std::process::exit", "core::process::exit"],
        ))
    } else {
        None
    };

    let mut locals = Vec::with_capacity(body.locals.len());
    let mut local_indices = HashMap::new();
    for (idx, (local_id, local)) in body.locals.iter().enumerate() {
        let ty = if is_rust_entry_main && idx == 0 {
            Ty::new_tuple(&[])
        } else {
            local.ty
        };
        locals.push(MirLocalDecl {
            ty,
            span: local.span,
            mutability: Mutability::Mut,
        });
        local_indices.insert(local_id, idx);
    }

    let mut builder = Builder {
        deps,
        local_indices,
        locals,
        extra_locals: Vec::new(),
        blocks: Vec::new(),
        stmts: Vec::new(),
        label_blocks: HashMap::new(),
        pending_gotos: Vec::new(),
        span,
        is_rust_entry_main,
        exit_fn,
        exit_code_local: None,
    };

    if is_rust_entry_main {
        let i32_ty = Ty::signed_ty(IntTy::I32);
        let local = builder.new_temp(i32_ty, Mutability::Mut, span);
        let zero = MirConst::try_from_uint(0, UintTy::U32).expect("failed to build zero const");
        builder.stmts.push(MirStatement {
            kind: MirStatementKind::Assign(
                place(local),
                Rvalue::Cast(
                    CastKind::IntToInt,
                    MirOperand::Constant(ConstOperand {
                        span,
                        user_ty: None,
                        const_: zero,
                    }),
                    i32_ty,
                ),
            ),
            span,
        });
        builder.exit_code_local = Some(local);
    }

    for stmt in &body.stmts {
        builder.lower_stmt(stmt);
    }

    builder.terminate_fallthrough();
    builder.locals.extend(builder.extra_locals);

    Body::new(
        builder.blocks,
        builder.locals,
        body.params.len(),
        vec![],
        None,
        span,
    )
}

pub(crate) struct Builder<'a> {
    pub(crate) deps: &'a rustc_gen::DependencyInfo,
    pub(crate) local_indices: HashMap<LocalId, usize>,
    pub(crate) locals: Vec<MirLocalDecl>,
    pub(crate) extra_locals: Vec<MirLocalDecl>,
    pub(crate) blocks: Vec<rustc_gen::rustc_public::mir::BasicBlock>,
    pub(crate) stmts: Vec<rustc_gen::rustc_public::mir::Statement>,
    pub(crate) label_blocks: HashMap<LabelId, usize>,
    pub(crate) pending_gotos: Vec<(usize, LabelId)>,
    pub(crate) span: RustSpan,
    pub(crate) is_rust_entry_main: bool,
    pub(crate) exit_fn: Option<FnDef>,
    pub(crate) exit_code_local: Option<usize>,
}

pub(crate) fn variant_idx(id: usize) -> VariantIdx {
    unsafe { std::mem::transmute::<usize, VariantIdx>(id) }
}

pub(crate) fn fn_const_operand(
    fn_def: FnDef,
    generic_args: Vec<GenericArgKind>,
    span: RustSpan,
) -> rustc_gen::rustc_public::mir::Operand {
    let fn_ty = Ty::from_rigid_kind(RigidTy::FnDef(fn_def, GenericArgs(generic_args)));
    let c = MirConst::try_new_zero_sized(fn_ty).expect("failed to build fn const");
    rustc_gen::rustc_public::mir::Operand::Constant(ConstOperand {
        span,
        user_ty: None,
        const_: c,
    })
}

pub(crate) fn infer_fn_generic_args(
    sig: &rustc_public_generative::rustc_public::ty::FnSig,
    args: &[co2_hir::HirExpr],
    ret_ty: Ty,
) -> Vec<GenericArgKind> {
    let mut by_index: BTreeMap<u32, Ty> = BTreeMap::new();
    for (expected, actual) in sig.inputs().iter().zip(args.iter()) {
        collect_param_bindings(*expected, actual.ty, &mut by_index);
    }
    collect_param_bindings(sig.output(), ret_ty, &mut by_index);
    by_index
        .into_values()
        .map(GenericArgKind::Type)
        .collect::<Vec<_>>()
}

fn collect_param_bindings(expected: Ty, actual: Ty, out: &mut BTreeMap<u32, Ty>) {
    match (expected.kind(), actual.kind()) {
        (TyKind::Param(param), _) => {
            out.entry(param.index).or_insert(actual);
        }
        (TyKind::RigidTy(RigidTy::Ref(_, expected_inner, _)), _) => {
            collect_param_bindings(expected_inner, actual, out);
        }
        (
            TyKind::RigidTy(RigidTy::Adt(expected_adt, expected_args)),
            TyKind::RigidTy(RigidTy::Adt(actual_adt, actual_args)),
        ) if expected_adt == actual_adt && expected_args.0.len() == actual_args.0.len() => {
            for (e, a) in expected_args.0.iter().zip(actual_args.0.iter()) {
                if let (GenericArgKind::Type(et), GenericArgKind::Type(at)) = (e, a) {
                    collect_param_bindings(*et, *at, out);
                }
            }
        }
        _ => {}
    }
}

pub(crate) fn ty_matches_expected(expected: Ty, actual: Ty) -> bool {
    match (expected.kind(), actual.kind()) {
        (TyKind::Param(_), _) => true,
        (TyKind::RigidTy(RigidTy::Ref(_, expected_inner, _)), _) => {
            ty_matches_expected(expected_inner, actual)
        }
        (
            TyKind::RigidTy(RigidTy::Adt(expected_adt, expected_args)),
            TyKind::RigidTy(RigidTy::Adt(actual_adt, actual_args)),
        ) => {
            expected_adt == actual_adt
                && expected_args.0.len() == actual_args.0.len()
                && expected_args
                    .0
                    .iter()
                    .zip(actual_args.0.iter())
                    .all(|(e, a)| match (e, a) {
                        (GenericArgKind::Type(et), GenericArgKind::Type(at)) => {
                            ty_matches_expected(*et, *at)
                        }
                        _ => e == a,
                    })
        }
        _ => expected == actual,
    }
}

pub(crate) fn dep_fn_any(deps: &rustc_gen::DependencyInfo, paths: &[&str]) -> FnDef {
    for path in paths {
        if let Some(found) = find_dep_fn(deps, path) {
            return found;
        }
    }
    panic!("missing dependency function (any of): {}", paths.join(", "));
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
                normalized.ends_with(&format!("::{normalized_path}"))
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
        if let Some(found) = deps
            .functions
            .iter()
            .find(|f| {
                let normalized = normalize_dep_path(&f.path);
                normalized.ends_with(&format!("::{last}"))
                    && f.fn_def.is_some()
                    && !f.path.contains("::{closure")
                    && !f.path.contains("{{")
                    && required_segments.iter().all(|seg| normalized.contains(seg))
            })
            .and_then(|f| f.fn_def)
        {
            return Some(found);
        }
        if let Some(found) = deps
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
