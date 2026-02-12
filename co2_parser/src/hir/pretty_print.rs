use itertools::Itertools;

use crate::hir::{Block, Expr, HirBody, HirCtxInterface, Stmt};

use super::{ExprKind, StmtKind};

trait HirPrint<C: HirCtxInterface> {
    fn print(&self, ctx: &HirBody<C>) -> String;
}

fn indent(x: String) -> String {
    x.lines().map(|l| format!("    {l}")).join("\n")
}

impl<C: HirCtxInterface> HirPrint<C> for Expr<C> {
    fn print(&self, ctx: &HirBody<C>) -> String {
        match &self.kind {
            ExprKind::Lit(lit) => todo!(),
            ExprKind::Local(_) => todo!(),
            ExprKind::Call(expr, exprs) => todo!(),
            ExprKind::Binary(bin_op, expr, expr1) => todo!(),
            ExprKind::Unary(un_op, expr) => todo!(),
            ExprKind::Assign(expr, expr1) => todo!(),
            ExprKind::AssignWithBinOp(expr, expr1, bin_op, _, return_semantic) => todo!(),
            ExprKind::Field(expr, _) => todo!(),
            ExprKind::PtrOffset(expr, expr1) => todo!(),
            ExprKind::PtrDiff(expr, expr1) => todo!(),
            ExprKind::AssignPtrOffset(expr, expr1, return_semantic) => todo!(),
            ExprKind::Cast(expr) => todo!(),
            ExprKind::InitializerList(initializer_tree) => todo!(),
            ExprKind::Comma(exprs) => todo!(),
            ExprKind::OffsetOf => todo!(),
            ExprKind::Cond(expr, expr1, expr2) => todo!(),
            ExprKind::GnuBlock(block) => todo!(),
            ExprKind::Empty => todo!(),
        }
    }
}

impl<C: HirCtxInterface> HirPrint<C> for Stmt<C> {
    fn print(&self, ctx: &HirBody<C>) -> String {
        match &self.kind {
            StmtKind::Block(block) => block.print(ctx),
            StmtKind::Expr(expr) => format!("{};", expr.print(ctx)),
            StmtKind::Decl(items) => todo!(),
            StmtKind::Ret(expr) => todo!(),
            StmtKind::Label(idx, stmt) => todo!(),
            StmtKind::Goto(idx) => todo!(),
            StmtKind::If(expr, stmt, stmt1) => todo!(),
            StmtKind::Noop => todo!(),
        }
    }
}

impl<C: HirCtxInterface> HirPrint<C> for Block<C> {
    fn print(&self, ctx: &HirBody<C>) -> String {
        let inner = self.stmts.iter().map(|stmt| stmt.print(ctx)).join("\n");
        format!("{{\n{}\n}}", indent(inner))
    }
}

impl<C: HirCtxInterface> HirBody<C> {
    pub fn pretty_print(&self) -> String {
        self.root.print(self)
    }
}
