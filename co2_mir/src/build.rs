use std::collections::{BTreeMap, HashMap, HashSet};

use co2_hir::{HirBody, LabelId, LocalId, LocalResolver, WellknownDefs};
use rustc_public_generative as rustc_gen;
use rustc_public_generative::rustc_public::{
    CrateDefType, DefId,
    mir::{
        Body, ConstOperand, LocalDecl as MirLocalDecl, Mutability, Place, SourceInfo, VarDebugInfo,
        VarDebugInfoContents,
    },
    ty::{
        FnDef, GenericArgKind, GenericArgs, MirConst, RigidTy, Span as RustSpan, Ty, TyKind,
        VariantIdx,
    },
};

pub struct MirBodyResult {
    pub body: Body,
}

pub fn build_mir_for_body(
    body: &HirBody,
    ctx: &rustc_gen::HirStructureCtx,
    owner: DefId,
    file_id: rustc_gen::FileId,
    wellknown_defs: WellknownDefs,
    decl_resolver: Option<LocalResolver>,
) -> MirBodyResult {
    let span = ctx.span_in_file(file_id, 0, 0);

    let mut locals = Vec::with_capacity(body.locals.len());
    let mut local_indices = HashMap::new();
    for (idx, (local_id, local)) in body.locals.iter().enumerate() {
        let ty = ctx.normalize_ty_defaults(local.ty);
        locals.push(MirLocalDecl {
            ty,
            span: local.span,
            mutability: Mutability::Mut,
        });
        local_indices.insert(local_id, idx);
    }

    let param_indices: HashSet<usize> = body
        .params
        .iter()
        .map(|param_id| local_indices[param_id])
        .collect();

    let mut var_debug_info = Vec::new();
    let mut local_vdi_map = HashMap::new();
    for (idx, (_, local)) in body.locals.iter().enumerate() {
        if local.name == "_ret" || local.name.starts_with("__co2_") {
            continue;
        }
        let argument_index = if param_indices.contains(&idx) {
            body.params
                .iter()
                .position(|p| local_indices[p] == idx)
                .map(|p| p as u16 + 1)
        } else {
            None
        };
        local_vdi_map.insert(idx, var_debug_info.len());
        var_debug_info.push(VarDebugInfo {
            name: local.name.clone(),
            source_info: SourceInfo {
                span: local.span,
                scope: 0,
            },
            composite: None,
            value: VarDebugInfoContents::Place(Place::from(idx)),
            argument_index,
        });
    }

    let mut builder = Builder {
        ctx,
        owner,
        c_variadic_local: body.c_variadic_local.map(|l| local_indices[&l]),
        local_indices,
        locals,
        extra_locals: Vec::new(),
        blocks: Vec::new(),
        stmts: Vec::new(),
        label_blocks: HashMap::new(),
        label_discriminants: body
            .labels
            .iter()
            .filter(|(_, label)| label.name.is_some())
            .enumerate()
            .map(|(idx, (label_id, _))| (label_id, idx as u128 + 1))
            .collect(),
        pending_gotos: Vec::new(),
        pending_indirect_gotos: Vec::new(),
        span,
        wellknown_defs,
        decl_resolver,
        var_debug_info,
        local_vdi_map,
        scope_stack: vec![0],
        next_scope: 1,
    };

    let generation = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        for stmt in &body.stmts {
            builder.lower_stmt(stmt);
        }

        builder.terminate_fallthrough();
        builder.locals.extend(builder.extra_locals);

        MirBodyResult {
            body: Body::new(
                builder.blocks,
                builder.locals,
                body.params.len(),
                builder.var_debug_info,
                None,
                span,
            ),
        }
    }));

    match generation {
        Ok(result) => result,
        Err(payload) => {
            if co2_ast::is_diagnostic_abort(payload.as_ref()) {
                // Mirrored from HIR's `terminate_with_error`: the error was
                // already reported, so substitute a valid dummy body instead
                // of propagating the panic out of the parallel query (which
                // would surface as a rustc ICE).
                dummy_mir_body(body, ctx, file_id, span)
            } else {
                std::panic::resume_unwind(payload)
            }
        }
    }
}

/// A valid, minimal body used when MIR generation terminated with an error.
/// The emitted diagnostic causes the driver to abort the compilation, so this
/// body only needs to pass MIR validation.
fn dummy_mir_body(
    body: &HirBody,
    ctx: &rustc_gen::HirStructureCtx,
    _file_id: rustc_gen::FileId,
    span: RustSpan,
) -> MirBodyResult {
    let locals: Vec<MirLocalDecl> = body
        .locals
        .iter()
        .map(|(_, local)| MirLocalDecl {
            ty: ctx.normalize_ty_defaults(local.ty),
            span: local.span,
            mutability: Mutability::Mut,
        })
        .collect();
    let mut blocks = Vec::new();
    blocks.push(rustc_gen::rustc_public::mir::BasicBlock {
        statements: Vec::new(),
        terminator: rustc_gen::rustc_public::mir::Terminator {
            kind: rustc_gen::rustc_public::mir::TerminatorKind::Return,
            source_info: SourceInfo { span, scope: 0 },
        },
    });
    MirBodyResult {
        body: Body::new(blocks, locals, body.params.len(), Vec::new(), None, span),
    }
}

pub(crate) struct Builder<'ctx, 'tcx> {
    pub(crate) ctx: &'ctx rustc_gen::HirStructureCtx<'tcx>,
    pub(crate) owner: DefId,
    pub(crate) decl_resolver: Option<co2_hir::LocalResolver>,
    pub(crate) wellknown_defs: WellknownDefs,
    pub(crate) local_indices: HashMap<LocalId, usize>,
    pub(crate) locals: Vec<MirLocalDecl>,
    pub(crate) extra_locals: Vec<MirLocalDecl>,
    pub(crate) blocks: Vec<rustc_gen::rustc_public::mir::BasicBlock>,
    pub(crate) stmts: Vec<rustc_gen::rustc_public::mir::Statement>,
    pub(crate) label_blocks: HashMap<LabelId, usize>,
    pub(crate) label_discriminants: HashMap<LabelId, u128>,
    pub(crate) pending_gotos: Vec<(usize, LabelId)>,
    pub(crate) pending_indirect_gotos: Vec<usize>,
    pub(crate) span: RustSpan,
    pub(crate) c_variadic_local: Option<usize>,
    pub(crate) var_debug_info: Vec<VarDebugInfo>,
    pub(crate) local_vdi_map: HashMap<usize, usize>,
    pub(crate) scope_stack: Vec<u32>,
    pub(crate) next_scope: u32,
}

impl Builder<'_, '_> {
    /// Report a compilation error and terminate MIR generation for this body,
    /// mirroring `HirCtx::terminate_with_error`. `build_mir_for_body` catches
    /// the resulting `DiagnosticAbort` and substitutes a dummy body.
    pub(crate) fn terminate_with_error(&self, rust_span: RustSpan, msg: &str) -> ! {
        let co2_span = self
            .decl_resolver
            .as_ref()
            .map(|resolver| resolver.rust_span_to_co2_span(rust_span))
            .unwrap_or_else(|| co2_ast::Span::from_parts(co2_ast::FileId::INVALID, 0..0));
        co2_ast::emit_errors_and_terminate(vec![co2_ast::Rich::custom(co2_span, msg.to_owned())]);
    }

    pub(crate) fn current_scope(&self) -> u32 {
        *self.scope_stack.last().unwrap_or(&0)
    }

    pub(crate) fn enter_scope(&mut self) -> u32 {
        let scope = self.next_scope;
        self.next_scope += 1;
        self.scope_stack.push(scope);
        scope
    }

    pub(crate) fn exit_scope(&mut self) {
        self.scope_stack.pop();
    }
}

impl Builder<'_, '_> {
    pub(crate) fn format_ty(&self, ty: Ty) -> String {
        co2_hir::format_ty(self.decl_resolver.as_ref(), ty)
    }
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
                        .map_or_else(|| arg.clone(), GenericArgKind::Type),
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

pub(crate) fn complete_fn_generic_args(
    fn_def: FnDef,
    sig: &rustc_public_generative::rustc_public::ty::FnSig,
    args: &[co2_hir::HirExpr],
    ret_ty: Ty,
    provided: &[GenericArgKind],
) -> Vec<GenericArgKind> {
    let inferred = infer_fn_generic_args(fn_def, sig, args, ret_ty);
    if provided.is_empty() {
        return inferred;
    }

    if inferred.len() >= provided.len() {
        let mut completed = inferred;
        completed[..provided.len()].clone_from_slice(provided);
        completed
    } else {
        provided.to_vec()
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
    args.iter().all(
        |arg| !matches!(arg, GenericArgKind::Type(ty) if matches!(ty.kind(), TyKind::Param(_))),
    )
}
