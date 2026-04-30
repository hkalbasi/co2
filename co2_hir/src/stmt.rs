use std::collections::HashMap;

use co2_ast::{CompoundStatement, ForInit, Statement, StatementOrDeclaration};
use co2_crate_sig::LocalResolver;
use la_arena::Arena;
use rustc_public_generative::rustc_public::ty::{IntTy, Span as RustSpan, Ty};

use crate::HirDecl;
use crate::expr::{HirExpr, HirExprKind, coerce_expr_to_type};
use crate::item::{HirLocal, LabelId, LocalId};
use crate::resolver::HirCtx;
use crate::ty::{is_condition_ty, needs_implicit_cast};

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
}

impl HirCtx<'_> {
    pub(crate) fn lower_compound_items(
        &self,
        compound: CompoundStatement<LocalResolver>,
        out: &mut Vec<HirStmt>,
        locals: &mut Arena<HirLocal>,
        local_map: &mut HashMap<usize, LocalId>,
    ) -> Result<(), String> {
        let mut body_stmts = Vec::new();
        for (stmt_or_decl, _) in compound.statements {
            let mut lowered = Vec::new();
            match stmt_or_decl {
                StatementOrDeclaration::Statement((stmt, span)) => {
                    self.lower_stmt(stmt, span, &mut lowered, locals, local_map)?;
                }
                StatementOrDeclaration::Declaration((decl, _)) => {
                    self.lower_decl(decl, &mut lowered, locals, local_map)?;
                }
            }
            for stmt in lowered {
                if matches!(
                    &stmt,
                    HirStmt::Decl(HirDecl {
                        initializer: Some(HirExpr {
                            kind: HirExprKind::Zeroed,
                            ..
                        }),
                        ..
                    })
                ) {
                    self.hoist_zeroed_decl(stmt);
                } else {
                    body_stmts.push(stmt);
                }
            }
        }
        out.extend(body_stmts);
        Ok(())
    }

    pub(crate) fn lower_stmt(
        &self,
        stmt: Statement<LocalResolver>,
        parser_span: co2_ast::Span,
        out: &mut Vec<HirStmt>,
        locals: &mut Arena<HirLocal>,
        local_map: &mut HashMap<usize, LocalId>,
    ) -> Result<(), String> {
        let span = self.to_rust_span(parser_span);
        match stmt {
            Statement::Empty => {}
            Statement::Break => {
                let Some(label) = self.current_break_label() else {
                    return Err("Break statement outside of loop body.".to_owned());
                };
                out.push(HirStmt::Goto(label, span));
            }
            Statement::Continue => {
                let Some(label) = self.current_continue_label() else {
                    return Err("Continue statement outside of loop body.".to_owned());
                };
                out.push(HirStmt::Goto(label, span));
            }
            Statement::Case { expr, statement } => {
                let (discr_local, discr_ty) = self
                    .current_switch_discr()
                    .ok_or_else(|| "case label outside of switch body".to_owned())?;
                let case_label = self.fresh_label();
                let case_expr = self.lower_expr(expr, locals, local_map)?;
                if !is_condition_ty(case_expr.ty) {
                    return Err(format!(
                        "switch case expression must be scalar-like, got {:?}",
                        case_expr.ty
                    ));
                }
                let case_expr_ty = case_expr.ty;
                let case_expr = coerce_expr_to_type(case_expr, discr_ty).map_err(|_| {
                    format!(
                        "switch case expression type mismatch: expected {:?}, got {:?}",
                        discr_ty, case_expr_ty
                    )
                })?;
                let discr_expr = HirExpr {
                    kind: HirExprKind::Local(discr_local),
                    ty: discr_ty,
                    span,
                };
                let cond = HirExpr {
                    kind: HirExprKind::Binary {
                        op: crate::HirBinOp::Eq,
                        lhs: Box::new(discr_expr),
                        rhs: Box::new(case_expr),
                    },
                    ty: Ty::signed_ty(IntTy::I32),
                    span,
                };
                self.register_case(cond, case_label);
                out.push(HirStmt::Label(case_label, span));
                self.lower_stmt(statement.0, statement.1, out, locals, local_map)?;
            }
            Statement::Default { statement } => {
                if !self.in_switch() {
                    return Err("default label outside of switch body".to_owned());
                }
                let label = self.fresh_label();
                self.register_default(label)?;
                out.push(HirStmt::Label(label, span));
                self.lower_stmt(statement.0, statement.1, out, locals, local_map)?;
            }
            Statement::Goto(name) => {
                out.push(HirStmt::Goto(self.resolve_or_insert_label(name.0), span))
            }
            Statement::IndirectGoto(expr) => {
                let expr = self.lower_expr(expr, locals, local_map)?;
                if !is_condition_ty(expr.ty) {
                    return Err(format!(
                        "indirect goto operand must be scalar-like, got {:?}",
                        expr.ty
                    ));
                }
                out.push(HirStmt::IndirectGoto(expr, span));
            }
            Statement::Label { name, statement } => {
                out.push(HirStmt::Label(self.resolve_or_insert_label(name.0), span));
                self.lower_stmt(statement.0, statement.1, out, locals, local_map)?;
            }
            Statement::Return(expr) => {
                if let Some(expr) = expr {
                    let mut expr = self.lower_expr((expr.0, expr.1), locals, local_map)?;
                    self.array_to_pointer_decay_if_array(&mut expr);
                    self.fn_def_to_c_fn_ptr_decay_if_fn_def(&mut expr);
                    if needs_implicit_cast(self.ret_ty, expr.ty) {
                        expr = HirExpr {
                            kind: HirExprKind::Cast(Box::new(expr.clone())),
                            ty: self.ret_ty,
                            span: expr.span,
                        };
                    }
                    out.push(HirStmt::Return(Some(expr), span));
                } else {
                    out.push(HirStmt::Return(None, span));
                }
            }
            Statement::Expression(expr) => {
                let expr = self.lower_expr(expr, locals, local_map)?;
                out.push(HirStmt::Expr(expr));
            }
            Statement::If {
                cond,
                then_branch,
                else_branch,
            } => {
                let cond = self.lower_expr(cond, locals, local_map)?;
                if !is_condition_ty(cond.ty) {
                    return Err(format!(
                        "if condition must be scalar-like, got {:?}",
                        cond.ty
                    ));
                }

                let mut then_map = local_map.clone();
                let mut then_stmts = Vec::new();
                self.lower_stmt(
                    then_branch.0,
                    then_branch.1,
                    &mut then_stmts,
                    locals,
                    &mut then_map,
                )?;

                let mut else_stmts = Vec::new();
                if let Some(else_branch) = else_branch {
                    let mut else_map = local_map.clone();
                    self.lower_stmt(
                        else_branch.0,
                        else_branch.1,
                        &mut else_stmts,
                        locals,
                        &mut else_map,
                    )?;
                }

                out.push(HirStmt::If {
                    cond,
                    then_stmts,
                    else_stmts,
                    span,
                });
            }
            Statement::While { cond, body } => {
                let cond = self.lower_expr(cond, locals, local_map)?;
                if !is_condition_ty(cond.ty) {
                    return Err(format!(
                        "while condition must be scalar-like, got {:?}",
                        cond.ty
                    ));
                }
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
                let body_result = self.lower_stmt(body.0, body.1, out, locals, &mut body_map);
                self.exit_loop();
                body_result?;
                out.push(HirStmt::Goto(cond_label, span));
                out.push(HirStmt::Label(end_label, span));
            }
            Statement::DoWhile { body, cond } => {
                let cond = self.lower_expr(cond, locals, local_map)?;
                if !is_condition_ty(cond.ty) {
                    return Err(format!(
                        "do-while condition must be scalar-like, got {:?}",
                        cond.ty
                    ));
                }
                let body_label = self.fresh_label();
                let cond_label = self.fresh_label();
                let end_label = self.fresh_label();

                out.push(HirStmt::Goto(body_label, span));
                out.push(HirStmt::Label(body_label, span));
                let mut body_map = local_map.clone();
                self.enter_loop(cond_label, end_label);
                let body_result = self.lower_stmt(body.0, body.1, out, locals, &mut body_map);
                self.exit_loop();
                body_result?;
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
                if let Some(init) = init {
                    match init {
                        ForInit::Expression(init) => {
                            let init = self.lower_expr(init, locals, &mut loop_map)?;
                            out.push(HirStmt::Expr(init));
                        }
                        ForInit::Declaration((decl, _)) => {
                            self.lower_decl(decl, out, locals, &mut loop_map)?;
                        }
                    }
                }
                let cond = if let Some(cond) = cond {
                    let cond = self.lower_expr(cond, locals, &mut loop_map)?;
                    if !is_condition_ty(cond.ty) {
                        return Err(format!(
                            "for condition must be scalar-like, got {:?}",
                            cond.ty
                        ));
                    }
                    cond
                } else {
                    HirExpr {
                        kind: HirExprKind::ConstInt(1),
                        ty: Ty::signed_ty(IntTy::I32),
                        span,
                    }
                };

                let cond_label = self.fresh_label();
                let body_label = self.fresh_label();
                let continue_label = self.fresh_label();
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
                body_map.extend(loop_map.clone());
                self.enter_loop(continue_label, end_label);
                let body_result = self.lower_stmt(body.0, body.1, out, locals, &mut body_map);
                self.exit_loop();
                body_result?;
                out.push(HirStmt::Label(continue_label, span));
                if let Some(post) = post {
                    let post = self.lower_expr(post, locals, &mut body_map)?;
                    out.push(HirStmt::Expr(post));
                }
                out.push(HirStmt::Goto(cond_label, span));
                out.push(HirStmt::Label(end_label, span));
            }
            Statement::Switch { expr, body } => {
                let discr = self.lower_expr(expr, locals, local_map)?;
                if !is_condition_ty(discr.ty) {
                    return Err(format!(
                        "switch expression must be scalar-like, got {:?}",
                        discr.ty
                    ));
                }
                let discr_ty = discr.ty;
                let discr_local = locals.alloc(HirLocal {
                    name: format!("__switch_discr{}", locals.len()),
                    ty: discr_ty,
                    span,
                    read_only: false,
                });
                out.push(HirStmt::Decl(HirDecl {
                    local: discr_local,
                    initializer: Some(discr),
                    span,
                }));

                let end_label = self.fresh_label();
                let mut body_stmts = Vec::new();

                self.enter_switch_scope(discr_local, discr_ty, end_label);
                let body_res = self.lower_stmt(body.0, body.1, &mut body_stmts, locals, local_map);
                let scope = self.exit_switch_scope();
                body_res?;

                for (cond, label) in scope.case_labels {
                    out.push(HirStmt::If {
                        cond,
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
                self.lower_compound_items(nested.0, out, locals, local_map)?;
                *local_map = outer_scope;
            }
        }
        Ok(())
    }
}
