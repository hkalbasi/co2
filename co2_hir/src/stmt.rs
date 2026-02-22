use std::collections::HashMap;

use co2_parser::{
    CompoundStatement, RustPath, Statement, StatementOrDeclaration, TypeQueryResult,
    TypeResolver as ParserTypeResolver, parse_compound_statement,
};
use la_arena::Arena;
use rustc_public_generative::rustc_public::ty::{IntTy, Span as RustSpan, Ty};

use crate::HirDecl;
use crate::expr::{HirExpr, HirExprKind};
use crate::item::{HirLocal, LocalId};
use crate::resolver::HirCtx;
use crate::ty::is_condition_ty;

#[derive(Clone, Debug)]
pub enum HirStmt {
    Decl(HirDecl),
    Expr(HirExpr),
    Return(Option<HirExpr>, RustSpan),
    If {
        cond: HirExpr,
        then_stmts: Vec<HirStmt>,
        else_stmts: Vec<HirStmt>,
        span: RustSpan,
    },
    While {
        cond: HirExpr,
        body_stmts: Vec<HirStmt>,
        span: RustSpan,
    },
}

impl<R> HirCtx<'_, R> {
    pub(crate) fn lower_compound_items(
        &self,
        compound: CompoundStatement,
        source_name: &str,
        source: &'static str,
        out: &mut Vec<HirStmt>,
        locals: &mut Arena<HirLocal>,
        local_map: &mut HashMap<String, LocalId>,
    ) -> Result<(), String> {
        for (stmt_or_decl, stmt_span) in compound.statements {
            match stmt_or_decl {
                StatementOrDeclaration::Statement((stmt, span)) => {
                    self.lower_stmt(stmt, span, source_name, source, out, locals, local_map)?;
                }
                StatementOrDeclaration::Declaration((decl, _)) => {
                    self.lower_decl(decl, stmt_span, out, locals, local_map)?;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn lower_stmt(
        &self,
        stmt: Statement,
        parser_span: co2_parser::Span,
        source_name: &str,
        source: &'static str,
        out: &mut Vec<HirStmt>,
        locals: &mut Arena<HirLocal>,
        local_map: &mut HashMap<String, LocalId>,
    ) -> Result<(), String> {
        let span = self.to_rust_span(parser_span);
        match stmt {
            Statement::Empty => {}
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
                    source_name,
                    source,
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
                        source_name,
                        source,
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
                let mut body_map = local_map.clone();
                let mut body_stmts = Vec::new();
                self.lower_stmt(
                    body.0,
                    body.1,
                    source_name,
                    source,
                    &mut body_stmts,
                    locals,
                    &mut body_map,
                )?;
                out.push(HirStmt::While {
                    cond,
                    body_stmts,
                    span,
                });
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
                let mut body_map = local_map.clone();
                let mut body_stmts = Vec::new();
                self.lower_stmt(
                    body.0,
                    body.1,
                    source_name,
                    source,
                    &mut body_stmts,
                    locals,
                    &mut body_map,
                )?;
                if let Some(post) = post {
                    let post = self.lower_expr(post, locals, &body_map)?;
                    body_stmts.push(HirStmt::Expr(post));
                }
                out.push(HirStmt::While {
                    cond,
                    body_stmts,
                    span,
                });
            }
            Statement::Compound((lazy, _)) => {
                struct BodyParseResolver<'a, R> {
                    hir_ctx: &'a HirCtx<'a, R>,
                }
                impl<R> ParserTypeResolver for BodyParseResolver<'_, R> {
                    fn classify_path(&self, path: &RustPath) -> TypeQueryResult {
                        let path = path.to_pretty();
                        if self.hir_ctx.resolve_type(&path).is_some() {
                            TypeQueryResult::Type
                        } else {
                            TypeQueryResult::Expr
                        }
                    }
                }
                let parse_resolver = BodyParseResolver { hir_ctx: self };
                let nested = parse_compound_statement(
                    &lazy.tokens.0,
                    source_name.to_owned(),
                    source,
                    &parse_resolver,
                )
                .ok_or_else(|| "failed to parse nested compound statement".to_owned())?;
                let outer_scope = local_map.clone();
                self.lower_compound_items(
                    nested.0,
                    source_name,
                    source,
                    out,
                    locals,
                    local_map,
                )?;
                *local_map = outer_scope;
            }
        }
        Ok(())
    }
}
