use co2_hir::{HirDecl, HirExpr, HirExprKind, HirStmt};
use rustc_public_generative::rustc_public::{
    mir::{
        Rvalue, Statement as MirStatement, StatementKind as MirStatementKind, SwitchTargets,
        Terminator as MirTerminator, TerminatorKind, UnwindAction,
    },
    ty::{IntTy, Span as RustSpan, Ty},
};

use crate::{build::Builder, place::place};

impl Builder<'_> {
    pub(crate) fn lower_stmt(&mut self, stmt: &HirStmt) {
        match stmt {
            HirStmt::Decl(HirDecl {
                local, initializer, ..
            }) => {
                if let Some(init) = initializer {
                    let local_index = self.local_to_index(*local);
                    if let HirExprKind::Aggregate { args } = &init.kind {
                        let rustc_public_generative::rustc_public::ty::TyKind::RigidTy(
                            rustc_public_generative::rustc_public::ty::RigidTy::Adt(adt, adt_args),
                        ) = init.ty.kind()
                        else {
                            panic!("aggregate initializer expects adt type, got {:?}", init.ty);
                        };
                        let mut operands = Vec::with_capacity(args.len());
                        for arg in args {
                            operands.push(self.lower_expr_to_operand(arg));
                        }
                        self.stmts.push(MirStatement {
                            kind: MirStatementKind::Assign(
                                place(local_index),
                                Rvalue::Aggregate(
                                    rustc_public_generative::rustc_public::mir::AggregateKind::Adt(
                                        adt,
                                        crate::build::variant_idx(0),
                                        adt_args,
                                        None,
                                        None,
                                    ),
                                    operands,
                                ),
                            ),
                            span: init.span,
                        });
                    } else if let HirExprKind::Call { func, args } = &init.kind {
                        let local_ty = self.locals[local_index].ty;
                        self.lower_call_to_destination(
                            func,
                            args,
                            init.span,
                            place(local_index),
                            local_ty,
                        );
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
            HirStmt::Return(expr, span) => {
                if let Some(expr) = expr {
                    if self.is_rust_entry_main {
                        let mut value = self.lower_expr_to_operand(expr);
                        if expr.ty != Ty::signed_ty(IntTy::I32) {
                            let cast_local = self.new_temp(
                                Ty::signed_ty(IntTy::I32),
                                rustc_public_generative::rustc_public::mir::Mutability::Mut,
                                expr.span,
                            );
                            self.stmts.push(MirStatement {
                                kind: MirStatementKind::Assign(
                                    place(cast_local),
                                    Rvalue::Cast(
                                        rustc_public_generative::rustc_public::mir::CastKind::IntToInt,
                                        value,
                                        Ty::signed_ty(IntTy::I32),
                                    ),
                                ),
                                span: expr.span,
                            });
                            value = rustc_public_generative::rustc_public::mir::Operand::Copy(place(cast_local));
                        }
                        self.stmts.push(MirStatement {
                            kind: MirStatementKind::Assign(
                                place(self.exit_code_local.expect("missing exit code local")),
                                Rvalue::Use(value),
                            ),
                            span: expr.span,
                        });
                    } else if let HirExprKind::Call { func, args } = &expr.kind {
                        self.lower_call_to_destination(func, args, expr.span, place(0), self.locals[0].ty);
                    } else {
                        let value = self.lower_expr_to_operand(expr);
                        self.stmts.push(MirStatement {
                            kind: MirStatementKind::Assign(place(0), Rvalue::Use(value)),
                            span: expr.span,
                        });
                    }
                }

                if self.is_rust_entry_main {
                    self.push_exit_terminator(*span);
                } else {
                    self.push_terminator(TerminatorKind::Return, *span);
                }
            }
            HirStmt::If {
                cond,
                then_stmts,
                else_stmts,
                span,
            } => {
                self.lower_if_stmt(cond, then_stmts, else_stmts, *span);
            }
            HirStmt::While {
                cond,
                body_stmts,
                span,
            } => {
                self.lower_while_stmt(cond, body_stmts, *span);
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
        let cond_op = self.lower_expr_to_operand(cond);
        let entry_span = span;
        let entry_idx = self.blocks.len();
        self.blocks.push(rustc_public_generative::rustc_public::mir::BasicBlock {
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
        let then_exit = self.push_terminator(TerminatorKind::Goto { target: usize::MAX }, entry_span);

        let else_bb = self.blocks.len();
        for stmt in else_stmts {
            self.lower_stmt(stmt);
        }
        let else_exit = self.push_terminator(TerminatorKind::Goto { target: usize::MAX }, entry_span);

        let join_bb = self.blocks.len();
        self.patch_goto_target(then_exit, join_bb);
        self.patch_goto_target(else_exit, join_bb);
        self.patch_switch_targets(entry_idx, then_bb, else_bb);
    }

    pub(crate) fn lower_while_stmt(
        &mut self,
        cond: &HirExpr,
        body_stmts: &[HirStmt],
        span: RustSpan,
    ) {
        let cond_bb = self.blocks.len() + 1;
        self.push_terminator(TerminatorKind::Goto { target: cond_bb }, span);

        let cond_op = self.lower_expr_to_operand(cond);
        let cond_idx = self.push_terminator(
            TerminatorKind::SwitchInt {
                discr: cond_op,
                targets: SwitchTargets::new(vec![(0, usize::MAX)], usize::MAX),
            },
            span,
        );

        let body_bb = self.blocks.len();
        for stmt in body_stmts {
            self.lower_stmt(stmt);
        }
        self.push_terminator(TerminatorKind::Goto { target: cond_bb }, span);

        let exit_bb = self.blocks.len();
        self.patch_switch_targets(cond_idx, body_bb, exit_bb);
    }

    pub(crate) fn terminate_fallthrough(&mut self) {
        if self.is_rust_entry_main {
            self.push_exit_terminator(self.span);
        } else {
            self.push_terminator(TerminatorKind::Return, self.span);
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
        self.blocks.push(rustc_public_generative::rustc_public::mir::BasicBlock {
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

    pub(crate) fn push_exit_terminator(
        &mut self,
        span: rustc_public_generative::rustc_public::ty::Span,
    ) {
        self.push_terminator(
            TerminatorKind::Call {
                func: crate::build::fn_const_operand(
                    self.exit_fn.expect("missing exit fn"),
                    vec![],
                    span,
                ),
                args: vec![rustc_public_generative::rustc_public::mir::Operand::Copy(place(
                    self.exit_code_local.expect("missing exit code local"),
                ))],
                destination: place(0),
                target: None,
                unwind: UnwindAction::Continue,
            },
            span,
        );
    }

    pub(crate) fn push_terminator(
        &mut self,
        kind: TerminatorKind,
        span: rustc_public_generative::rustc_public::ty::Span,
    ) -> usize {
        let idx = self.blocks.len();
        self.blocks.push(rustc_public_generative::rustc_public::mir::BasicBlock {
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

    pub(crate) fn patch_switch_targets(&mut self, block_idx: usize, then_bb: usize, else_bb: usize) {
        match &mut self.blocks[block_idx].terminator.kind {
            TerminatorKind::SwitchInt { targets, .. } => {
                *targets = SwitchTargets::new(vec![(0, else_bb)], then_bb);
            }
            _ => panic!("expected switchint terminator at block {block_idx}"),
        }
    }
}
