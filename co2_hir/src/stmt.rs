use std::collections::HashMap;

use co2_parser::{CompoundStatement, Statement, StatementOrDeclaration};
use la_arena::Arena;
use rustc_public_generative::rustc_public::ty::{IntTy, Span as RustSpan, Ty};

use crate::HirDecl;
use crate::expr::{HirExpr, HirExprKind};
use crate::item::{HirLocal, LabelId, LocalId};
use crate::resolver::HirCtx;
use crate::ty::is_condition_ty;

#[derive(Clone, Debug)]
pub enum HirStmt {
    Decl(HirDecl),
    Expr(HirExpr),
    Label(LabelId, RustSpan),
    Goto(LabelId, RustSpan),
    Return(Option<HirExpr>, RustSpan),
    If {
        cond: HirExpr,
        then_stmts: Vec<HirStmt>,
        else_stmts: Vec<HirStmt>,
        span: RustSpan,
    },
}

impl<R> HirCtx<'_, R> {
    fn lower_switch_stmt(
        &self,
        stmt: Statement,
        parser_span: co2_parser::Span,
        out: &mut Vec<HirStmt>,
        locals: &mut Arena<HirLocal>,
        local_map: &mut HashMap<String, LocalId>,
        discr_local: LocalId,
        discr_ty: Ty,
        case_labels: &mut Vec<(HirExpr, LabelId)>,
        default_label: &mut Option<LabelId>,
    ) -> Result<(), String> {
        let span = self.to_rust_span(parser_span);
        match stmt {
            Statement::Case { expr, statement } => {
                let case_label = self.fresh_label();
                let case_expr = self.lower_expr(expr, locals, local_map)?;
                if !is_condition_ty(case_expr.ty) {
                    return Err(format!(
                        "switch case expression must be scalar-like, got {:?}",
                        case_expr.ty
                    ));
                }
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
                case_labels.push((cond, case_label));
                out.push(HirStmt::Label(case_label, span));
                self.lower_switch_stmt(
                    statement.0,
                    statement.1,
                    out,
                    locals,
                    local_map,
                    discr_local,
                    discr_ty,
                    case_labels,
                    default_label,
                )
            }
            Statement::Default { statement } => {
                if default_label.is_some() {
                    return Err("duplicate `default` label in switch".to_owned());
                }
                let label = self.fresh_label();
                *default_label = Some(label);
                out.push(HirStmt::Label(label, span));
                self.lower_switch_stmt(
                    statement.0,
                    statement.1,
                    out,
                    locals,
                    local_map,
                    discr_local,
                    discr_ty,
                    case_labels,
                    default_label,
                )
            }
            Statement::Compound(nested) => {
                let outer_scope = local_map.clone();
                for (stmt_or_decl, _) in nested.0.statements {
                    match stmt_or_decl {
                        StatementOrDeclaration::Statement((stmt, span)) => {
                            self.lower_switch_stmt(
                                stmt,
                                span,
                                out,
                                locals,
                                local_map,
                                discr_local,
                                discr_ty,
                                case_labels,
                                default_label,
                            )?;
                        }
                        StatementOrDeclaration::Declaration((decl, _)) => {
                            self.lower_decl(decl, out, locals, local_map)?;
                        }
                    }
                }
                *local_map = outer_scope;
                Ok(())
            }
            _ => self.lower_stmt(stmt, parser_span, out, locals, local_map),
        }
    }

    pub(crate) fn lower_compound_items(
        &self,
        compound: CompoundStatement,
        out: &mut Vec<HirStmt>,
        locals: &mut Arena<HirLocal>,
        local_map: &mut HashMap<String, LocalId>,
    ) -> Result<(), String> {
        for (stmt_or_decl, _) in compound.statements {
            match stmt_or_decl {
                StatementOrDeclaration::Statement((stmt, span)) => {
                    self.lower_stmt(stmt, span, out, locals, local_map)?;
                }
                StatementOrDeclaration::Declaration((decl, _)) => {
                    self.lower_decl(decl, out, locals, local_map)?;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn lower_stmt(
        &self,
        stmt: Statement,
        parser_span: co2_parser::Span,
        out: &mut Vec<HirStmt>,
        locals: &mut Arena<HirLocal>,
        local_map: &mut HashMap<String, LocalId>,
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
            Statement::Case { .. } => return Err("case label outside of switch body".to_owned()),
            Statement::Default { .. } => {
                return Err("default label outside of switch body".to_owned());
            }
            Statement::Goto(name) => out.push(HirStmt::Goto(self.resolve_or_insert_label(name.0), span)),
            Statement::Label { name, statement } => {
                out.push(HirStmt::Label(self.resolve_or_insert_label(name.0), span));
                self.lower_stmt(statement.0, statement.1, out, locals, local_map)?;
            }
            Statement::Return(expr) => {
                if let Some(expr) = expr {
                    let expr = self.lower_expr((expr.0, expr.1), locals, local_map)?;
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
                    return Err(format!("if condition must be scalar-like, got {:?}", cond.ty));
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
                    return Err(format!("while condition must be scalar-like, got {:?}", cond.ty));
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
                if let Some(init) = init {
                    let init = self.lower_expr(init, locals, local_map)?;
                    out.push(HirStmt::Expr(init));
                }
                let cond = if let Some(cond) = cond {
                    let cond = self.lower_expr(cond, locals, local_map)?;
                    if !is_condition_ty(cond.ty) {
                        return Err(format!("for condition must be scalar-like, got {:?}", cond.ty));
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
                    return Err(format!("switch expression must be scalar-like, got {:?}", discr.ty));
                }
                let discr_local = locals.alloc(HirLocal {
                    name: format!("__switch_discr{}", locals.len()),
                    ty: discr.ty,
                    span,
                });
                out.push(HirStmt::Decl(HirDecl {
                    local: discr_local,
                    initializer: Some(discr.clone()),
                    span,
                }));

                let end_label = self.fresh_label();
                let mut case_labels = Vec::new();
                let mut default_label = None;
                let mut body_stmts = Vec::new();

                self.enter_switch(end_label);
                let switch_res = self.lower_switch_stmt(
                    body.0,
                    body.1,
                    &mut body_stmts,
                    locals,
                    local_map,
                    discr_local,
                    discr.ty,
                    &mut case_labels,
                    &mut default_label,
                );
                self.exit_switch();
                switch_res?;

                for (cond, label) in case_labels {
                    out.push(HirStmt::If {
                        cond,
                        then_stmts: vec![HirStmt::Goto(label, span)],
                        else_stmts: vec![],
                        span,
                    });
                }
                out.push(HirStmt::Goto(default_label.unwrap_or(end_label), span));
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
