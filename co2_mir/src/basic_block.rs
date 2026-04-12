use co2_hir::{HirDecl, HirExpr, HirExprKind, HirStmt, LabelId};
use rustc_public_generative::rustc_public::{
    mir::{
        Rvalue, Statement as MirStatement, StatementKind as MirStatementKind, SwitchTargets,
        Terminator as MirTerminator, TerminatorKind, UnwindAction,
    },
    ty::{GenericArgKind, RigidTy, Span as RustSpan, Ty, TyKind},
};

use crate::{build::Builder, place::place};

impl Builder {
    pub(crate) fn lower_stmt(&mut self, stmt: &HirStmt) {
        match stmt {
            HirStmt::Decl(HirDecl {
                local, initializer, ..
            }) => {
                if let Some(init) = initializer {
                    let local_index = self.local_to_index(*local);
                    if let HirExprKind::Zeroed = &init.kind {
                        let local_ty = self.locals[local_index].ty;
                        self.lower_zeroed_to_destination(place(local_index), init.span, local_ty);
                    } else {
                        let value = self.lower_expr_to_operand(init);
                        self.stmts.push(MirStatement {
                            kind: MirStatementKind::Assign(place(local_index), Rvalue::Use(value)),
                            span: init.span,
                        });
                    }
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
                        self.stmts.push(MirStatement {
                            kind: MirStatementKind::Assign(place(0), Rvalue::Use(value)),
                            span: expr.span,
                        });
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
        }
    }

    pub(crate) fn lower_if_stmt(
        &mut self,
        cond: &HirExpr,
        then_stmts: &[HirStmt],
        else_stmts: &[HirStmt],
        span: RustSpan,
    ) {
        let cond_is_maybe_uninit_fn_ptr = matches!(
            cond.ty.kind(),
            TyKind::RigidTy(RigidTy::Adt(_, args))
                if args.0.len() == 1
                    && matches!(args.0[0], GenericArgKind::Type(ty) if matches!(ty.kind(), TyKind::RigidTy(RigidTy::FnPtr(_))))
        );
        let cond_expr = if matches!(
            cond.ty.kind(),
            TyKind::RigidTy(RigidTy::RawPtr(_, _) | RigidTy::FnPtr(_))
        ) || cond_is_maybe_uninit_fn_ptr
        {
            HirExpr {
                kind: HirExprKind::Cast(Box::new(cond.clone())),
                ty: Ty::usize_ty(),
                span: cond.span,
            }
        } else {
            cond.clone()
        };
        let cond_op = self.lower_expr_to_operand(&cond_expr);
        let entry_span = span;
        let entry_idx = self.blocks.len();
        self.blocks
            .push(rustc_public_generative::rustc_public::mir::BasicBlock {
                statements: std::mem::take(&mut self.stmts),
                terminator: MirTerminator {
                    kind: TerminatorKind::SwitchInt {
                        discr: cond_op,
                        targets: SwitchTargets::new(vec![(0, usize::MAX)], usize::MAX),
                    },
                    span: entry_span,
                },
            });

        let then_bb = self.blocks.len();
        for stmt in then_stmts {
            self.lower_stmt(stmt);
        }
        let then_exit =
            self.push_terminator(TerminatorKind::Goto { target: usize::MAX }, entry_span);

        let else_bb = self.blocks.len();
        for stmt in else_stmts {
            self.lower_stmt(stmt);
        }
        let else_exit =
            self.push_terminator(TerminatorKind::Goto { target: usize::MAX }, entry_span);

        let join_bb = self.blocks.len();
        self.patch_goto_target(then_exit, join_bb);
        self.patch_goto_target(else_exit, join_bb);
        self.patch_switch_targets(entry_idx, then_bb, else_bb);
    }

    pub(crate) fn terminate_fallthrough(&mut self) {
        self.push_terminator(TerminatorKind::Return, self.span);
        self.patch_pending_gotos();
    }

    fn bind_label(&mut self, label: LabelId, span: RustSpan) {
        let target = if self.stmts.is_empty() {
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
            let target = *self
                .label_blocks
                .get(&label)
                .unwrap_or_else(|| panic!("unresolved label id `{label:?}`"));
            self.patch_goto_target(bb, target);
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
        self.blocks
            .push(rustc_public_generative::rustc_public::mir::BasicBlock {
                statements: std::mem::take(&mut self.stmts),
                terminator: MirTerminator {
                    kind: TerminatorKind::Call {
                        func,
                        args,
                        destination,
                        target: Some(next),
                        unwind: UnwindAction::Continue,
                    },
                    span,
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
                terminator: MirTerminator { kind, span },
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
}
