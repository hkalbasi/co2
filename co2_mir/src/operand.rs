use co2_hir::{HirExpr, HirExprKind, HirLogicalOp, ResolvedValue, ReturnSemantic};
use rustc_public_generative::rustc_public::{
    CrateDefType,
    mir::{
        AggregateKind, BorrowKind, CastKind, ConstOperand, MutBorrowKind, Mutability,
        Operand as MirOperand, PointerCoercion, ProjectionElem as MirProjection, RawPtrKind,
        Rvalue, Safety, Statement as MirStatement, StatementKind as MirStatementKind,
        SwitchTargets, TerminatorKind,
    },
    ty::{GenericArgKind, GenericArgs, IntTy, MirConst, RigidTy, Span as RustSpan, Ty, TyKind},
};

use crate::{
    build::{Builder, fn_const_operand, infer_fn_generic_args, ty_matches_expected, variant_idx},
    place::place,
};

fn find_ptr_offset_from_fn(
    deps: &rustc_public_generative::DependencyInfo,
) -> Option<rustc_public_generative::rustc_public::ty::FnDef> {
    let exact = [
        "core::ptr::mut_ptr::offset_from",
        "core::ptr::const_ptr::offset_from",
        "std::ptr::mut_ptr::offset_from",
        "std::ptr::const_ptr::offset_from",
    ];
    for wanted in exact {
        if let Some(found) = deps
            .functions
            .iter()
            .find(|f| f.fn_def.is_some() && f.path.contains(wanted))
            .and_then(|f| f.fn_def)
        {
            return Some(found);
        }
    }

    deps.functions
        .iter()
        .find(|f| {
            f.fn_def.is_some()
                && f.path.ends_with("::offset_from")
                && (f.path.contains("::ptr::mut_ptr::") || f.path.contains("::ptr::const_ptr::"))
        })
        .and_then(|f| f.fn_def)
}

fn maybe_uninit_fn_ptr_inner(ty: Ty) -> Option<Ty> {
    let TyKind::RigidTy(RigidTy::Adt(_, args)) = ty.kind() else {
        return None;
    };
    if args.0.len() != 1 {
        return None;
    }
    let GenericArgKind::Type(inner) = args.0[0] else {
        return None;
    };
    if matches!(inner.kind(), TyKind::RigidTy(RigidTy::FnPtr(_))) {
        Some(inner)
    } else {
        None
    }
}

fn callable_sig(
    ty: Ty,
) -> Option<
    rustc_public_generative::rustc_public::ty::Binder<
        rustc_public_generative::rustc_public::ty::FnSig,
    >,
> {
    ty.kind()
        .fn_sig()
        .or_else(|| maybe_uninit_fn_ptr_inner(ty).and_then(|inner| inner.kind().fn_sig()))
}

impl Builder<'_> {
    fn write_value_into_maybe_uninit_storage(
        &mut self,
        dst_maybe_ty: Ty,
        value_op: MirOperand,
        value_ty: Ty,
        span: RustSpan,
    ) -> MirOperand {
        let dst_local = self.new_temp(dst_maybe_ty, Mutability::Mut, span);
        let ptr_maybe_ty = Ty::new_ptr(dst_maybe_ty, Mutability::Mut);
        let ptr_maybe_local = self.new_temp(ptr_maybe_ty, Mutability::Mut, span);
        self.stmts.push(MirStatement {
            kind: MirStatementKind::Assign(
                place(ptr_maybe_local),
                Rvalue::AddressOf(RawPtrKind::Mut, place(dst_local)),
            ),
            span,
        });
        let ptr_value_ty = Ty::new_ptr(value_ty, Mutability::Mut);
        let ptr_value_local = self.new_temp(ptr_value_ty, Mutability::Mut, span);
        self.stmts.push(MirStatement {
            kind: MirStatementKind::Assign(
                place(ptr_value_local),
                Rvalue::Cast(
                    CastKind::PtrToPtr,
                    MirOperand::Copy(place(ptr_maybe_local)),
                    ptr_value_ty,
                ),
            ),
            span,
        });
        let value_place = rustc_public_generative::rustc_public::mir::Place {
            local: ptr_value_local,
            projection: vec![MirProjection::Deref],
        };
        self.stmts.push(MirStatement {
            kind: MirStatementKind::Assign(value_place, Rvalue::Use(value_op)),
            span,
        });
        MirOperand::Copy(place(dst_local))
    }

    fn read_maybe_uninit_as(&mut self, src: &HirExpr, value_ty: Ty, span: RustSpan) -> MirOperand {
        let src_place = if let Some(place) = self.lower_expr_to_place(src) {
            place
        } else {
            let tmp = self.new_temp(src.ty, Mutability::Mut, span);
            let op = self.lower_expr_to_operand(src);
            self.stmts.push(MirStatement {
                kind: MirStatementKind::Assign(place(tmp), Rvalue::Use(op)),
                span,
            });
            place(tmp)
        };
        let ptr_maybe_ty = Ty::new_ptr(src.ty, Mutability::Mut);
        let ptr_maybe_local = self.new_temp(ptr_maybe_ty, Mutability::Mut, span);
        self.stmts.push(MirStatement {
            kind: MirStatementKind::Assign(
                place(ptr_maybe_local),
                Rvalue::AddressOf(RawPtrKind::Mut, src_place),
            ),
            span,
        });
        let ptr_value_ty = Ty::new_ptr(value_ty, Mutability::Mut);
        let ptr_value_local = self.new_temp(ptr_value_ty, Mutability::Mut, span);
        self.stmts.push(MirStatement {
            kind: MirStatementKind::Assign(
                place(ptr_value_local),
                Rvalue::Cast(
                    CastKind::PtrToPtr,
                    MirOperand::Copy(place(ptr_maybe_local)),
                    ptr_value_ty,
                ),
            ),
            span,
        });
        let out_local = self.new_temp(value_ty, Mutability::Mut, span);
        let value_place = rustc_public_generative::rustc_public::mir::Place {
            local: ptr_value_local,
            projection: vec![MirProjection::Deref],
        };
        self.stmts.push(MirStatement {
            kind: MirStatementKind::Assign(
                place(out_local),
                Rvalue::Use(MirOperand::Copy(value_place)),
            ),
            span,
        });
        MirOperand::Copy(place(out_local))
    }

    pub(crate) fn lower_expr_to_operand(&mut self, expr: &HirExpr) -> MirOperand {
        match &expr.kind {
            HirExprKind::ArrayToPointer(inner) => {
                let base_place = self
                    .lower_expr_to_place(inner)
                    .expect("array decay expects place-expressible operand");
                let rustc_public_generative::rustc_public::ty::TyKind::RigidTy(
                    rustc_public_generative::rustc_public::ty::RigidTy::Array(_, _),
                ) = inner.ty.kind()
                else {
                    panic!("array decay expects array type, got {:?}", inner.ty);
                };
                let array_ptr_ty = Ty::new_ptr(inner.ty, Mutability::Mut);
                let array_ptr_local = self.new_temp(array_ptr_ty, Mutability::Mut, expr.span);
                self.stmts.push(MirStatement {
                    kind: MirStatementKind::Assign(
                        place(array_ptr_local),
                        Rvalue::AddressOf(RawPtrKind::Mut, base_place),
                    ),
                    span: expr.span,
                });
                let ptr_local = self.new_temp(expr.ty, Mutability::Mut, expr.span);
                self.stmts.push(MirStatement {
                    kind: MirStatementKind::Assign(
                        place(ptr_local),
                        Rvalue::Cast(
                            CastKind::PtrToPtr,
                            MirOperand::Copy(place(array_ptr_local)),
                            expr.ty,
                        ),
                    ),
                    span: expr.span,
                });
                MirOperand::Copy(place(ptr_local))
            }
            HirExprKind::Zeroed => {
                let temp = self.new_temp(expr.ty, Mutability::Mut, expr.span);
                self.lower_zeroed_to_destination(place(temp), expr.span, expr.ty);
                MirOperand::Copy(place(temp))
            }
            HirExprKind::Local(local) => {
                let local_index = self.local_to_index(*local);
                MirOperand::Copy(place(local_index))
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
                MirOperand::Copy(place)
            }
            HirExprKind::PtrOffset { base, index } => {
                let base_op = self.lower_expr_to_operand(base);
                let isize_ty = Ty::signed_ty(IntTy::Isize);
                let idx_local = self.new_temp(isize_ty, Mutability::Mut, expr.span);
                let idx_op = self.lower_expr_to_operand(index);
                self.stmts.push(MirStatement {
                    kind: MirStatementKind::Assign(
                        place(idx_local),
                        Rvalue::Cast(CastKind::IntToInt, idx_op, isize_ty),
                    ),
                    span: expr.span,
                });
                let ret_local = self.new_temp(expr.ty, Mutability::Mut, expr.span);
                self.stmts.push(MirStatement {
                    kind: MirStatementKind::Assign(
                        place(ret_local),
                        Rvalue::BinaryOp(
                            rustc_public_generative::rustc_public::mir::BinOp::Offset,
                            base_op,
                            MirOperand::Copy(place(idx_local)),
                        ),
                    ),
                    span: expr.span,
                });
                MirOperand::Copy(place(ret_local))
            }
            HirExprKind::PtrDiff { lhs, rhs } => {
                let lhs_op = self.lower_expr_to_operand(lhs);
                let rhs_op = self.lower_expr_to_operand(rhs);
                let TyKind::RigidTy(RigidTy::RawPtr(pointee_ty, _)) = lhs.ty.kind() else {
                    panic!("ptr diff lhs must be raw pointer, got {:?}", lhs.ty);
                };
                let isize_ty = Ty::signed_ty(IntTy::Isize);
                let ret_local = self.new_temp(isize_ty, Mutability::Mut, expr.span);
                let offset_from = find_ptr_offset_from_fn(self.deps)
                    .expect("missing pointer offset_from dependency function");
                let const_ptr_ty = Ty::new_ptr(pointee_ty, Mutability::Not);
                let lhs_cast = self.new_temp(const_ptr_ty, Mutability::Mut, expr.span);
                self.stmts.push(MirStatement {
                    kind: MirStatementKind::Assign(
                        place(lhs_cast),
                        Rvalue::Cast(CastKind::PtrToPtr, lhs_op, const_ptr_ty),
                    ),
                    span: expr.span,
                });
                let rhs_cast = self.new_temp(const_ptr_ty, Mutability::Mut, expr.span);
                self.stmts.push(MirStatement {
                    kind: MirStatementKind::Assign(
                        place(rhs_cast),
                        Rvalue::Cast(CastKind::PtrToPtr, rhs_op, const_ptr_ty),
                    ),
                    span: expr.span,
                });
                let generic_args = match offset_from.ty().kind() {
                    TyKind::RigidTy(RigidTy::FnDef(_, existing)) if !existing.0.is_empty() => {
                        existing
                            .0
                            .iter()
                            .map(|arg| match arg {
                                GenericArgKind::Type(ty)
                                    if matches!(ty.kind(), TyKind::Param(_)) =>
                                {
                                    GenericArgKind::Type(pointee_ty)
                                }
                                _ => arg.clone(),
                            })
                            .collect()
                    }
                    _ => vec![GenericArgKind::Type(pointee_ty)],
                };
                self.emit_call_block(
                    fn_const_operand(offset_from, generic_args, expr.span),
                    vec![
                        MirOperand::Copy(place(lhs_cast)),
                        MirOperand::Copy(place(rhs_cast)),
                    ],
                    place(ret_local),
                    expr.span,
                );
                MirOperand::Copy(place(ret_local))
            }
            HirExprKind::Binary { op, lhs, rhs } => {
                let lhs = self.lower_expr_to_operand(lhs);
                let rhs = self.lower_expr_to_operand(rhs);
                if matches!(
                    op,
                    co2_hir::HirBinOp::Eq
                        | co2_hir::HirBinOp::Lt
                        | co2_hir::HirBinOp::Le
                        | co2_hir::HirBinOp::Ne
                        | co2_hir::HirBinOp::Ge
                        | co2_hir::HirBinOp::Gt
                ) {
                    let bool_local = self.new_temp(Ty::bool_ty(), Mutability::Mut, expr.span);
                    self.stmts.push(MirStatement {
                        kind: MirStatementKind::Assign(
                            place(bool_local),
                            Rvalue::BinaryOp(self.lower_bin_op(*op), lhs, rhs),
                        ),
                        span: expr.span,
                    });

                    let tmp = self.new_temp(expr.ty, Mutability::Mut, expr.span);
                    self.stmts.push(MirStatement {
                        kind: MirStatementKind::Assign(
                            place(tmp),
                            Rvalue::Cast(
                                CastKind::IntToInt,
                                MirOperand::Copy(place(bool_local)),
                                expr.ty,
                            ),
                        ),
                        span: expr.span,
                    });
                    return MirOperand::Copy(place(tmp));
                }
                let tmp = self.new_temp(expr.ty, Mutability::Mut, expr.span);
                self.stmts.push(MirStatement {
                    kind: MirStatementKind::Assign(
                        place(tmp),
                        Rvalue::BinaryOp(self.lower_bin_op(*op), lhs, rhs),
                    ),
                    span: expr.span,
                });
                MirOperand::Copy(place(tmp))
            }
            HirExprKind::Logical { op, lhs, rhs } => {
                self.lower_logical_expr(*op, lhs, rhs, expr.span, expr.ty)
            }
            HirExprKind::Conditional {
                cond,
                then_expr,
                else_expr,
            } => self.lower_conditional_expr(cond, then_expr, else_expr, expr.span, expr.ty),
            HirExprKind::StatementExpr { statements, tail } => {
                for stmt in statements {
                    self.lower_stmt(stmt);
                }
                self.lower_expr_to_operand(tail)
            }
            HirExprKind::LogicalNot(inner) => {
                self.lower_logical_not_expr(inner, expr.span, expr.ty)
            }
            HirExprKind::BitNot(inner) => {
                let inner_op = self.lower_expr_to_operand(inner);
                let minus_one = self.lower_expr_to_operand(&HirExpr {
                    kind: HirExprKind::ConstInt(-1),
                    ty: expr.ty,
                    span: expr.span,
                });
                let tmp = self.new_temp(expr.ty, Mutability::Mut, expr.span);
                self.stmts.push(MirStatement {
                    kind: MirStatementKind::Assign(
                        place(tmp),
                        Rvalue::BinaryOp(
                            rustc_public_generative::rustc_public::mir::BinOp::BitXor,
                            inner_op,
                            minus_one,
                        ),
                    ),
                    span: expr.span,
                });
                MirOperand::Copy(place(tmp))
            }
            HirExprKind::Aggregate { args } => match expr.ty.kind() {
                TyKind::RigidTy(RigidTy::Adt(adt, adt_args)) => {
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
                TyKind::RigidTy(RigidTy::Array(elem, _)) => {
                    let mut operands = Vec::with_capacity(args.len());
                    for arg in args {
                        operands.push(self.lower_expr_to_operand(arg));
                    }
                    let tmp = self.new_temp(expr.ty, Mutability::Mut, expr.span);
                    self.stmts.push(MirStatement {
                        kind: MirStatementKind::Assign(
                            place(tmp),
                            Rvalue::Aggregate(AggregateKind::Array(elem), operands),
                        ),
                        span: expr.span,
                    });
                    MirOperand::Copy(place(tmp))
                }
                _ => {
                    panic!("aggregate initializer expects adt type, got {:?}", expr.ty);
                }
            },
            HirExprKind::ConstStr(s) => self.lower_const_string(s, expr.span),
            HirExprKind::Path(path) => match path {
                ResolvedValue::Fn(fn_def) => {
                    let fn_ty = Ty::from_rigid_kind(RigidTy::FnDef(*fn_def, GenericArgs(vec![])));
                    let c = MirConst::try_new_zero_sized(fn_ty).expect("failed to build fn const");
                    MirOperand::Constant(ConstOperand {
                        span: expr.span,
                        user_ty: None,
                        const_: c,
                    })
                }
                ResolvedValue::ConstInt(v) => {
                    let (uint_ty, bits) = crate::rvalue::int_literal_bits(*v, expr.ty);
                    let c =
                        MirConst::try_from_uint(bits, uint_ty).expect("failed to build enum const");
                    let const_op = MirOperand::Constant(ConstOperand {
                        span: expr.span,
                        user_ty: None,
                        const_: c,
                    });
                    if matches!(expr.ty.kind(), TyKind::RigidTy(RigidTy::Uint(_))) {
                        const_op
                    } else {
                        let tmp = self.new_temp(expr.ty, Mutability::Mut, expr.span);
                        self.stmts.push(MirStatement {
                            kind: MirStatementKind::Assign(
                                place(tmp),
                                Rvalue::Cast(CastKind::IntToInt, const_op, expr.ty),
                            ),
                            span: expr.span,
                        });
                        MirOperand::Copy(place(tmp))
                    }
                }
                ResolvedValue::Static { .. } => {
                    let place = self
                        .lower_expr_to_place(expr)
                        .expect("static path should be place-expressible");
                    MirOperand::Copy(place)
                }
            },
            HirExprKind::Call { func, args } => {
                self.lower_call_expr(func, args, expr.span, expr.ty)
            }
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
            HirExprKind::AssignWithBinOp {
                lhs,
                rhs,
                op,
                return_semantic,
            } => {
                let lhs_place = self
                    .lower_expr_to_place(lhs)
                    .expect("assignment lhs should be place-expressible");
                let rhs_value = self.lower_expr_to_operand(rhs);
                let old_lhs = self.new_temp(lhs.ty, Mutability::Mut, expr.span);
                self.stmts.push(MirStatement {
                    kind: MirStatementKind::Assign(
                        place(old_lhs),
                        Rvalue::Use(MirOperand::Copy(lhs_place.clone())),
                    ),
                    span: expr.span,
                });
                let new_val = self.new_temp(lhs.ty, Mutability::Mut, expr.span);
                self.stmts.push(MirStatement {
                    kind: MirStatementKind::Assign(
                        place(new_val),
                        Rvalue::BinaryOp(
                            self.lower_bin_op(*op),
                            MirOperand::Copy(place(old_lhs)),
                            rhs_value,
                        ),
                    ),
                    span: expr.span,
                });
                self.stmts.push(MirStatement {
                    kind: MirStatementKind::Assign(
                        lhs_place.clone(),
                        Rvalue::Use(MirOperand::Copy(place(new_val))),
                    ),
                    span: expr.span,
                });
                match return_semantic {
                    ReturnSemantic::AfterAssign => MirOperand::Copy(lhs_place),
                    ReturnSemantic::BeforeAssign => MirOperand::Copy(place(old_lhs)),
                }
            }
            HirExprKind::AssignPtrOffset {
                lhs,
                rhs,
                return_semantic,
            } => {
                let lhs_place = self
                    .lower_expr_to_place(lhs)
                    .expect("assignment lhs should be place-expressible");
                let old_lhs = self.new_temp(lhs.ty, Mutability::Mut, expr.span);
                self.stmts.push(MirStatement {
                    kind: MirStatementKind::Assign(
                        place(old_lhs),
                        Rvalue::Use(MirOperand::Copy(lhs_place.clone())),
                    ),
                    span: expr.span,
                });
                let rhs_op = self.lower_expr_to_operand(rhs);
                let isize_ty = Ty::signed_ty(IntTy::Isize);
                let idx_local = self.new_temp(isize_ty, Mutability::Mut, expr.span);
                self.stmts.push(MirStatement {
                    kind: MirStatementKind::Assign(
                        place(idx_local),
                        Rvalue::Cast(CastKind::IntToInt, rhs_op, isize_ty),
                    ),
                    span: expr.span,
                });
                let new_ptr = self.new_temp(lhs.ty, Mutability::Mut, expr.span);
                self.stmts.push(MirStatement {
                    kind: MirStatementKind::Assign(
                        place(new_ptr),
                        Rvalue::BinaryOp(
                            rustc_public_generative::rustc_public::mir::BinOp::Offset,
                            MirOperand::Copy(place(old_lhs)),
                            MirOperand::Copy(place(idx_local)),
                        ),
                    ),
                    span: expr.span,
                });
                self.stmts.push(MirStatement {
                    kind: MirStatementKind::Assign(
                        lhs_place.clone(),
                        Rvalue::Use(MirOperand::Copy(place(new_ptr))),
                    ),
                    span: expr.span,
                });
                match return_semantic {
                    ReturnSemantic::AfterAssign => MirOperand::Copy(lhs_place),
                    ReturnSemantic::BeforeAssign => MirOperand::Copy(place(old_lhs)),
                }
            }
            HirExprKind::AddrOf(inner) => {
                let target_place = if let Some(place) = self.lower_expr_to_place(inner) {
                    place
                } else {
                    let tmp_target = self.new_temp(inner.ty, Mutability::Mut, inner.span);
                    let value = self.lower_expr_to_operand(inner);
                    self.stmts.push(MirStatement {
                        kind: MirStatementKind::Assign(place(tmp_target), Rvalue::Use(value)),
                        span: inner.span,
                    });
                    place(tmp_target)
                };
                let tmp = self.new_temp(expr.ty, Mutability::Mut, expr.span);
                self.stmts.push(MirStatement {
                    kind: MirStatementKind::Assign(
                        place(tmp),
                        Rvalue::AddressOf(
                            rustc_public_generative::rustc_public::mir::RawPtrKind::Mut,
                            target_place,
                        ),
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
            HirExprKind::Cast(inner) => {
                let inner_op = self.lower_expr_to_operand(inner);
                let src_ty = inner.ty;
                let dst_ty = expr.ty;
                let src_is_int = matches!(
                    src_ty.kind(),
                    TyKind::RigidTy(RigidTy::Int(_) | RigidTy::Uint(_))
                );
                let dst_is_int = matches!(
                    dst_ty.kind(),
                    TyKind::RigidTy(RigidTy::Int(_) | RigidTy::Uint(_))
                );
                let src_is_ptr = matches!(src_ty.kind(), TyKind::RigidTy(RigidTy::RawPtr(_, _)));
                let dst_is_ptr = matches!(dst_ty.kind(), TyKind::RigidTy(RigidTy::RawPtr(_, _)));
                let src_is_fn_ptr = matches!(src_ty.kind(), TyKind::RigidTy(RigidTy::FnPtr(_)));
                let dst_is_fn_ptr = matches!(dst_ty.kind(), TyKind::RigidTy(RigidTy::FnPtr(_)));
                let src_is_fn_def = matches!(src_ty.kind(), TyKind::RigidTy(RigidTy::FnDef(_, _)));
                let src_mu_fn_ptr = maybe_uninit_fn_ptr_inner(src_ty);
                let dst_mu_fn_ptr = maybe_uninit_fn_ptr_inner(dst_ty);

                if src_is_int && dst_is_int {
                    let tmp = self.new_temp(dst_ty, Mutability::Mut, expr.span);
                    self.stmts.push(MirStatement {
                        kind: MirStatementKind::Assign(
                            place(tmp),
                            Rvalue::Cast(CastKind::IntToInt, inner_op, dst_ty),
                        ),
                        span: expr.span,
                    });
                    return MirOperand::Copy(place(tmp));
                }

                if src_is_fn_def && dst_is_fn_ptr {
                    let tmp = self.new_temp(dst_ty, Mutability::Mut, expr.span);
                    self.stmts.push(MirStatement {
                        kind: MirStatementKind::Assign(
                            place(tmp),
                            Rvalue::Cast(
                                CastKind::PointerCoercion(PointerCoercion::ReifyFnPointer(
                                    Safety::Safe,
                                )),
                                inner_op,
                                dst_ty,
                            ),
                        ),
                        span: expr.span,
                    });
                    return MirOperand::Copy(place(tmp));
                }

                if dst_mu_fn_ptr.is_some() && src_is_fn_def {
                    let fn_ptr_ty = dst_mu_fn_ptr.expect("checked is_some");
                    let fn_ptr_local = self.new_temp(fn_ptr_ty, Mutability::Mut, expr.span);
                    self.stmts.push(MirStatement {
                        kind: MirStatementKind::Assign(
                            place(fn_ptr_local),
                            Rvalue::Cast(
                                CastKind::PointerCoercion(PointerCoercion::ReifyFnPointer(
                                    Safety::Safe,
                                )),
                                inner_op,
                                fn_ptr_ty,
                            ),
                        ),
                        span: expr.span,
                    });
                    return self.write_value_into_maybe_uninit_storage(
                        dst_ty,
                        MirOperand::Copy(place(fn_ptr_local)),
                        fn_ptr_ty,
                        expr.span,
                    );
                }

                if dst_mu_fn_ptr.is_some() && src_is_fn_ptr {
                    return self.write_value_into_maybe_uninit_storage(
                        dst_ty, inner_op, src_ty, expr.span,
                    );
                }

                if src_is_ptr && dst_is_ptr {
                    let tmp = self.new_temp(dst_ty, Mutability::Mut, expr.span);
                    self.stmts.push(MirStatement {
                        kind: MirStatementKind::Assign(
                            place(tmp),
                            Rvalue::Cast(CastKind::PtrToPtr, inner_op, dst_ty),
                        ),
                        span: expr.span,
                    });
                    return MirOperand::Copy(place(tmp));
                }

                if src_is_ptr && dst_is_int {
                    let usize_ty = Ty::usize_ty();
                    let usize_tmp = self.new_temp(usize_ty, Mutability::Mut, expr.span);
                    self.stmts.push(MirStatement {
                        kind: MirStatementKind::Assign(
                            place(usize_tmp),
                            Rvalue::Cast(CastKind::PointerExposeAddress, inner_op, usize_ty),
                        ),
                        span: expr.span,
                    });
                    if dst_ty == usize_ty {
                        return MirOperand::Copy(place(usize_tmp));
                    }
                    let dst_tmp = self.new_temp(dst_ty, Mutability::Mut, expr.span);
                    self.stmts.push(MirStatement {
                        kind: MirStatementKind::Assign(
                            place(dst_tmp),
                            Rvalue::Cast(
                                CastKind::IntToInt,
                                MirOperand::Copy(place(usize_tmp)),
                                dst_ty,
                            ),
                        ),
                        span: expr.span,
                    });
                    return MirOperand::Copy(place(dst_tmp));
                }

                if src_mu_fn_ptr.is_some() && dst_is_int {
                    let usize_ty = Ty::usize_ty();
                    let usize_op = self.read_maybe_uninit_as(&inner, usize_ty, expr.span);
                    if dst_ty == usize_ty {
                        return usize_op;
                    }
                    let dst_tmp = self.new_temp(dst_ty, Mutability::Mut, expr.span);
                    self.stmts.push(MirStatement {
                        kind: MirStatementKind::Assign(
                            place(dst_tmp),
                            Rvalue::Cast(CastKind::IntToInt, usize_op, dst_ty),
                        ),
                        span: expr.span,
                    });
                    return MirOperand::Copy(place(dst_tmp));
                }

                if src_is_int && dst_is_ptr {
                    let usize_ty = Ty::usize_ty();
                    let usize_op = if src_ty == usize_ty {
                        inner_op
                    } else {
                        let usize_tmp = self.new_temp(usize_ty, Mutability::Mut, expr.span);
                        self.stmts.push(MirStatement {
                            kind: MirStatementKind::Assign(
                                place(usize_tmp),
                                Rvalue::Cast(CastKind::IntToInt, inner_op, usize_ty),
                            ),
                            span: expr.span,
                        });
                        MirOperand::Copy(place(usize_tmp))
                    };
                    let dst_tmp = self.new_temp(dst_ty, Mutability::Mut, expr.span);
                    self.stmts.push(MirStatement {
                        kind: MirStatementKind::Assign(
                            place(dst_tmp),
                            Rvalue::Cast(CastKind::PointerWithExposedProvenance, usize_op, dst_ty),
                        ),
                        span: expr.span,
                    });
                    return MirOperand::Copy(place(dst_tmp));
                }

                if src_is_int && dst_mu_fn_ptr.is_some() {
                    let usize_ty = Ty::usize_ty();
                    let usize_op = if src_ty == usize_ty {
                        inner_op
                    } else {
                        let usize_tmp = self.new_temp(usize_ty, Mutability::Mut, expr.span);
                        self.stmts.push(MirStatement {
                            kind: MirStatementKind::Assign(
                                place(usize_tmp),
                                Rvalue::Cast(CastKind::IntToInt, inner_op, usize_ty),
                            ),
                            span: expr.span,
                        });
                        MirOperand::Copy(place(usize_tmp))
                    };
                    return self.write_value_into_maybe_uninit_storage(
                        dst_ty, usize_op, usize_ty, expr.span,
                    );
                }

                if src_mu_fn_ptr.is_some() && dst_is_fn_ptr {
                    return self.read_maybe_uninit_as(&inner, dst_ty, expr.span);
                }

                panic!("unsupported cast from {:?} to {:?}", src_ty, dst_ty);
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

    fn lower_logical_expr(
        &mut self,
        op: HirLogicalOp,
        lhs: &HirExpr,
        rhs: &HirExpr,
        span: RustSpan,
        ty: Ty,
    ) -> MirOperand {
        let result_local = self.new_temp(ty, Mutability::Mut, span);
        let zero_init = self.lower_expr_to_operand(&HirExpr {
            kind: HirExprKind::ConstInt(0),
            ty,
            span,
        });
        self.stmts.push(MirStatement {
            kind: MirStatementKind::Assign(place(result_local), Rvalue::Use(zero_init)),
            span,
        });

        let lhs_op = self.lower_expr_to_operand(lhs);
        let entry_bb = self.blocks.len();
        self.blocks
            .push(rustc_public_generative::rustc_public::mir::BasicBlock {
                statements: std::mem::take(&mut self.stmts),
                terminator: rustc_public_generative::rustc_public::mir::Terminator {
                    kind: TerminatorKind::SwitchInt {
                        discr: lhs_op,
                        targets: SwitchTargets::new(vec![(0, usize::MAX)], usize::MAX),
                    },
                    span,
                },
            });

        let (lhs_short_bb, rhs_eval_bb, lhs_short_val) = match op {
            HirLogicalOp::And => (self.blocks.len(), self.blocks.len() + 1, 0),
            HirLogicalOp::Or => (self.blocks.len(), self.blocks.len() + 1, 1),
        };

        let lhs_short_operand = self.lower_expr_to_operand(&HirExpr {
            kind: HirExprKind::ConstInt(lhs_short_val),
            ty,
            span,
        });
        self.stmts.push(MirStatement {
            kind: MirStatementKind::Assign(place(result_local), Rvalue::Use(lhs_short_operand)),
            span,
        });
        let lhs_short_exit =
            self.push_terminator(TerminatorKind::Goto { target: usize::MAX }, span);

        debug_assert_eq!(rhs_eval_bb, self.blocks.len());
        let rhs_op = self.lower_expr_to_operand(rhs);
        let rhs_switch_bb = self.blocks.len();
        self.blocks
            .push(rustc_public_generative::rustc_public::mir::BasicBlock {
                statements: std::mem::take(&mut self.stmts),
                terminator: rustc_public_generative::rustc_public::mir::Terminator {
                    kind: TerminatorKind::SwitchInt {
                        discr: rhs_op,
                        targets: SwitchTargets::new(vec![(0, usize::MAX)], usize::MAX),
                    },
                    span,
                },
            });

        let rhs_false_bb = self.blocks.len();
        let rhs_false_operand = self.lower_expr_to_operand(&HirExpr {
            kind: HirExprKind::ConstInt(0),
            ty,
            span,
        });
        self.stmts.push(MirStatement {
            kind: MirStatementKind::Assign(place(result_local), Rvalue::Use(rhs_false_operand)),
            span,
        });
        let rhs_false_exit =
            self.push_terminator(TerminatorKind::Goto { target: usize::MAX }, span);

        let rhs_true_bb = self.blocks.len();
        let rhs_true_operand = self.lower_expr_to_operand(&HirExpr {
            kind: HirExprKind::ConstInt(1),
            ty,
            span,
        });
        self.stmts.push(MirStatement {
            kind: MirStatementKind::Assign(place(result_local), Rvalue::Use(rhs_true_operand)),
            span,
        });
        let rhs_true_exit = self.push_terminator(TerminatorKind::Goto { target: usize::MAX }, span);

        let join_bb = self.blocks.len();

        self.patch_goto_target(lhs_short_exit, join_bb);
        self.patch_goto_target(rhs_false_exit, join_bb);
        self.patch_goto_target(rhs_true_exit, join_bb);
        match op {
            HirLogicalOp::And => {
                self.patch_switch_targets(entry_bb, rhs_eval_bb, lhs_short_bb);
            }
            HirLogicalOp::Or => {
                self.patch_switch_targets(entry_bb, lhs_short_bb, rhs_eval_bb);
            }
        }
        self.patch_switch_targets(rhs_switch_bb, rhs_true_bb, rhs_false_bb);

        MirOperand::Copy(place(result_local))
    }

    fn lower_logical_not_expr(&mut self, inner: &HirExpr, span: RustSpan, ty: Ty) -> MirOperand {
        let result_local = self.new_temp(ty, Mutability::Mut, span);
        let zero_init = self.lower_expr_to_operand(&HirExpr {
            kind: HirExprKind::ConstInt(0),
            ty,
            span,
        });
        self.stmts.push(MirStatement {
            kind: MirStatementKind::Assign(place(result_local), Rvalue::Use(zero_init)),
            span,
        });
        let inner_op = self.lower_expr_to_operand(inner);
        let entry_bb = self.blocks.len();
        self.blocks
            .push(rustc_public_generative::rustc_public::mir::BasicBlock {
                statements: std::mem::take(&mut self.stmts),
                terminator: rustc_public_generative::rustc_public::mir::Terminator {
                    kind: TerminatorKind::SwitchInt {
                        discr: inner_op,
                        targets: SwitchTargets::new(vec![(0, usize::MAX)], usize::MAX),
                    },
                    span,
                },
            });

        let true_bb = self.blocks.len();
        let one = self.lower_expr_to_operand(&HirExpr {
            kind: HirExprKind::ConstInt(1),
            ty,
            span,
        });
        self.stmts.push(MirStatement {
            kind: MirStatementKind::Assign(place(result_local), Rvalue::Use(one)),
            span,
        });
        let true_exit = self.push_terminator(TerminatorKind::Goto { target: usize::MAX }, span);

        let false_bb = self.blocks.len();
        let zero = self.lower_expr_to_operand(&HirExpr {
            kind: HirExprKind::ConstInt(0),
            ty,
            span,
        });
        self.stmts.push(MirStatement {
            kind: MirStatementKind::Assign(place(result_local), Rvalue::Use(zero)),
            span,
        });
        let false_exit = self.push_terminator(TerminatorKind::Goto { target: usize::MAX }, span);

        let join_bb = self.blocks.len();
        self.patch_goto_target(true_exit, join_bb);
        self.patch_goto_target(false_exit, join_bb);
        self.patch_switch_targets(entry_bb, false_bb, true_bb);

        MirOperand::Copy(place(result_local))
    }

    fn lower_conditional_expr(
        &mut self,
        cond: &HirExpr,
        then_expr: &HirExpr,
        else_expr: &HirExpr,
        span: RustSpan,
        ty: Ty,
    ) -> MirOperand {
        let result_local = self.new_temp(ty, Mutability::Mut, span);
        let cond_is_maybe_uninit_fn_ptr = matches!(
            cond.ty.kind(),
            TyKind::RigidTy(RigidTy::Adt(_, args))
                if args.0.len() == 1
                    && matches!(args.0[0], GenericArgKind::Type(ty) if matches!(ty.kind(), TyKind::RigidTy(RigidTy::FnPtr(_))))
        );
        let cond_expr = if matches!(
            cond.ty.kind(),
            TyKind::RigidTy(RigidTy::RawPtr(_, _) | RigidTy::FnPtr(_))
        ) || cond_is_maybe_uninit_fn_ptr
        {
            HirExpr {
                kind: HirExprKind::Cast(Box::new(cond.clone())),
                ty: Ty::usize_ty(),
                span: cond.span,
            }
        } else {
            cond.clone()
        };
        let cond_op = self.lower_expr_to_operand(&cond_expr);
        let entry_bb = self.blocks.len();
        self.blocks
            .push(rustc_public_generative::rustc_public::mir::BasicBlock {
                statements: std::mem::take(&mut self.stmts),
                terminator: rustc_public_generative::rustc_public::mir::Terminator {
                    kind: TerminatorKind::SwitchInt {
                        discr: cond_op,
                        targets: SwitchTargets::new(vec![(0, usize::MAX)], usize::MAX),
                    },
                    span,
                },
            });

        let then_bb = self.blocks.len();
        let then_op = self.lower_expr_to_operand(then_expr);
        self.stmts.push(MirStatement {
            kind: MirStatementKind::Assign(place(result_local), Rvalue::Use(then_op)),
            span: then_expr.span,
        });
        let then_exit = self.push_terminator(TerminatorKind::Goto { target: usize::MAX }, span);

        let else_bb = self.blocks.len();
        let else_op = self.lower_expr_to_operand(else_expr);
        self.stmts.push(MirStatement {
            kind: MirStatementKind::Assign(place(result_local), Rvalue::Use(else_op)),
            span: else_expr.span,
        });
        let else_exit = self.push_terminator(TerminatorKind::Goto { target: usize::MAX }, span);

        let join_bb = self.blocks.len();
        self.patch_goto_target(then_exit, join_bb);
        self.patch_goto_target(else_exit, join_bb);
        self.patch_switch_targets(entry_bb, then_bb, else_bb);

        MirOperand::Copy(place(result_local))
    }

    pub(crate) fn lower_call_to_destination(
        &mut self,
        func: &HirExpr,
        args: &[HirExpr],
        span: RustSpan,
        destination: rustc_public_generative::rustc_public::mir::Place,
        ret_ty: Ty,
    ) {
        let sig = callable_sig(func.ty)
            .expect("call target has no fn signature")
            .skip_binder();

        let mut arg_ops = Vec::with_capacity(args.len());
        for (idx, arg) in args.iter().enumerate() {
            let op = if let Some(expected_ty) = sig.inputs().get(idx) {
                self.lower_call_arg(arg, *expected_ty)
            } else {
                self.lower_expr_to_operand(arg)
            };
            arg_ops.push(op);
        }
        match &func.kind {
            HirExprKind::Path(ResolvedValue::Fn(fn_def)) => {
                let generic_args = infer_fn_generic_args(&sig, args, ret_ty);
                self.emit_call_block(
                    fn_const_operand(*fn_def, generic_args, span),
                    arg_ops,
                    destination,
                    span,
                );
            }
            _ => {
                let func_op = if let Some(inner_fn_ptr) = maybe_uninit_fn_ptr_inner(func.ty) {
                    self.read_maybe_uninit_as(func, inner_fn_ptr, span)
                } else {
                    self.lower_expr_to_operand(func)
                };
                self.emit_call_block(func_op, arg_ops, destination, span);
            }
        }
    }

    pub(crate) fn lower_zeroed_to_destination(
        &mut self,
        destination: rustc_public_generative::rustc_public::mir::Place,
        span: RustSpan,
        ret_ty: Ty,
    ) {
        let zeroed_fn =
            crate::build::dep_fn_any(self.deps, &["std::mem::zeroed", "core::mem::zeroed"]);
        let sig = zeroed_fn
            .ty()
            .kind()
            .fn_sig()
            .expect("std::mem::zeroed has no signature")
            .skip_binder();
        let generic_args = infer_fn_generic_args(&sig, &[], ret_ty);
        self.emit_call_block(
            fn_const_operand(zeroed_fn, generic_args, span),
            vec![],
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
        MirOperand::Copy(place(ref_local))
    }
}
