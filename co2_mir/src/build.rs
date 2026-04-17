use std::collections::{BTreeMap, HashMap};

use co2_hir::{HirBody, LabelId, LocalId, WellknownDefs};
use rustc_public_generative as rustc_gen;
use rustc_public_generative::rustc_public::{
    CrateDefType,
    mir::{Body, ConstOperand, LocalDecl as MirLocalDecl, Mutability},
    ty::{
        FnDef, GenericArgKind, GenericArgs, MirConst, RigidTy, Span as RustSpan, Ty, TyKind,
        VariantIdx,
    },
};

pub fn build_mir_for_body(
    body: &HirBody,
    ctx: &rustc_gen::HirStructureCtx,
    file_id: rustc_gen::FileId,
    wellknown_defs: WellknownDefs,
) -> Body {
    let span = ctx.span_in_file(file_id, 0, 0);

    let mut locals = Vec::with_capacity(body.locals.len());
    let mut local_indices = HashMap::new();
    for (idx, (local_id, local)) in body.locals.iter().enumerate() {
        let ty = local.ty;
        locals.push(MirLocalDecl {
            ty,
            span: local.span,
            mutability: Mutability::Mut,
        });
        local_indices.insert(local_id, idx);
    }

    let mut builder = Builder {
        c_variadic_local: body.c_variadic_local.map(|l| local_indices[&l]),
        local_indices,
        locals,
        extra_locals: Vec::new(),
        blocks: Vec::new(),
        stmts: Vec::new(),
        label_blocks: HashMap::new(),
        pending_gotos: Vec::new(),
        span,
        wellknown_defs,
    };

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

pub(crate) struct Builder {
    pub(crate) wellknown_defs: WellknownDefs,
    pub(crate) local_indices: HashMap<LocalId, usize>,
    pub(crate) locals: Vec<MirLocalDecl>,
    pub(crate) extra_locals: Vec<MirLocalDecl>,
    pub(crate) blocks: Vec<rustc_gen::rustc_public::mir::BasicBlock>,
    pub(crate) stmts: Vec<rustc_gen::rustc_public::mir::Statement>,
    pub(crate) label_blocks: HashMap<LabelId, usize>,
    pub(crate) pending_gotos: Vec<(usize, LabelId)>,
    pub(crate) span: RustSpan,
    pub(crate) c_variadic_local: Option<usize>,
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
    fn_def: FnDef,
    sig: &rustc_public_generative::rustc_public::ty::FnSig,
    args: &[co2_hir::HirExpr],
    ret_ty: Ty,
) -> Vec<GenericArgKind> {
    let mut by_index: BTreeMap<u32, Ty> = BTreeMap::new();
    for (expected, actual) in sig.inputs().iter().zip(args.iter()) {
        collect_param_bindings(*expected, actual.ty, &mut by_index);
    }
    collect_param_bindings(sig.output(), ret_ty, &mut by_index);
    match fn_def.ty().kind() {
        TyKind::RigidTy(RigidTy::FnDef(_, existing)) if !existing.0.is_empty() => existing
            .0
            .iter()
            .map(|arg| match arg {
                GenericArgKind::Type(ty) => match ty.kind() {
                    TyKind::Param(param) => by_index
                        .get(&param.index)
                        .copied()
                        .map(GenericArgKind::Type)
                        .unwrap_or_else(|| arg.clone()),
                    _ => arg.clone(),
                },
                _ => arg.clone(),
            })
            .collect::<Vec<_>>(),
        _ => by_index
            .into_values()
            .map(GenericArgKind::Type)
            .collect::<Vec<_>>(),
    }
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
            TyKind::RigidTy(RigidTy::RawPtr(expected_inner, _)),
            TyKind::RigidTy(RigidTy::RawPtr(actual_inner, _)),
        ) => {
            collect_param_bindings(expected_inner, actual_inner, out);
        }
        (
            TyKind::RigidTy(RigidTy::Adt(expected_adt, expected_args)),
            TyKind::RigidTy(RigidTy::Adt(actual_adt, actual_args)),
        ) if expected_adt == actual_adt && actual_args.0.len() <= expected_args.0.len() => {
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
        (
            TyKind::RigidTy(RigidTy::Adt(expected_adt, expected_args)),
            TyKind::RigidTy(RigidTy::Adt(actual_adt, actual_args)),
        ) => {
            expected_adt == actual_adt
                && expected_args
                    .0
                    .iter()
                    .zip(actual_args.0.iter())
                    .all(|(e, a)| match (e, a) {
                        (GenericArgKind::Type(et), GenericArgKind::Type(at)) => {
                            ty_matches_expected(*et, *at)
                        }
                        (GenericArgKind::Lifetime(_), GenericArgKind::Lifetime(_)) => true,
                        _ => e == a,
                    })
                && extra_adt_args_are_concrete(
                    &expected_args.0[actual_args.0.len().min(expected_args.0.len())..],
                )
                && extra_adt_args_are_concrete(
                    &actual_args.0[expected_args.0.len().min(actual_args.0.len())..],
                )
        }
        _ => expected == actual,
    }
}

fn extra_adt_args_are_concrete(args: &[GenericArgKind]) -> bool {
    args.iter().all(|arg| !matches!(arg, GenericArgKind::Type(ty) if matches!(ty.kind(), TyKind::Param(_))))
}
