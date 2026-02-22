use co2_hir::{HirExpr, HirExprKind};
use rustc_public_generative::rustc_public::{
    mir::{Mutability, Place as MirPlace, ProjectionElem as MirProjection, Rvalue, Statement as MirStatement, StatementKind as MirStatementKind},
};

use crate::{build::Builder};

pub(crate) fn place(local: usize) -> MirPlace {
    MirPlace {
        local,
        projection: vec![],
    }
}

impl Builder<'_> {
    pub(crate) fn lower_expr_to_place(&mut self, expr: &HirExpr) -> Option<MirPlace> {
        match &expr.kind {
            HirExprKind::Local(local) => Some(place(self.local_to_index(*local))),
            HirExprKind::Field { base, index } => {
                let mut base_place = self.lower_expr_to_place(base)?;
                base_place
                    .projection
                    .push(MirProjection::Field(*index, expr.ty));
                Some(base_place)
            }
            HirExprKind::Deref(inner) => {
                let mut base_place = if let Some(place) = self.lower_expr_to_place(inner) {
                    place
                } else {
                    let tmp = self.new_temp(inner.ty, Mutability::Mut, inner.span);
                    let value = self.lower_expr_to_operand(inner);
                    self.stmts.push(MirStatement {
                        kind: MirStatementKind::Assign(place(tmp), Rvalue::Use(value)),
                        span: inner.span,
                    });
                    place(tmp)
                };
                base_place.projection.push(MirProjection::Deref);
                Some(base_place)
            }
            HirExprKind::StatementExpr { statements, tail } => {
                for stmt in statements {
                    self.lower_stmt(stmt);
                }
                self.lower_expr_to_place(tail)
            }
            _ => None,
        }
    }
}
