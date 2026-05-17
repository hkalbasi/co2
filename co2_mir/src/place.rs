use co2_hir::{HirExpr, HirExprKind, ResolvedValue};
use rustc_public_generative::rustc_public::{
    CrateItem,
    mir::{
        Mutability, Place as MirPlace, ProjectionElem as MirProjection, Rvalue,
        Statement as MirStatement, StatementKind as MirStatementKind,
    },
    ty::Ty,
};

use crate::build::Builder;

pub(crate) fn place(local: usize) -> MirPlace {
    MirPlace {
        local,
        projection: vec![],
    }
}

impl Builder<'_, '_> {
    pub(crate) fn lower_expr_to_place(&mut self, expr: &HirExpr) -> Option<MirPlace> {
        match &expr.kind {
            HirExprKind::Local(local) | HirExprKind::LocalConst(local) => {
                Some(place(self.local_to_index(*local)))
            }
            HirExprKind::Field { base, index } => {
                let mut base_place = self.lower_expr_to_place_or_temp(base);
                base_place
                    .projection
                    .push(MirProjection::Field(*index, expr.ty));
                Some(base_place)
            }
            HirExprKind::Deref(inner) => {
                let mut base_place = self.lower_expr_to_place_or_temp(inner);
                base_place.projection.push(MirProjection::Deref);
                Some(base_place)
            }
            HirExprKind::Path(ResolvedValue::Static(def) | ResolvedValue::StaticConst(def)) => {
                let value_ty = CrateItem(*def).ty();
                let ptr_ty = Ty::new_ptr(value_ty, Mutability::Mut);
                let tmp_ptr = self.new_temp(ptr_ty, Mutability::Mut, expr.span);
                self.stmts.push(MirStatement {
                    kind: MirStatementKind::Assign(
                        place(tmp_ptr),
                        Rvalue::ThreadLocalRef(CrateItem(*def)),
                    ),
                    span: expr.span,
                });
                let mut base_place = place(tmp_ptr);
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

    fn lower_expr_to_place_or_temp(&mut self, inner: &HirExpr) -> MirPlace {
        if let Some(place) = self.lower_expr_to_place(inner) {
            place
        } else {
            let tmp = self.new_temp(inner.ty, Mutability::Mut, inner.span);
            let value = self.lower_expr_to_operand(inner);
            self.stmts.push(MirStatement {
                kind: MirStatementKind::Assign(place(tmp), Rvalue::Use(value)),
                span: inner.span,
            });
            place(tmp)
        }
    }
}
