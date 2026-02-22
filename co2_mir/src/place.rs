use co2_hir::{HirExpr, HirExprKind};
use rustc_public_generative::rustc_public::{
    mir::{CastKind, Mutability, Place as MirPlace, ProjectionElem as MirProjection, Rvalue, Statement as MirStatement, StatementKind as MirStatementKind},
    ty::{RigidTy, Ty, TyKind},
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
            HirExprKind::Subscript { base, index } => {
                let mut base_place = if let Some(place) = self.lower_expr_to_place(base) {
                    place
                } else {
                    let tmp = self.new_temp(base.ty, Mutability::Mut, base.span);
                    let value = self.lower_expr_to_operand(base);
                    self.stmts.push(MirStatement {
                        kind: MirStatementKind::Assign(place(tmp), Rvalue::Use(value)),
                        span: base.span,
                    });
                    place(tmp)
                };
                if matches!(base.ty.kind(), TyKind::RigidTy(RigidTy::RawPtr(_, _))) {
                    base_place.projection.push(MirProjection::Deref);
                }

                let index_ty = Ty::usize_ty();
                let index_local = self.new_temp(index_ty, Mutability::Mut, index.span);
                let index_operand = self.lower_expr_to_operand(index);
                self.stmts.push(MirStatement {
                    kind: MirStatementKind::Assign(
                        place(index_local),
                        Rvalue::Cast(CastKind::IntToInt, index_operand, index_ty),
                    ),
                    span: index.span,
                });
                base_place.projection.push(MirProjection::Index(index_local));
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
            _ => None,
        }
    }
}
