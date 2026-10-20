use rustc_data_structures::fx::{FxHashMap, FxHashSet};
use std::collections::BTreeMap;

use co2_hir::{HirBody, HirStmt, LabelId, LocalId, LocalResolver, WellknownDefs};
use rustc_public_generative as rustc_gen;
use rustc_public_generative::rustc_public::{
    CrateDefType, DefId,
    mir::{
        Body, ConstOperand, LocalDecl as MirLocalDecl, Mutability, Place, SourceInfo,
        StatementKind as MirStatementKind, VarDebugInfo, VarDebugInfoContents,
    },
    ty::{
        FnDef, GenericArgKind, GenericArgs, MirConst, RigidTy, Span as RustSpan, Ty, TyKind,
        VariantIdx,
    },
};

pub struct MirBodyResult {
    pub body: Body,
    pub is_error_recovery: bool,
}

pub fn build_mir_for_body(
    body: &HirBody,
    ctx: &rustc_gen::HirStructureCtx,
    owner: DefId,
    file_id: rustc_gen::FileId,
    wellknown_defs: WellknownDefs,
    decl_resolver: Option<LocalResolver>,
) -> MirBodyResult {
    let span = body.span;

    let mut locals = Vec::with_capacity(body.locals.len());
    let mut local_indices = FxHashMap::default();
    for (idx, (local_id, local)) in body.locals.iter().enumerate() {
        let ty = ctx.normalize_ty_defaults(local.ty);
        locals.push(MirLocalDecl {
            ty,
            span: local.span,
            mutability: Mutability::Mut,
        });
        local_indices.insert(local_id, idx);
    }

    let param_indices: FxHashSet<usize> = body
        .params
        .iter()
        .map(|param_id| local_indices[param_id])
        .collect();

    let mut var_debug_info = Vec::new();
    let mut local_vdi_map = FxHashMap::default();
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
        label_targets: FxHashMap::default(),
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
        last_stmt_span: None,
        wellknown_defs,
        decl_resolver,
        var_debug_info,
        local_vdi_map,
        scopes: vec![Scope::default()],
        next_scope: 1,
        naked_templates: Vec::new(),
    };

    let generation = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        builder.lower_stmts(&body.stmts);

        if body.naked {
            builder.terminate_naked();
        } else {
            builder.terminate_fallthrough();
        }
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
            is_error_recovery: false,
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
        is_error_recovery: true,
    }
}

/// One lexical scope while lowering, and also the snapshot kept for a label or
/// a jump.
#[derive(Clone, Default)]
pub(crate) struct Scope {
    /// Identity used to match scopes between two control-flow points.
    pub(crate) id: u32,
    /// Locals declared directly in the block: their storage lives from block
    /// entry to block exit (C11 6.2.4p5), so `goto` edges move these between
    /// `StorageLive` and `StorageDead`.
    pub(crate) locals: Vec<usize>,
    /// VLA locals declared in the block so far. A VLA identifier's scope starts
    /// at its declaration, not at block entry (C11 6.8.6.1p1).
    pub(crate) vlas: Vec<usize>,
}

/// The active scopes at a point control flow can reach, outermost first.
pub(crate) type ScopeState = Vec<Scope>;

pub(crate) struct Builder<'ctx, 'tcx> {
    pub(crate) ctx: &'ctx rustc_gen::HirStructureCtx<'tcx>,
    pub(crate) owner: DefId,
    pub(crate) decl_resolver: Option<co2_hir::LocalResolver>,
    pub(crate) wellknown_defs: WellknownDefs,
    pub(crate) local_indices: FxHashMap<LocalId, usize>,
    pub(crate) locals: Vec<MirLocalDecl>,
    pub(crate) extra_locals: Vec<MirLocalDecl>,
    pub(crate) blocks: Vec<rustc_gen::rustc_public::mir::BasicBlock>,
    pub(crate) stmts: Vec<rustc_gen::rustc_public::mir::Statement>,
    /// Target block and scope state of each bound label.
    pub(crate) label_targets: FxHashMap<LabelId, (usize, ScopeState)>,
    pub(crate) label_discriminants: FxHashMap<LabelId, u128>,
    pub(crate) pending_gotos: Vec<(usize, LabelId, ScopeState)>,
    pub(crate) pending_indirect_gotos: Vec<(usize, ScopeState)>,
    pub(crate) span: RustSpan,
    pub(crate) last_stmt_span: Option<RustSpan>,
    pub(crate) c_variadic_local: Option<usize>,
    pub(crate) var_debug_info: Vec<VarDebugInfo>,
    pub(crate) local_vdi_map: FxHashMap<usize, usize>,
    pub(crate) scopes: Vec<Scope>,
    pub(crate) next_scope: u32,
    /// Buffered `(template, span)` of naked-function asm statements,
    /// concatenated into one diverging `naked_asm!` at fallthrough.
    pub(crate) naked_templates: Vec<(String, RustSpan)>,
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

    /// `()` or `core::ffi::c_void` (which `libc` re-exports): the MIR
    /// counterpart of `HirCtx::is_void_ty`, for pointer arithmetic and
    /// casts over `void *`/`c_void *`.
    pub(crate) fn is_void_ty(&self, ty: Ty) -> bool {
        matches!(ty.kind(), TyKind::RigidTy(RigidTy::Tuple(l)) if l.is_empty())
            || matches!(ty.kind(), TyKind::RigidTy(RigidTy::Adt(adt, _)) if adt == self.wellknown_defs.c_void)
    }

    pub(crate) fn current_scope(&self) -> u32 {
        self.scopes.last().map_or(0, |scope| scope.id)
    }

    /// Snapshot of the active scopes, used to insert the
    /// `StorageLive`/`StorageDead` transitions a `goto` needs and to reject
    /// jumps into a VLA's scope.
    pub(crate) fn scope_state(&self) -> ScopeState {
        self.scopes.clone()
    }

    pub(crate) fn enter_scope(&mut self) {
        let id = self.next_scope;
        self.next_scope += 1;
        self.scopes.push(Scope {
            id,
            ..Scope::default()
        });
    }

    pub(crate) fn record_local_decl(&mut self, local_index: usize) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.locals.push(local_index);
        }
    }

    /// Records that a VLA identifier's scope starts here. Storage for every
    /// local in the block goes live on block entry, but a jump into the block
    /// before the declaration is legal, so the VLA is not in scope until now.
    pub(crate) fn record_vla_decl(&mut self, local_index: usize) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.vlas.push(local_index);
        }
    }

    /// Marks every local declared directly in `stmts` live. C gives a local
    /// the lifetime of its whole block (C11 6.2.4p5), so this runs on block
    /// entry rather than at the declaration: a `goto` past the declaration
    /// still leaves the local alive.
    pub(crate) fn declare_scope_locals(&mut self, stmts: &[HirStmt]) {
        for stmt in stmts {
            if let HirStmt::Decl(decl) = stmt {
                let local_index = self.local_to_index(decl.local);
                self.push_statement(MirStatementKind::StorageLive(local_index), decl.span);
                self.record_local_decl(local_index);
            }
        }
    }

    /// Lowers the statements of the current scope: marks its locals live on
    /// entry, then lowers each statement.
    pub(crate) fn lower_stmts(&mut self, stmts: &[HirStmt]) {
        self.declare_scope_locals(stmts);
        for stmt in stmts {
            self.lower_stmt(stmt);
        }
    }

    pub(crate) fn exit_scope(&mut self, span: RustSpan) {
        // Take the locals without popping the scope yet: `push_statement`
        // attributes each `StorageDead` to the scope being exited.
        let locals = match self.scopes.last_mut() {
            Some(scope) => std::mem::take(&mut scope.locals),
            None => Vec::new(),
        };
        for local_index in locals {
            self.push_statement(MirStatementKind::StorageDead(local_index), span);
        }
        self.scopes.pop();
    }

    pub(crate) fn emit_nullary_call(
        &mut self,
        func: FnDef,
        ret_ty: Ty,
        dst: Place,
        span: RustSpan,
    ) {
        let sig = func
            .ty()
            .kind()
            .fn_sig()
            .expect("nullary callee has no signature")
            .skip_binder();
        let generic_args = infer_fn_generic_args(func, &sig, &[], ret_ty);
        self.emit_call_block(
            fn_const_operand(func, generic_args, span),
            vec![],
            dst,
            span,
        );
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
