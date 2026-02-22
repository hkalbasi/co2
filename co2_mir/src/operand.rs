use co2_hir::{HirExpr, HirExprKind, ResolvedValue};
use rustc_public_generative::rustc_public::{
    mir::{
        AggregateKind, BorrowKind, CastKind, ConstOperand, MutBorrowKind, Mutability,
        Operand as MirOperand, Rvalue, Statement as MirStatement, StatementKind as MirStatementKind,
    },
    ty::{GenericArgs, MirConst, RigidTy, Span as RustSpan, Ty, TyKind},
};

use crate::{
    build::{Builder, fn_const_operand, infer_fn_generic_args, ty_matches_expected, variant_idx},
    place::place,
};

impl Builder<'_> {
    pub(crate) fn lower_expr_to_operand(&mut self, expr: &HirExpr) -> MirOperand {
        match &expr.kind {
            HirExprKind::Local(local) => {
                let local_index = self.local_to_index(*local);
                match self.locals[local_index].ty.kind() {
                    TyKind::RigidTy(RigidTy::Adt(_, _)) => MirOperand::Move(place(local_index)),
                    _ => MirOperand::Copy(place(local_index)),
                }
            }
            HirExprKind::ConstInt(v) => {
                let span = expr.span;
                let (uint_ty, bits) = crate::rvalue::int_literal_bits(*v, expr.ty);
                let c = MirConst::try_from_uint(bits, uint_ty).expect("failed to build int const");
                let const_op = MirOperand::Constant(ConstOperand {
                    span,
                    user_ty: None,
                    const_: c,
                });

                if matches!(expr.ty.kind(), TyKind::RigidTy(RigidTy::Uint(_))) {
                    return const_op;
                }

                let tmp = self.new_temp(expr.ty, Mutability::Mut, span);
                self.stmts.push(MirStatement {
                    kind: MirStatementKind::Assign(
                        place(tmp),
                        Rvalue::Cast(CastKind::IntToInt, const_op, expr.ty),
                    ),
                    span,
                });
                MirOperand::Copy(place(tmp))
            }
            HirExprKind::Field { .. } => {
                let place = self
                    .lower_expr_to_place(expr)
                    .expect("field expression should be place-expressible");
                MirOperand::Move(place)
            }
            HirExprKind::Subscript { .. } => {
                let place = self
                    .lower_expr_to_place(expr)
                    .expect("subscript expression should be place-expressible");
                MirOperand::Copy(place)
            }
            HirExprKind::Binary { op, lhs, rhs } => {
                let lhs = self.lower_expr_to_operand(lhs);
                let rhs = self.lower_expr_to_operand(rhs);
                let tmp = self.new_temp(expr.ty, Mutability::Mut, expr.span);
                self.stmts.push(MirStatement {
                    kind: MirStatementKind::Assign(
                        place(tmp),
                        Rvalue::BinaryOp(self.lower_bin_op(*op), lhs, rhs),
                    ),
                    span: expr.span,
                });
                MirOperand::Move(place(tmp))
            }
            HirExprKind::Aggregate { args } => {
                let TyKind::RigidTy(RigidTy::Adt(adt, adt_args)) = expr.ty.kind() else {
                    panic!("aggregate initializer expects adt type, got {:?}", expr.ty);
                };
                let mut operands = Vec::with_capacity(args.len());
                for arg in args {
                    operands.push(self.lower_expr_to_operand(arg));
                }
                let tmp = self.new_temp(expr.ty, Mutability::Mut, expr.span);
                self.stmts.push(MirStatement {
                    kind: MirStatementKind::Assign(
                        place(tmp),
                        Rvalue::Aggregate(
                            AggregateKind::Adt(adt, variant_idx(0), adt_args, None, None),
                            operands,
                        ),
                    ),
                    span: expr.span,
                });
                MirOperand::Copy(place(tmp))
            }
            HirExprKind::ConstStr(s) => self.lower_const_string(s, expr.span),
            HirExprKind::Path(path) => {
                let ResolvedValue::Fn(fn_def) = path;
                let fn_ty = Ty::from_rigid_kind(RigidTy::FnDef(*fn_def, GenericArgs(vec![])));
                let c = MirConst::try_new_zero_sized(fn_ty).expect("failed to build fn const");
                MirOperand::Constant(ConstOperand {
                    span: expr.span,
                    user_ty: None,
                    const_: c,
                })
            }
            HirExprKind::Call { func, args } => self.lower_call_expr(func, args, expr.span, expr.ty),
            HirExprKind::Assign { lhs, rhs } => {
                let lhs_place = self
                    .lower_expr_to_place(lhs)
                    .expect("assignment lhs should be place-expressible");
                let rhs_value = self.lower_expr_to_operand(rhs);
                self.stmts.push(MirStatement {
                    kind: MirStatementKind::Assign(lhs_place.clone(), Rvalue::Use(rhs_value)),
                    span: expr.span,
                });
                MirOperand::Copy(lhs_place)
            }
            HirExprKind::AddrOf(inner) => {
                let target_place = self
                    .lower_expr_to_place(inner)
                    .expect("address-of target should be place-expressible");
                let tmp = self.new_temp(expr.ty, Mutability::Mut, expr.span);
                self.stmts.push(MirStatement {
                    kind: MirStatementKind::Assign(
                        place(tmp),
                        Rvalue::AddressOf(rustc_public_generative::rustc_public::mir::RawPtrKind::Mut, target_place),
                    ),
                    span: expr.span,
                });
                MirOperand::Copy(place(tmp))
            }
            HirExprKind::Deref(_) => {
                let place = self
                    .lower_expr_to_place(expr)
                    .expect("deref expression should be place-expressible");
                MirOperand::Copy(place)
            }
        }
    }

    pub(crate) fn lower_call_expr(
        &mut self,
        func: &HirExpr,
        args: &[HirExpr],
        span: RustSpan,
        ret_ty: Ty,
    ) -> MirOperand {
        let ret_local = self.new_temp(ret_ty, Mutability::Mut, span);
        self.lower_call_to_destination(func, args, span, place(ret_local), ret_ty);
        MirOperand::Copy(place(ret_local))
    }

    pub(crate) fn lower_call_to_destination(
        &mut self,
        func: &HirExpr,
        args: &[HirExpr],
        span: RustSpan,
        destination: rustc_public_generative::rustc_public::mir::Place,
        ret_ty: Ty,
    ) {
        let fn_def = match &func.kind {
            HirExprKind::Path(ResolvedValue::Fn(fn_def)) => *fn_def,
            _ => panic!("unsupported call target: {:?}", func.kind),
        };
        let sig = func
            .ty
            .kind()
            .fn_sig()
            .expect("call target has no fn signature")
            .skip_binder();

        let mut arg_ops = Vec::with_capacity(args.len());
        for (idx, arg) in args.iter().enumerate() {
            let expected_ty = sig.inputs()[idx];
            let op = self.lower_call_arg(arg, expected_ty);
            arg_ops.push(op);
        }
        let generic_args = infer_fn_generic_args(&sig, args);

        let _ = ret_ty;
        self.emit_call_block(
            fn_const_operand(fn_def, generic_args, span),
            arg_ops,
            destination,
            span,
        );
    }

    pub(crate) fn lower_call_arg(&mut self, arg: &HirExpr, expected_ty: Ty) -> MirOperand {
        let TyKind::RigidTy(RigidTy::Ref(region, inner, mutability)) = expected_ty.kind() else {
            return self.lower_expr_to_operand(arg);
        };

        if !ty_matches_expected(inner, arg.ty) {
            return self.lower_expr_to_operand(arg);
        }

        let borrowed_place = if let Some(place) = self.lower_expr_to_place(arg) {
            place
        } else {
            let tmp = self.new_temp(arg.ty, Mutability::Mut, arg.span);
            let value = self.lower_expr_to_operand(arg);
            self.stmts.push(MirStatement {
                kind: MirStatementKind::Assign(place(tmp), Rvalue::Use(value)),
                span: arg.span,
            });
            place(tmp)
        };

        let borrow_kind = if mutability == Mutability::Mut {
            BorrowKind::Mut {
                kind: MutBorrowKind::Default,
            }
        } else {
            BorrowKind::Shared
        };
        let concrete_ref_ty = Ty::new_ref(region.clone(), arg.ty, mutability);
        let ref_local = self.new_temp(concrete_ref_ty, Mutability::Not, arg.span);
        self.stmts.push(MirStatement {
            kind: MirStatementKind::Assign(
                place(ref_local),
                Rvalue::Ref(region.clone(), borrow_kind, borrowed_place),
            ),
            span: arg.span,
        });
        MirOperand::Move(place(ref_local))
    }
}
