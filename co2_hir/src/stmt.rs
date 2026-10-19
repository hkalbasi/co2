use rustc_data_structures::fx::FxHashMap;

use co2_ast::{
    BinOp as ParsedBinOp, CompoundStatement, Expression, ForInit, Spanned, Statement,
    StatementOrDeclaration,
};
use co2_crate_sig::LocalResolver;
use la_arena::Arena;
use rustc_public_generative::rustc_public::ty::{Span as RustSpan, Ty};

use crate::HirDecl;
use crate::expr::{HirExpr, HirExprKind};
use crate::item::{HirLocal, LabelId, LocalId};
use crate::resolver::HirCtx;
use crate::ty::{integer_promote_ty, is_integer_ty};

#[derive(Clone, Debug)]
pub enum HirStmt {
    Decl(HirDecl),
    Expr(HirExpr),
    Label(LabelId, RustSpan),
    Goto(LabelId, RustSpan),
    IndirectGoto(HirExpr, RustSpan),
    Return(Option<HirExpr>, RustSpan),
    If {
        cond: HirExpr,
        then_stmts: Vec<HirStmt>,
        else_stmts: Vec<HirStmt>,
        span: RustSpan,
    },
    Block(Vec<HirStmt>, RustSpan),
    Asm(HirAsm),
}

/// Lowered GNU extended asm: operand exprs are already lowered; MIR decides
/// reg classes, template rewriting, and clobber handling.
#[derive(Clone, Debug)]
pub struct HirAsm {
    pub template: String,
    /// `true` for basic asm: template is literal assembler text.
    pub is_basic: bool,
    /// `true` inside a naked function: buffer the template for the single
    /// diverging `naked_asm!` instead of emitting a terminator.
    pub naked: bool,
    pub outputs: Vec<HirAsmOperand>,
    pub inputs: Vec<HirAsmOperand>,
    pub clobbers: Vec<String>,
    pub is_volatile: bool,
    pub span: RustSpan,
}

#[derive(Clone, Debug)]
pub struct HirAsmOperand {
    pub name: Option<String>,
    pub constraint: String,
    pub expr: HirExpr,
}

impl HirCtx<'_> {
    /// Resolve a forward `goto` at `Label`: if any VLA declared between the
    /// `goto` and this label is still in scope here, the jump entered the VLA's
    /// scope, which C11 6.8.6.1p1 forbids.
    fn check_goto_into_vla_scope(&self, label: LabelId) {
        let label_depth = self.block_depth.get();
        let mut offending = None;
        self.pending_gotos
            .borrow_mut()
            .retain(|(goto_label, _goto_depth, decl_len, _span)| {
                if *goto_label != label {
                    return true;
                }
                let vla = self.vla.borrow();
                if let Some((_, span)) = vla.decls[*decl_len..]
                    .iter()
                    .find(|(depth, _)| *depth <= label_depth)
                {
                    offending = Some(*span);
                }
                false
            });
        if let Some(span) = offending {
            self.terminate_with_error(span, "jump into scope of variable length array");
        }
    }

    pub(crate) fn lower_compound_items(
        &self,
        compound: CompoundStatement<LocalResolver>,
        out: &mut Vec<HirStmt>,
        locals: &mut Arena<HirLocal>,
        local_map: &mut FxHashMap<usize, LocalId>,
    ) {
        let depth = self.block_depth.get();
        self.block_depth.set(depth + 1);
        for (stmt_or_decl, _) in compound.statements {
            match stmt_or_decl {
                StatementOrDeclaration::Statement((stmt, span)) => {
                    self.lower_stmt(stmt, span, out, locals, local_map);
                }
                StatementOrDeclaration::Declaration((decl, _span)) => {
                    self.lower_decl(decl, out, locals, local_map)
                        .unwrap_or_else(|err| self.terminate_with_spanned_error(err));
                }
            }
        }
        self.block_depth.set(depth);
    }

    /// Naked-function path (GNU `__attribute__((naked))`): the body must
    /// be basic asm only — no C statements, no declarations, no extended
    /// asm with operands.
    pub(crate) fn lower_naked_compound_items(
        &self,
        compound: CompoundStatement<LocalResolver>,
        out: &mut Vec<HirStmt>,
        locals: &mut Arena<HirLocal>,
        local_map: &mut FxHashMap<usize, LocalId>,
    ) {
        for (stmt_or_decl, _) in compound.statements {
            match stmt_or_decl {
                StatementOrDeclaration::Statement((stmt, span)) => {
                    let Statement::Asm((asm, _)) = stmt else {
                        self.terminate_with_error(
                            span,
                            "naked function body must contain only basic asm statements",
                        );
                    };
                    self.lower_naked_asm(asm, span, out, locals, local_map);
                }
                StatementOrDeclaration::Declaration((_, span)) => {
                    self.terminate_with_error(
                        span,
                        "naked function body cannot contain declarations; use basic asm only",
                    );
                }
            }
        }
    }

    /// Lower one basic asm statement inside a naked function.
    fn lower_naked_asm(
        &self,
        asm: co2_ast::AsmStatement<LocalResolver>,
        parser_span: co2_ast::Span,
        out: &mut Vec<HirStmt>,
        locals: &mut Arena<HirLocal>,
        local_map: &mut FxHashMap<usize, LocalId>,
    ) {
        let span = self.to_rust_span(parser_span);
        if !asm.outputs.is_empty() || !asm.inputs.is_empty() {
            self.terminate_with_error(
                parser_span,
                "naked functions cannot use extended asm with operands; use basic asm only",
            );
        }
        // Basic asm has no operand exprs to lower; the template is buffered
        // for the single diverging `naked_asm!` (see `HirAsm::naked`).
        let _ = (locals, local_map);
        out.push(HirStmt::Asm(HirAsm {
            template: asm.template,
            is_basic: asm.is_basic,
            naked: true,
            outputs: Vec::new(),
            inputs: Vec::new(),
            clobbers: asm.clobbers,
            is_volatile: asm.is_volatile,
            span,
        }));
    }

    /// Lower a `case` bound (`case expr:` or one side of `case lo ... hi:`):
    /// const-check, lower, require integer-like, coerce to the switch
    /// discriminant type. Terminates with a diagnostic on any failure.
    pub(crate) fn lower_case_bound(
        &self,
        expr: Spanned<Expression<LocalResolver>>,
        discr_ty: Ty,
        locals: &mut Arena<HirLocal>,
        local_map: &mut FxHashMap<usize, LocalId>,
    ) -> HirExpr {
        let bound_span = expr.1;
        if let Some(span) =
            co2_crate_sig::expr_contains_vla_sizeof(&expr, &self.decl_resolver.vla_locals())
        {
            self.terminate_with_error(span, "case label is not an integer constant expression");
        }
        if let Err(err) = self.eval_const_expr_in_scope(&expr, locals, local_map) {
            self.terminate_with_spanned_error(err);
        }
        let bound = self
            .lower_expr(expr, locals, local_map)
            .unwrap_or_else(|err| self.terminate_with_spanned_error(err));
        if !is_integer_ty(bound.ty) {
            self.terminate_with_error(
                bound_span,
                &format!(
                    "switch case expression must be integer-like, got {}",
                    self.format_ty(bound.ty)
                ),
            );
        }
        let bound_ty = bound.ty;
        self.coerce_expr_to_type(&bound, discr_ty)
            .unwrap_or_else(|| {
                self.terminate_with_error(
                    bound_span,
                    &format!(
                        "switch case expression type mismatch: expected {}, got {}",
                        self.format_ty(discr_ty),
                        self.format_ty(bound_ty)
                    ),
                )
            })
    }

    pub(crate) fn lower_stmt(
        &self,
        stmt: Statement<LocalResolver>,
        parser_span: co2_ast::Span,
        out: &mut Vec<HirStmt>,
        locals: &mut Arena<HirLocal>,
        local_map: &mut FxHashMap<usize, LocalId>,
    ) {
        let span = self.to_rust_span(parser_span);
        match stmt {
            Statement::Empty => {}
            Statement::BreakCo2 => {
                panic!("break co2!");
            }
            Statement::Break => {
                let Some(label) = self.current_break_label() else {
                    self.terminate_with_error(parser_span, "Break statement outside of loop body.");
                };
                out.push(HirStmt::Goto(label, span));
            }
            Statement::Continue => {
                let Some(label) = self.current_continue_label() else {
                    self.terminate_with_error(
                        parser_span,
                        "Continue statement outside of loop body.",
                    );
                };
                out.push(HirStmt::Goto(label, span));
            }
            Statement::Case { expr } => {
                let Some((discr_local, discr_ty)) = self.current_switch_discr() else {
                    self.terminate_with_error(expr.1, "case label outside of switch body");
                };
                let case_label = self.fresh_label();
                let case_expr = self.lower_case_bound(expr, discr_ty, locals, local_map);
                let discr_expr = HirExpr {
                    kind: HirExprKind::Local(discr_local),
                    ty: discr_ty,
                    span,
                };
                let cond = self
                    .lower_binop_from_lowered(
                        discr_expr,
                        case_expr,
                        ParsedBinOp::Eq,
                        span,
                        parser_span,
                        false,
                    )
                    .unwrap_or_else(|err| self.terminate_with_spanned_error(err));
                self.register_case(cond, case_label);
                out.push(HirStmt::Label(case_label, span));
            }
            Statement::CaseRange { lo, hi } => {
                let Some((discr_local, discr_ty)) = self.current_switch_discr() else {
                    self.terminate_with_error(lo.1, "case label outside of switch body");
                };
                let case_label = self.fresh_label();
                let lo_expr = self.lower_case_bound(lo, discr_ty, locals, local_map);
                let hi_expr = self.lower_case_bound(hi, discr_ty, locals, local_map);
                let ge = self
                    .lower_binop_from_lowered(
                        HirExpr {
                            kind: HirExprKind::Local(discr_local),
                            ty: discr_ty,
                            span,
                        },
                        lo_expr,
                        ParsedBinOp::Ge,
                        span,
                        parser_span,
                        false,
                    )
                    .unwrap_or_else(|err| self.terminate_with_spanned_error(err));
                let le = self
                    .lower_binop_from_lowered(
                        HirExpr {
                            kind: HirExprKind::Local(discr_local),
                            ty: discr_ty,
                            span,
                        },
                        hi_expr,
                        ParsedBinOp::Le,
                        span,
                        parser_span,
                        false,
                    )
                    .unwrap_or_else(|err| self.terminate_with_spanned_error(err));
                let cond = self
                    .lower_binop_from_lowered(ge, le, ParsedBinOp::And, span, parser_span, false)
                    .unwrap_or_else(|err| self.terminate_with_spanned_error(err));
                self.register_case(cond, case_label);
                out.push(HirStmt::Label(case_label, span));
            }
            Statement::Default { keyword_span } => {
                if !self.in_switch() {
                    self.terminate_with_error(keyword_span, "default label outside of switch body");
                }
                let label = self.fresh_label();
                self.register_default(label, keyword_span);
                out.push(HirStmt::Label(label, span));
            }
            Statement::Goto(name) => {
                let label = self.resolve_or_insert_label(name.0);
                self.pending_gotos.borrow_mut().push((
                    label,
                    self.block_depth.get(),
                    self.vla.borrow().decls.len(),
                    parser_span,
                ));
                out.push(HirStmt::Goto(label, span));
            }
            Statement::IndirectGoto(expr) => {
                let expr = self
                    .lower_expr(expr, locals, local_map)
                    .unwrap_or_else(|err| self.terminate_with_spanned_error(err));
                if !self.is_condition_ty(expr.ty) {
                    self.terminate_with_error(
                        parser_span,
                        &format!(
                            "indirect goto operand must be scalar-like, got {:?}",
                            expr.ty
                        ),
                    );
                }
                out.push(HirStmt::IndirectGoto(expr, span));
            }
            Statement::Label { name } => {
                let label = self.resolve_or_insert_label(name.0);
                self.check_goto_into_vla_scope(label);
                out.push(HirStmt::Label(label, span));
            }
            Statement::Return(expr) => {
                if let Some(expr) = expr {
                    let expr_span = expr.1;
                    let mut expr = self
                        .lower_expr((expr.0, expr.1), locals, local_map)
                        .unwrap_or_else(|err| self.terminate_with_spanned_error(err));
                    let returns_vla_object =
                        self.vla_shape_of_expr(&expr).is_some_and(|(_, is_ptr)| !is_ptr);
                    self.array_to_pointer_decay_if_array(&mut expr);
                    self.fn_def_to_fn_ptr_decay_for_expected(&mut expr, self.ret_ty);
                    if returns_vla_object && !self.ty_matches_expected(self.ret_ty, expr.ty) {
                        self.terminate_with_error(
                            expr_span,
                            "cannot return a variable length array",
                        );
                    }
                    if let Some(coerced) = self.coerce_expr_to_type(&expr, self.ret_ty) {
                        expr = coerced;
                    }
                    if !self.ty_matches_expected(self.ret_ty, expr.ty) {
                        self.terminate_with_error(
                            expr_span,
                            &format!(
                                "return type mismatch: expected {}, got {}",
                                self.format_ty(self.ret_ty),
                                self.format_ty(expr.ty)
                            ),
                        );
                    }
                    out.push(HirStmt::Return(Some(expr), span));
                } else {
                    if !self.is_void_ty(self.ret_ty) {
                        self.terminate_with_error(
                            parser_span,
                            &format!(
                                "`return;` is not valid for functions returning value {}",
                                self.format_ty(self.ret_ty),
                            ),
                        );
                    }
                    out.push(HirStmt::Return(None, span));
                }
            }
            Statement::Expression(expr) => {
                let expr = self
                    .lower_expr(expr, locals, local_map)
                    .unwrap_or_else(|err| self.terminate_with_spanned_error(err));
                out.push(HirStmt::Expr(expr));
            }
            Statement::Asm(asm) => {
                let (asm, _) = asm;
                let mut outputs = Vec::with_capacity(asm.outputs.len());
                for op in asm.outputs {
                    let expr = self
                        .lower_expr(op.expr, locals, local_map)
                        .unwrap_or_else(|err| self.terminate_with_spanned_error(err));
                    outputs.push(HirAsmOperand {
                        name: op.name,
                        constraint: op.constraint,
                        expr,
                    });
                }
                let mut inputs = Vec::with_capacity(asm.inputs.len());
                for op in asm.inputs {
                    let expr = self
                        .lower_expr(op.expr, locals, local_map)
                        .unwrap_or_else(|err| self.terminate_with_spanned_error(err));
                    inputs.push(HirAsmOperand {
                        name: op.name,
                        constraint: op.constraint,
                        expr,
                    });
                }
                out.push(HirStmt::Asm(HirAsm {
                    template: asm.template,
                    is_basic: asm.is_basic,
                    naked: false,
                    outputs,
                    inputs,
                    clobbers: asm.clobbers,
                    is_volatile: asm.is_volatile,
                    span,
                }));
            }
            Statement::If {
                cond,
                then_branch,
                else_branch,
            } => {
                let cond = self
                    .lower_condition(cond, locals, local_map)
                    .unwrap_or_else(|err| self.terminate_with_spanned_error(err));
                let mut then_map = local_map.clone();
                let mut then_stmts = Vec::new();
                self.lower_stmt(
                    then_branch.0,
                    then_branch.1,
                    &mut then_stmts,
                    locals,
                    &mut then_map,
                );

                let mut else_stmts = Vec::new();
                if let Some(else_branch) = else_branch {
                    let mut else_map = local_map.clone();
                    self.lower_stmt(
                        else_branch.0,
                        else_branch.1,
                        &mut else_stmts,
                        locals,
                        &mut else_map,
                    );
                }

                out.push(HirStmt::If {
                    cond,
                    then_stmts,
                    else_stmts,
                    span,
                });
            }
            Statement::While { cond, body } => {
                let cond = self
                    .lower_condition(cond, locals, local_map)
                    .unwrap_or_else(|err| self.terminate_with_spanned_error(err));
                let cond_label = self.fresh_label();
                let body_label = self.fresh_label();
                let end_label = self.fresh_label();

                out.push(HirStmt::Label(cond_label, span));
                out.push(HirStmt::If {
                    cond,
                    then_stmts: vec![HirStmt::Goto(body_label, span)],
                    else_stmts: vec![HirStmt::Goto(end_label, span)],
                    span,
                });
                out.push(HirStmt::Label(body_label, span));

                let mut body_map = local_map.clone();
                self.enter_loop(cond_label, end_label);
                self.lower_stmt(body.0, body.1, out, locals, &mut body_map);
                self.exit_loop();
                out.push(HirStmt::Goto(cond_label, span));
                out.push(HirStmt::Label(end_label, span));
            }
            Statement::DoWhile { body, cond } => {
                let cond = self
                    .lower_condition(cond, locals, local_map)
                    .unwrap_or_else(|err| self.terminate_with_spanned_error(err));
                let body_label = self.fresh_label();
                let cond_label = self.fresh_label();
                let end_label = self.fresh_label();

                out.push(HirStmt::Goto(body_label, span));
                out.push(HirStmt::Label(body_label, span));
                let mut body_map = local_map.clone();
                self.enter_loop(cond_label, end_label);
                self.lower_stmt(body.0, body.1, out, locals, &mut body_map);
                self.exit_loop();
                out.push(HirStmt::Label(cond_label, span));
                out.push(HirStmt::If {
                    cond,
                    then_stmts: vec![HirStmt::Goto(body_label, span)],
                    else_stmts: vec![HirStmt::Goto(end_label, span)],
                    span,
                });
                out.push(HirStmt::Label(end_label, span));
            }
            Statement::For {
                init,
                cond,
                post,
                body,
            } => {
                let mut loop_map = local_map.clone();
                // The `for` statement is its own scope: a for-init declaration
                // (and anything else declared here) dies at the loop's end.
                let mut for_stmts = Vec::new();
                if let Some(init) = init {
                    match init {
                        ForInit::Expression(init) => {
                            let init = self
                                .lower_expr(init, locals, &mut loop_map)
                                .unwrap_or_else(|err| self.terminate_with_spanned_error(err));
                            for_stmts.push(HirStmt::Expr(init));
                        }
                        ForInit::Declaration((decl, _)) => {
                            self.lower_decl(decl, &mut for_stmts, locals, &mut loop_map)
                                .unwrap_or_else(|err| self.terminate_with_spanned_error(err));
                        }
                    }
                }
                let cond = if let Some(cond) = cond {
                    self.lower_condition(cond, locals, &mut loop_map)
                        .unwrap_or_else(|err| self.terminate_with_spanned_error(err))
                } else {
                    HirExpr {
                        kind: HirExprKind::ConstInt(1),
                        ty: Ty::bool_ty(),
                        span,
                    }
                };

                let cond_label = self.fresh_label();
                let body_label = self.fresh_label();
                let continue_label = self.fresh_label();
                let end_label = self.fresh_label();

                for_stmts.push(HirStmt::Label(cond_label, span));
                for_stmts.push(HirStmt::If {
                    cond,
                    then_stmts: vec![HirStmt::Goto(body_label, span)],
                    else_stmts: vec![HirStmt::Goto(end_label, span)],
                    span,
                });
                for_stmts.push(HirStmt::Label(body_label, span));

                let mut body_map = local_map.clone();
                body_map.extend(loop_map.clone());
                self.enter_loop(continue_label, end_label);
                self.lower_stmt(body.0, body.1, &mut for_stmts, locals, &mut body_map);
                self.exit_loop();
                for_stmts.push(HirStmt::Label(continue_label, span));
                if let Some(post) = post {
                    let post = self
                        .lower_expr(post, locals, &mut body_map)
                        .unwrap_or_else(|err| self.terminate_with_spanned_error(err));
                    for_stmts.push(HirStmt::Expr(post));
                }
                for_stmts.push(HirStmt::Goto(cond_label, span));
                for_stmts.push(HirStmt::Label(end_label, span));
                out.push(HirStmt::Block(for_stmts, span));
            }
            Statement::Switch { expr, body } => {
                let switch_expr_span = expr.1;
                let discr = self
                    .lower_expr(expr, locals, local_map)
                    .unwrap_or_else(|err| self.terminate_with_spanned_error(err));
                if !is_integer_ty(discr.ty) {
                    self.terminate_with_error(
                        switch_expr_span,
                        &format!(
                            "switch expression must be integer-like, got {}",
                            self.format_ty(discr.ty)
                        ),
                    );
                }
                let discr_ty = integer_promote_ty(discr.ty);
                let discr_init = if discr_ty != discr.ty {
                    HirExpr {
                        kind: HirExprKind::Cast(Box::new(discr)),
                        ty: discr_ty,
                        span,
                    }
                } else {
                    discr
                };
                let discr_local = locals.alloc(HirLocal {
                    name: format!("__switch_discr{}", locals.len()),
                    ty: discr_ty,
                    span,
                    read_only: false,
                });
                out.push(HirStmt::Decl(HirDecl {
                    local: discr_local,
                    initializer: Some(discr_init),
                    span,
                }));

                let end_label = self.fresh_label();
                let mut body_stmts = Vec::new();

                self.enter_switch_scope(discr_local, discr_ty, end_label);
                self.lower_stmt(body.0, body.1, &mut body_stmts, locals, local_map);
                let scope = self.exit_switch_scope();

                for (cond, label) in scope.case_labels {
                    out.push(HirStmt::If {
                        cond: self.condition_to_bool(cond, parser_span),
                        then_stmts: vec![HirStmt::Goto(label, span)],
                        else_stmts: vec![],
                        span,
                    });
                }
                out.push(HirStmt::Goto(
                    scope.default_label.unwrap_or(end_label),
                    span,
                ));
                out.extend(body_stmts);
                out.push(HirStmt::Label(end_label, span));
            }
            Statement::Compound(nested) => {
                let outer_scope = local_map.clone();
                let mut block_stmts = Vec::new();
                self.lower_compound_items(nested.0, &mut block_stmts, locals, local_map);
                *local_map = outer_scope;
                out.push(HirStmt::Block(block_stmts, span));
            }
        }
    }
}
