use co2_hir::{HirDecl, HirExpr, HirExprKind, HirStmt, LabelId};
use rustc_public_generative::rustc_public::{
    mir::{
        SourceInfo, Statement as MirStatement, StatementKind as MirStatementKind, SwitchTargets,
        Terminator as MirTerminator, TerminatorKind, UnwindAction,
    },
    ty::{RigidTy, Span as RustSpan, TyKind},
};

use crate::{build::Builder, operand::ptr_like_to_usize_expr, place::place};

impl Builder<'_, '_> {
    pub(crate) fn lower_stmt(&mut self, stmt: &HirStmt) {
        if let Some(span) = stmt_span(stmt) {
            self.last_stmt_span = Some(span);
        }
        match stmt {
            HirStmt::Decl(HirDecl {
                local, initializer, ..
            }) => {
                let local_index = self.local_to_index(*local);
                if let Some(init) = initializer {
                    let value = self.lower_expr_to_operand(init);
                    self.emit_assign_use(place(local_index), value, init.span);
                }
                if let Some(&vdi_idx) = self.local_vdi_map.get(&local_index) {
                    self.var_debug_info[vdi_idx].source_info.scope = self.current_scope();
                }
            }
            HirStmt::Expr(expr) => {
                let _ = self.lower_expr_to_operand(expr);
            }
            HirStmt::Label(label, span) => {
                self.bind_label(*label, *span);
            }
            HirStmt::Goto(label, span) => {
                let bb = self.push_terminator(TerminatorKind::Goto { target: usize::MAX }, *span);
                self.pending_gotos.push((bb, *label));
            }
            HirStmt::IndirectGoto(expr, span) => {
                let discr_expr = ptr_like_to_usize_expr(expr);
                let discr = self.lower_expr_to_operand(&discr_expr);
                let bb = self.push_terminator(
                    TerminatorKind::SwitchInt {
                        discr,
                        targets: SwitchTargets::new(vec![(0, usize::MAX)], usize::MAX),
                    },
                    *span,
                );
                self.pending_indirect_gotos.push(bb);
            }
            HirStmt::Return(expr, span) => {
                if let Some(expr) = expr {
                    if let HirExprKind::Call { func, args } = &expr.kind {
                        self.lower_call_to_destination(
                            func,
                            args,
                            expr.span,
                            place(0),
                            self.locals[0].ty,
                        );
                    } else {
                        let value = self.lower_expr_to_operand(expr);
                        self.emit_assign_use(place(0), value, expr.span);
                    }
                }

                self.push_terminator(TerminatorKind::Return, *span);
            }
            HirStmt::If {
                cond,
                then_stmts,
                else_stmts,
                span,
            } => {
                self.lower_if_stmt(cond, then_stmts, else_stmts, *span);
            }
            HirStmt::Block(stmts, _span) => {
                self.enter_scope();
                for stmt in stmts {
                    self.lower_stmt(stmt);
                }
                self.exit_scope();
            }
        }
    }

    pub(crate) fn lower_if_stmt(
        &mut self,
        cond: &HirExpr,
        then_stmts: &[HirStmt],
        else_stmts: &[HirStmt],
        span: RustSpan,
    ) {
        debug_assert!(matches!(cond.ty.kind(), TyKind::RigidTy(RigidTy::Bool)));
        self.lower_condition(
            cond,
            span,
            |b| {
                for stmt in then_stmts {
                    b.lower_stmt(stmt);
                }
            },
            |b| {
                for stmt in else_stmts {
                    b.lower_stmt(stmt);
                }
            },
        );
    }

    /// Lowers a boolean condition into a branch on the taken/not-taken sides.
    ///
    /// When the condition is a compile-time constant (`1`, `0`, `true`,
    /// `false`, a cast of one of those, or `!` of a constant), a direct
    /// `Goto` to the taken branch is emitted, but both branches are still
    /// lowered: C labels have function scope, so code that looks dead may
    /// be reachable via `goto` from elsewhere in the function.
    pub(crate) fn lower_condition(
        &mut self,
        cond: &HirExpr,
        span: RustSpan,
        then: impl FnOnce(&mut Self),
        els: impl FnOnce(&mut Self),
    ) {
        debug_assert!(matches!(cond.ty.kind(), TyKind::RigidTy(RigidTy::Bool)));
        let constant = constant_condition(cond);
        let entry_kind = match constant {
            Some(_) => TerminatorKind::Goto { target: usize::MAX },
            None => TerminatorKind::SwitchInt {
                discr: self.lower_expr_to_operand(cond),
                targets: SwitchTargets::new(vec![(0, usize::MAX)], usize::MAX),
            },
        };
        let entry_bb = self.blocks.len();
        self.blocks
            .push(rustc_public_generative::rustc_public::mir::BasicBlock {
                statements: std::mem::take(&mut self.stmts),
                terminator: MirTerminator {
                    kind: entry_kind,
                    source_info: SourceInfo {
                        span,
                        scope: self.current_scope(),
                    },
                },
            });

        let then_start = self.blocks.len();
        then(self);
        let then_exit = self.push_terminator(TerminatorKind::Goto { target: usize::MAX }, span);

        let else_start = self.blocks.len();
        els(self);
        let else_exit = self.push_terminator(TerminatorKind::Goto { target: usize::MAX }, span);

        let join_bb = self.blocks.len();
        self.patch_goto_target(then_exit, join_bb);
        self.patch_goto_target(else_exit, join_bb);
        match constant {
            Some(true) => self.patch_goto_target(entry_bb, then_start),
            Some(false) => self.patch_goto_target(entry_bb, else_start),
            None => self.patch_switch_targets(entry_bb, then_start, else_start),
        }
    }

    pub(crate) fn terminate_fallthrough(&mut self) {
        let span = self.fallthrough_span();
        self.push_terminator(TerminatorKind::Return, span);
        self.patch_pending_gotos();
    }

    fn fallthrough_span(&self) -> RustSpan {
        let span = self.last_stmt_span.unwrap_or(self.span);
        let (file_id, lo, hi) = self.ctx.span_data(span);
        if hi > lo {
            self.ctx.span_in_file(file_id, lo, lo + 1)
        } else {
            self.span
        }
    }

    fn bind_label(&mut self, label: LabelId, span: RustSpan) {
        let target = if self.stmts.is_empty() && self.blocks.is_empty() {
            self.push_terminator(TerminatorKind::Goto { target: 1 }, span);
            1
        } else if self.stmts.is_empty() {
            self.blocks.len()
        } else {
            let next = self.blocks.len() + 1;
            self.push_terminator(TerminatorKind::Goto { target: next }, span);
            next
        };
        self.label_blocks.insert(label, target);
    }

    fn patch_pending_gotos(&mut self) {
        for (bb, label) in std::mem::take(&mut self.pending_gotos) {
            let target = match self.label_blocks.get(&label).copied() {
                Some(target) => target,
                None => {
                    let rust_span = self.blocks[bb].terminator.source_info.span;
                    self.terminate_with_error(rust_span, "unresolved label")
                }
            };
            self.patch_goto_target(bb, target);
        }
        for bb in std::mem::take(&mut self.pending_indirect_gotos) {
            self.patch_indirect_goto_targets(bb);
        }
    }

    pub(crate) fn emit_call_block(
        &mut self,
        func: rustc_public_generative::rustc_public::mir::Operand,
        args: Vec<rustc_public_generative::rustc_public::mir::Operand>,
        destination: rustc_public_generative::rustc_public::mir::Place,
        span: rustc_public_generative::rustc_public::ty::Span,
    ) {
        let next = self.blocks.len() + 1;
        self.emit_call_terminator(func, args, destination, span, Some(next));
    }

    pub(crate) fn emit_diverging_call_block(
        &mut self,
        func: rustc_public_generative::rustc_public::mir::Operand,
        args: Vec<rustc_public_generative::rustc_public::mir::Operand>,
        destination: rustc_public_generative::rustc_public::mir::Place,
        span: rustc_public_generative::rustc_public::ty::Span,
    ) {
        // `!`-returning callee never returns: no target. Following statements
        // land in an unreachable block, so dataflow (borrowck) ignores them.
        self.emit_call_terminator(func, args, destination, span, None);
    }

    fn emit_call_terminator(
        &mut self,
        func: rustc_public_generative::rustc_public::mir::Operand,
        args: Vec<rustc_public_generative::rustc_public::mir::Operand>,
        destination: rustc_public_generative::rustc_public::mir::Place,
        span: rustc_public_generative::rustc_public::ty::Span,
        target: Option<usize>,
    ) {
        self.push_terminator(
            TerminatorKind::Call {
                func,
                args,
                destination,
                target,
                unwind: UnwindAction::Continue,
            },
            span,
        );
    }

    pub(crate) fn push_statement(
        &mut self,
        kind: MirStatementKind,
        span: rustc_public_generative::rustc_public::ty::Span,
    ) {
        self.stmts.push(MirStatement {
            kind,
            source_info: SourceInfo {
                span,
                scope: self.current_scope(),
            },
        });
    }

    pub(crate) fn push_terminator(
        &mut self,
        kind: TerminatorKind,
        span: rustc_public_generative::rustc_public::ty::Span,
    ) -> usize {
        let idx = self.blocks.len();
        self.blocks
            .push(rustc_public_generative::rustc_public::mir::BasicBlock {
                statements: std::mem::take(&mut self.stmts),
                terminator: MirTerminator {
                    kind,
                    source_info: SourceInfo {
                        span,
                        scope: self.current_scope(),
                    },
                },
            });
        idx
    }

    pub(crate) fn patch_goto_target(&mut self, block_idx: usize, target: usize) {
        match &mut self.blocks[block_idx].terminator.kind {
            TerminatorKind::Goto {
                target: goto_target,
            } => *goto_target = target,
            _ => panic!("expected goto terminator at block {block_idx}"),
        }
    }

    pub(crate) fn patch_switch_targets(
        &mut self,
        block_idx: usize,
        then_bb: usize,
        else_bb: usize,
    ) {
        match &mut self.blocks[block_idx].terminator.kind {
            TerminatorKind::SwitchInt { targets, .. } => {
                *targets = SwitchTargets::new(vec![(0, else_bb)], then_bb);
            }
            _ => panic!("expected switchint terminator at block {block_idx}"),
        }
    }

    fn patch_indirect_goto_targets(&mut self, block_idx: usize) {
        let mut branches = self
            .label_discriminants
            .iter()
            .filter_map(|(label, discr)| {
                let target = match self.label_blocks.get(label).copied() {
                    Some(target) => target,
                    None => {
                        let rust_span = self.blocks[block_idx].terminator.source_info.span;
                        self.terminate_with_error(rust_span, "unresolved label")
                    }
                };
                Some((*discr, target))
            })
            .collect::<Vec<_>>();
        branches.sort_by_key(|(discr, _)| *discr);
        let otherwise = match branches.first() {
            Some((_, target)) => *target,
            None => {
                let rust_span = self.blocks[block_idx].terminator.source_info.span;
                self.terminate_with_error(
                    rust_span,
                    "indirect goto in function with no address-of-label expressions",
                )
            }
        };
        match &mut self.blocks[block_idx].terminator.kind {
            TerminatorKind::SwitchInt { targets, .. } => {
                *targets = SwitchTargets::new(branches, otherwise);
            }
            _ => panic!("expected switchint terminator at block {block_idx}"),
        }
    }
}

fn stmt_span(stmt: &HirStmt) -> Option<RustSpan> {
    match stmt {
        HirStmt::Decl(HirDecl { initializer, .. }) => initializer.as_ref().map(|init| init.span),
        HirStmt::Expr(expr) => Some(expr.span),
        HirStmt::Label(_, span)
        | HirStmt::Goto(_, span)
        | HirStmt::IndirectGoto(_, span)
        | HirStmt::Return(_, span)
        | HirStmt::Block(_, span) => Some(*span),
        HirStmt::If { span, .. } => Some(*span),
    }
}

/// Returns `Some(taken)` when a boolean condition is a compile-time constant,
/// so the taken branch is statically known. `None` means the condition must be
/// evaluated at runtime.
fn constant_condition(cond: &HirExpr) -> Option<bool> {
    match &cond.kind {
        HirExprKind::ConstInt(value) => Some(*value != 0),
        HirExprKind::Cast(inner) => constant_condition(inner),
        HirExprKind::LogicalNot(inner) => constant_condition(inner).map(|v| !v),
        _ => None,
    }
}
