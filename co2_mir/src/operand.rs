use co2_hir::{HirExpr, HirExprKind, HirLogicalOp, ResolvedValue, ReturnSemantic, WellknownDefs};
use rustc_public_generative::rustc_public::{
    CrateDefType,
    mir::{
        AggregateKind, BorrowKind, CastKind, ConstOperand, MutBorrowKind, Mutability,
        Operand as MirOperand, PointerCoercion, ProjectionElem as MirProjection, RawPtrKind,
        Rvalue, Safety, Statement as MirStatement, StatementKind as MirStatementKind,
        SwitchTargets, TerminatorKind,
    },
    ty::{
        FloatTy, GenericArgKind, GenericArgs, IntTy, MirConst, Region, RegionKind, RigidTy,
        Span as RustSpan, Ty, TyKind, UintTy,
    },
};

use crate::{
    build::{
        Builder, complete_fn_generic_args, fn_const_operand, infer_fn_generic_args,
        ty_matches_expected, variant_idx,
    },
    place::place,
};

fn find_ptr_offset_fn(
    deps: &WellknownDefs,
    mutability: Mutability,
) -> rustc_public_generative::rustc_public::ty::FnDef {
    match mutability {
        Mutability::Mut => deps.offset_mut,
        Mutability::Not => deps.offset_const,
    }
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

fn enum_payload_ty(ty: Ty) -> Option<Ty> {
    let TyKind::RigidTy(RigidTy::Adt(adt, args)) = ty.kind() else {
        return None;
    };
    let variant = adt.variant(variant_idx(0))?;
    let fields = variant.fields();
    if fields.len() != 1 || fields[0].name.clone() != "__co2_enum_value" {
        return None;
    }
    Some(fields[0].ty_with_args(&args))
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

impl Builder<'_, '_> {
    fn place_operand_for_ty(
        &self,
        place: rustc_public_generative::rustc_public::mir::Place,
        ty: Ty,
    ) -> MirOperand {
        if self.ctx.type_is_copy(self.owner, ty) {
            MirOperand::Copy(place)
        } else {
            MirOperand::Move(place)
        }
    }

    fn const_int_expr(value: i128, ty: Ty, span: RustSpan) -> HirExpr {
        HirExpr {
            kind: HirExprKind::ConstInt(value),
            ty,
            span,
        }
    }

    fn emit_cast_expr(expr: HirExpr, ty: Ty) -> HirExpr {
        if expr.ty == ty {
            expr
        } else {
            let span = expr.span;
            HirExpr {
                kind: HirExprKind::Cast(Box::new(expr)),
                ty,
                span,
            }
        }
    }

    fn bitfield_storage_bits(&self, ty: Ty) -> usize {
        match ty.kind() {
            TyKind::RigidTy(RigidTy::Uint(UintTy::U8) | RigidTy::Int(IntTy::I8)) => 8,
            TyKind::RigidTy(RigidTy::Uint(UintTy::U16) | RigidTy::Int(IntTy::I16)) => 16,
            TyKind::RigidTy(RigidTy::Uint(UintTy::U32) | RigidTy::Int(IntTy::I32)) => 32,
            TyKind::RigidTy(
                RigidTy::Uint(UintTy::U64 | UintTy::Usize)
                | RigidTy::Int(IntTy::I64 | IntTy::Isize),
            ) => 64,
            TyKind::RigidTy(RigidTy::Uint(UintTy::U128) | RigidTy::Int(IntTy::I128)) => 128,
            other => panic!("unsupported bitfield storage type: {other:?}"),
        }
    }

    fn signed_ty_for_storage(&self, ty: Ty) -> Ty {
        match ty.kind() {
            TyKind::RigidTy(RigidTy::Uint(UintTy::U8)) => Ty::signed_ty(IntTy::I8),
            TyKind::RigidTy(RigidTy::Uint(UintTy::U16)) => Ty::signed_ty(IntTy::I16),
            TyKind::RigidTy(RigidTy::Uint(UintTy::U32)) => Ty::signed_ty(IntTy::I32),
            TyKind::RigidTy(RigidTy::Uint(UintTy::U64)) => Ty::signed_ty(IntTy::I64),
            TyKind::RigidTy(RigidTy::Uint(UintTy::U128)) => Ty::signed_ty(IntTy::I128),
            TyKind::RigidTy(RigidTy::Uint(UintTy::Usize)) => Ty::signed_ty(IntTy::Isize),
            _ => ty,
        }
    }

    fn read_enum_payload_operand(
        &mut self,
        enum_op: MirOperand,
        enum_ty: Ty,
        payload_ty: Ty,
        span: RustSpan,
    ) -> MirOperand {
        let tmp = self.new_temp(enum_ty, Mutability::Mut, span);
        let mut payload_place = place(tmp);
        payload_place
            .projection
            .push(MirProjection::Field(0, payload_ty));
        self.stmts.push(MirStatement {
            kind: MirStatementKind::Assign(place(tmp), Rvalue::Use(enum_op)),
            span,
        });
        self.place_operand_for_ty(payload_place, payload_ty)
    }

    fn wrap_enum_payload_operand(
        &mut self,
        payload_op: MirOperand,
        _payload_ty: Ty,
        enum_ty: Ty,
        span: RustSpan,
    ) -> MirOperand {
        let tmp = self.new_temp(enum_ty, Mutability::Mut, span);
        self.stmts.push(MirStatement {
            kind: MirStatementKind::Assign(
                place(tmp),
                Rvalue::Aggregate(
                    match enum_ty.kind() {
                        TyKind::RigidTy(RigidTy::Adt(adt, adt_args)) => {
                            AggregateKind::Adt(adt, variant_idx(0), adt_args, None, None)
                        }
                        _ => unreachable!("enum wrapper must be an adt"),
                    },
                    vec![payload_op],
                ),
            ),
            span,
        });
        MirOperand::Copy(place(tmp))
    }

    fn bitfield_mask_expr(&self, width: usize, storage_ty: Ty, span: RustSpan) -> HirExpr {
        if width >= 128 {
            Self::emit_cast_expr(
                Self::const_int_expr(-1, Ty::signed_ty(IntTy::I32), span),
                storage_ty,
            )
        } else {
            Self::emit_cast_expr(
                Self::const_int_expr(
                    ((1u128 << width) - 1) as i128,
                    Ty::signed_ty(IntTy::I32),
                    span,
                ),
                storage_ty,
            )
        }
    }

    fn bitfield_storage_expr(
        &self,
        base: &HirExpr,
        storage_index: usize,
        storage_ty: Ty,
        span: RustSpan,
    ) -> HirExpr {
        HirExpr {
            kind: HirExprKind::Field {
                base: Box::new(base.clone()),
                index: storage_index,
            },
            ty: storage_ty,
            span,
        }
    }

    fn bitfield_read_expr(
        &self,
        base: &HirExpr,
        storage_index: usize,
        storage_ty: Ty,
        bit_offset: usize,
        bit_width: usize,
        signed: bool,
        result_ty: Ty,
        span: RustSpan,
    ) -> HirExpr {
        let storage = self.bitfield_storage_expr(base, storage_index, storage_ty, span);
        let shifted = if bit_offset == 0 {
            storage
        } else {
            HirExpr {
                kind: HirExprKind::Binary {
                    op: co2_hir::HirBinOp::Shr,
                    lhs: Box::new(storage),
                    rhs: Box::new(Self::emit_cast_expr(
                        Self::const_int_expr(bit_offset as i128, Ty::signed_ty(IntTy::I32), span),
                        storage_ty,
                    )),
                },
                ty: storage_ty,
                span,
            }
        };
        let masked = HirExpr {
            kind: HirExprKind::Binary {
                op: co2_hir::HirBinOp::BitAnd,
                lhs: Box::new(shifted),
                rhs: Box::new(self.bitfield_mask_expr(bit_width, storage_ty, span)),
            },
            ty: storage_ty,
            span,
        };
        if signed {
            let signed_storage_ty = self.signed_ty_for_storage(storage_ty);
            let signed_value = Self::emit_cast_expr(masked, signed_storage_ty);
            let shift = self.bitfield_storage_bits(storage_ty) - bit_width;
            let sign_extended = if shift == 0 {
                signed_value
            } else {
                let shifted_left = HirExpr {
                    kind: HirExprKind::Binary {
                        op: co2_hir::HirBinOp::Shl,
                        lhs: Box::new(signed_value),
                        rhs: Box::new(Self::emit_cast_expr(
                            Self::const_int_expr(shift as i128, Ty::signed_ty(IntTy::I32), span),
                            signed_storage_ty,
                        )),
                    },
                    ty: signed_storage_ty,
                    span,
                };
                HirExpr {
                    kind: HirExprKind::Binary {
                        op: co2_hir::HirBinOp::Shr,
                        lhs: Box::new(shifted_left),
                        rhs: Box::new(Self::emit_cast_expr(
                            Self::const_int_expr(shift as i128, Ty::signed_ty(IntTy::I32), span),
                            signed_storage_ty,
                        )),
                    },
                    ty: signed_storage_ty,
                    span,
                }
            };
            Self::emit_cast_expr(sign_extended, result_ty)
        } else {
            Self::emit_cast_expr(masked, result_ty)
        }
    }

    fn bitfield_insert_expr(
        &self,
        current_storage: HirExpr,
        value: HirExpr,
        storage_ty: Ty,
        bit_offset: usize,
        bit_width: usize,
        span: RustSpan,
    ) -> HirExpr {
        let value_mask = self.bitfield_mask_expr(bit_width, storage_ty, span);
        let field_mask = if bit_offset == 0 {
            value_mask.clone()
        } else {
            HirExpr {
                kind: HirExprKind::Binary {
                    op: co2_hir::HirBinOp::Shl,
                    lhs: Box::new(value_mask.clone()),
                    rhs: Box::new(Self::emit_cast_expr(
                        Self::const_int_expr(bit_offset as i128, Ty::signed_ty(IntTy::I32), span),
                        storage_ty,
                    )),
                },
                ty: storage_ty,
                span,
            }
        };
        let cleared = HirExpr {
            kind: HirExprKind::Binary {
                op: co2_hir::HirBinOp::BitAnd,
                lhs: Box::new(current_storage),
                rhs: Box::new(HirExpr {
                    kind: HirExprKind::BitNot(Box::new(field_mask.clone())),
                    ty: storage_ty,
                    span,
                }),
            },
            ty: storage_ty,
            span,
        };
        let masked_value = HirExpr {
            kind: HirExprKind::Binary {
                op: co2_hir::HirBinOp::BitAnd,
                lhs: Box::new(Self::emit_cast_expr(value, storage_ty)),
                rhs: Box::new(value_mask),
            },
            ty: storage_ty,
            span,
        };
        let shifted = if bit_offset == 0 {
            masked_value
        } else {
            HirExpr {
                kind: HirExprKind::Binary {
                    op: co2_hir::HirBinOp::Shl,
                    lhs: Box::new(masked_value),
                    rhs: Box::new(Self::emit_cast_expr(
                        Self::const_int_expr(bit_offset as i128, Ty::signed_ty(IntTy::I32), span),
                        storage_ty,
                    )),
                },
                ty: storage_ty,
                span,
            }
        };
        HirExpr {
            kind: HirExprKind::Binary {
                op: co2_hir::HirBinOp::BitOr,
                lhs: Box::new(cleared),
                rhs: Box::new(shifted),
            },
            ty: storage_ty,
            span,
        }
    }

    fn emit_bitfield_store(&mut self, lhs: &HirExpr, value: HirExpr, span: RustSpan) {
        let HirExprKind::Bitfield {
            base,
            storage_index,
            storage_ty,
            bit_offset,
            bit_width,
            ..
        } = &lhs.kind
        else {
            panic!("bitfield store requires bitfield lhs");
        };
        let storage_lhs = self.bitfield_storage_expr(base, *storage_index, *storage_ty, span);
        let current_storage = self.bitfield_storage_expr(base, *storage_index, *storage_ty, span);
        let rhs = self.bitfield_insert_expr(
            current_storage,
            value,
            *storage_ty,
            *bit_offset,
            *bit_width,
            span,
        );
        let assign_expr = HirExpr {
            kind: HirExprKind::Assign {
                lhs: Box::new(storage_lhs),
                rhs: Box::new(rhs),
            },
            ty: *storage_ty,
            span,
        };
        let _ = self.lower_expr_to_operand(&assign_expr);
    }

    fn emit_ptr_offset(
        &mut self,
        base_op: MirOperand,
        pointee_ty: Ty,
        ptr_mutability: Mutability,
        index: &HirExpr,
        out_ty: Ty,
        span: RustSpan,
    ) -> MirOperand {
        let isize_ty = Ty::signed_ty(IntTy::Isize);
        let idx_ty = index.ty;
        let idx_op = self.lower_expr_to_operand(index);
        let idx_op = self.lower_cast(idx_op, idx_ty, isize_ty, span);

        let offset = find_ptr_offset_fn(&self.wellknown_defs, ptr_mutability);
        let generic_args = match offset.ty().kind() {
            TyKind::RigidTy(RigidTy::FnDef(_, existing)) if !existing.0.is_empty() => existing
                .0
                .iter()
                .map(|arg| match arg {
                    GenericArgKind::Type(ty) if matches!(ty.kind(), TyKind::Param(_)) => {
                        GenericArgKind::Type(pointee_ty)
                    }
                    _ => arg.clone(),
                })
                .collect(),
            _ => vec![GenericArgKind::Type(pointee_ty)],
        };

        let ret_local = self.new_temp(out_ty, Mutability::Mut, span);
        self.emit_call_block(
            fn_const_operand(offset, generic_args, span),
            vec![base_op, idx_op],
            place(ret_local),
            span,
        );
        MirOperand::Copy(place(ret_local))
    }

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

    fn read_maybe_uninit_as(
        &mut self,
        op: MirOperand,
        op_ty: Ty,
        value_ty: Ty,
        span: RustSpan,
    ) -> MirOperand {
        let src_place = {
            let tmp = self.new_temp(op_ty, Mutability::Mut, span);
            self.stmts.push(MirStatement {
                kind: MirStatementKind::Assign(place(tmp), Rvalue::Use(op)),
                span,
            });
            place(tmp)
        };
        let ptr_maybe_ty = Ty::new_ptr(op_ty, Mutability::Mut);
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
                let TyKind::RigidTy(RigidTy::RawPtr(_, ptr_mutability)) = expr.ty.kind() else {
                    panic!("array decay expects pointer type, got {:?}", expr.ty);
                };
                let array_ptr_ty = Ty::new_ptr(inner.ty, ptr_mutability);
                let array_ptr_local = self.new_temp(array_ptr_ty, Mutability::Mut, expr.span);
                self.stmts.push(MirStatement {
                    kind: MirStatementKind::Assign(
                        place(array_ptr_local),
                        Rvalue::AddressOf(
                            if ptr_mutability == Mutability::Mut {
                                RawPtrKind::Mut
                            } else {
                                RawPtrKind::Const
                            },
                            base_place,
                        ),
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
            HirExprKind::VaStart(args) => {
                let args_ty = args.ty;
                let Some(args) = self.lower_expr_to_place(args) else {
                    panic!("VaStart operand was not lvalue");
                };
                let src_local = self.c_variadic_local.unwrap();
                let src_ty = self.locals[src_local].ty;
                let reg = Region {
                    kind: RegionKind::ReErased,
                };
                let src_ref_ty = Ty::new_ref(reg.clone(), src_ty, Mutability::Not);
                let src_ref_local = self.new_temp(src_ref_ty, Mutability::Not, expr.span);
                self.stmts.push(MirStatement {
                    kind: MirStatementKind::Assign(
                        place(src_ref_local),
                        Rvalue::Ref(reg, BorrowKind::Shared, place(src_local)),
                    ),
                    span: expr.span,
                });

                let clone_local = self.new_temp(src_ty, Mutability::Mut, expr.span);
                self.emit_call_block(
                    fn_const_operand(
                        self.wellknown_defs.clone,
                        vec![GenericArgKind::Type(src_ty)],
                        expr.span,
                    ),
                    vec![MirOperand::Copy(place(src_ref_local))],
                    place(clone_local),
                    expr.span,
                );

                let transmute_fn = self.wellknown_defs.transmute;
                let generic_args =
                    vec![GenericArgKind::Type(src_ty), GenericArgKind::Type(args_ty)];
                self.emit_call_block(
                    fn_const_operand(transmute_fn, generic_args, expr.span),
                    vec![MirOperand::Move(place(clone_local))],
                    args,
                    expr.span,
                );
                let temp = self.new_temp(expr.ty, Mutability::Mut, expr.span);
                self.lower_zeroed_to_destination(place(temp), expr.span, expr.ty);
                MirOperand::Copy(place(temp))
            }
            HirExprKind::VaArg(args) => {
                let reg = Region {
                    kind: RegionKind::ReErased,
                };
                let mut target_ty = expr.ty;
                let mut need_deref = false;
                if target_ty.kind().is_adt() {
                    target_ty = Ty::new_ptr(target_ty, Mutability::Mut);
                    need_deref = true;
                }
                let arg_ref_ty = Ty::new_ref(reg.clone(), args.ty, Mutability::Mut);
                let Some(args) = self.lower_expr_to_place(args) else {
                    panic!("VaArg operand was not lvalue");
                };
                let arg_ref = {
                    let tmp = self.new_temp(arg_ref_ty, Mutability::Mut, expr.span);
                    self.stmts.push(MirStatement {
                        kind: MirStatementKind::Assign(
                            place(tmp),
                            Rvalue::Ref(
                                reg.clone(),
                                BorrowKind::Mut {
                                    kind: MutBorrowKind::Default,
                                },
                                args,
                            ),
                        ),
                        span: expr.span,
                    });
                    MirOperand::Move(place(tmp))
                };

                let ret_local = self.new_temp(target_ty, Mutability::Mut, expr.span);
                let generic_args = vec![
                    GenericArgKind::Lifetime(reg),
                    GenericArgKind::Type(target_ty),
                ];
                self.emit_call_block(
                    fn_const_operand(self.wellknown_defs.valist_fn_arg, generic_args, expr.span),
                    vec![arg_ref],
                    place(ret_local),
                    expr.span,
                );
                let mut ret_place = place(ret_local);
                if need_deref {
                    ret_place.projection.push(MirProjection::Deref);
                }
                MirOperand::Copy(ret_place)
            }
            HirExprKind::VaCopy { dest, src } => {
                let dest_ty = dest.ty;
                let src_ty = src.ty;
                let Some(dest) = self.lower_expr_to_place(dest) else {
                    panic!("VaCopy destination operand was not lvalue");
                };
                let Some(src) = self.lower_expr_to_place(src) else {
                    panic!("VaCopy source operand was not lvalue");
                };

                let reg = Region {
                    kind: RegionKind::ReErased,
                };
                let src_ref_ty = Ty::new_ref(reg.clone(), src_ty, Mutability::Not);
                let src_ref_local = self.new_temp(src_ref_ty, Mutability::Not, expr.span);
                self.stmts.push(MirStatement {
                    kind: MirStatementKind::Assign(
                        place(src_ref_local),
                        Rvalue::Ref(reg, BorrowKind::Shared, src),
                    ),
                    span: expr.span,
                });

                let clone_local = self.new_temp(src_ty, Mutability::Mut, expr.span);
                self.emit_call_block(
                    fn_const_operand(
                        self.wellknown_defs.clone,
                        vec![GenericArgKind::Type(src_ty)],
                        expr.span,
                    ),
                    vec![MirOperand::Copy(place(src_ref_local))],
                    place(clone_local),
                    expr.span,
                );

                let generic_args =
                    vec![GenericArgKind::Type(src_ty), GenericArgKind::Type(dest_ty)];
                self.emit_call_block(
                    fn_const_operand(self.wellknown_defs.transmute, generic_args, expr.span),
                    vec![MirOperand::Move(place(clone_local))],
                    dest,
                    expr.span,
                );
                let temp = self.new_temp(expr.ty, Mutability::Mut, expr.span);
                self.lower_zeroed_to_destination(place(temp), expr.span, expr.ty);
                MirOperand::Copy(place(temp))
            }
            HirExprKind::VaEnd(args) => {
                let Some(_args) = self.lower_expr_to_place(args) else {
                    panic!("VaEnd operand was not lvalue");
                };
                let temp = self.new_temp(expr.ty, Mutability::Mut, expr.span);
                self.lower_zeroed_to_destination(place(temp), expr.span, expr.ty);
                MirOperand::Copy(place(temp))
            }

            HirExprKind::Zeroed => {
                let temp = self.new_temp(expr.ty, Mutability::Mut, expr.span);
                self.lower_zeroed_to_destination(place(temp), expr.span, expr.ty);
                MirOperand::Copy(place(temp))
            }
            HirExprKind::Local(local) | HirExprKind::LocalConst(local) => {
                let local_index = self.local_to_index(*local);
                self.place_operand_for_ty(place(local_index), self.locals[local_index].ty)
            }
            HirExprKind::LabelAddress(label) => {
                let discr = *self
                    .label_discriminants
                    .get(label)
                    .unwrap_or_else(|| panic!("missing label discriminant for `{label:?}`"));
                self.lower_expr_to_operand(&HirExpr {
                    kind: HirExprKind::ConstInt(discr as i128),
                    ty: expr.ty,
                    span: expr.span,
                })
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

                let src_ty = Ty::unsigned_ty(uint_ty);
                if src_ty == expr.ty {
                    return const_op;
                }
                self.lower_cast(const_op, src_ty, expr.ty, span)
            }
            HirExprKind::ConstFloat(v) => {
                let span = expr.span;
                let TyKind::RigidTy(RigidTy::Float(_)) = expr.ty.kind() else {
                    panic!("float const must have float type, got {:?}", expr.ty);
                };
                let c = MirConst::try_from_float(*v, FloatTy::F64)
                    .expect("failed to build float const");
                let const_op = MirOperand::Constant(ConstOperand {
                    span,
                    user_ty: None,
                    const_: c,
                });

                self.lower_cast(
                    const_op,
                    Ty::from_rigid_kind(RigidTy::Float(FloatTy::F64)),
                    expr.ty,
                    span,
                )
            }
            HirExprKind::Field { .. } => {
                let place = self
                    .lower_expr_to_place(expr)
                    .expect("field expression should be place-expressible");
                MirOperand::Copy(place)
            }
            HirExprKind::Bitfield {
                base,
                storage_index,
                storage_ty,
                bit_offset,
                bit_width,
                signed,
            } => {
                let read_expr = self.bitfield_read_expr(
                    base,
                    *storage_index,
                    *storage_ty,
                    *bit_offset,
                    *bit_width,
                    *signed,
                    expr.ty,
                    expr.span,
                );
                self.lower_expr_to_operand(&read_expr)
            }
            HirExprKind::PtrOffset { base, index } => {
                let base_op = self.lower_expr_to_operand(base);
                let TyKind::RigidTy(RigidTy::RawPtr(pointee_ty, mutability)) = base.ty.kind()
                else {
                    panic!("ptr offset base must be raw pointer, got {:?}", base.ty);
                };
                self.emit_ptr_offset(base_op, pointee_ty, mutability, index, expr.ty, expr.span)
            }
            HirExprKind::PtrDiff { lhs, rhs } => {
                let lhs_op = self.lower_expr_to_operand(lhs);
                let rhs_op = self.lower_expr_to_operand(rhs);
                let TyKind::RigidTy(RigidTy::RawPtr(pointee_ty, _)) = lhs.ty.kind() else {
                    panic!("ptr diff lhs must be raw pointer, got {:?}", lhs.ty);
                };
                let isize_ty = Ty::signed_ty(IntTy::Isize);
                let ret_local = self.new_temp(isize_ty, Mutability::Mut, expr.span);
                let offset_from = {
                    let deps: &WellknownDefs = &self.wellknown_defs;
                    deps.offset_from
                };
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
            HirExprKind::Comma { lhs, rhs } => {
                let _lhs = self.lower_expr_to_operand(lhs);

                self.lower_expr_to_operand(rhs)
            }
            HirExprKind::Binary { op, lhs, rhs } => {
                if matches!(
                    op,
                    co2_hir::HirBinOp::Eq
                        | co2_hir::HirBinOp::Lt
                        | co2_hir::HirBinOp::Le
                        | co2_hir::HirBinOp::Ne
                        | co2_hir::HirBinOp::Ge
                        | co2_hir::HirBinOp::Gt
                ) {
                    let normalize_cmp_operand = |expr: &HirExpr| {
                        if matches!(
                            expr.ty.kind(),
                            TyKind::RigidTy(
                                RigidTy::RawPtr(_, _) | RigidTy::FnPtr(_) | RigidTy::FnDef(_, _)
                            )
                        ) || maybe_uninit_fn_ptr_inner(expr.ty).is_some()
                        {
                            HirExpr {
                                kind: HirExprKind::Cast(Box::new(expr.clone())),
                                ty: Ty::usize_ty(),
                                span: expr.span,
                            }
                        } else {
                            expr.clone()
                        }
                    };
                    let lhs = self.lower_expr_to_operand(&normalize_cmp_operand(lhs));
                    let rhs = self.lower_expr_to_operand(&normalize_cmp_operand(rhs));
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
            HirExprKind::UnionAggregate { active_field, arg } => {
                let TyKind::RigidTy(RigidTy::Adt(adt, adt_args)) = expr.ty.kind() else {
                    panic!("union aggregate expects adt type, got {:?}", expr.ty);
                };
                let operand = self.lower_expr_to_operand(arg);
                let tmp = self.new_temp(expr.ty, Mutability::Mut, expr.span);
                self.stmts.push(MirStatement {
                    kind: MirStatementKind::Assign(
                        place(tmp),
                        Rvalue::Aggregate(
                            AggregateKind::Adt(
                                adt,
                                variant_idx(0),
                                adt_args,
                                None,
                                Some(*active_field),
                            ),
                            vec![operand],
                        ),
                    ),
                    span: expr.span,
                });
                MirOperand::Copy(place(tmp))
            }
            HirExprKind::ConstStr(s) => self.lower_const_string(s, expr.span),
            HirExprKind::Path(path) => match path {
                ResolvedValue::Fn(fn_def, generic_args) => {
                    let fn_ty = Ty::from_rigid_kind(RigidTy::FnDef(
                        *fn_def,
                        GenericArgs(generic_args.clone()),
                    ));
                    let c = MirConst::try_new_zero_sized(fn_ty).expect("failed to build fn const");
                    MirOperand::Constant(ConstOperand {
                        span: expr.span,
                        user_ty: None,
                        const_: c,
                    })
                }
                ResolvedValue::FnPtr(fn_def, generic_args) => {
                    let fn_ty = Ty::from_rigid_kind(RigidTy::FnDef(
                        *fn_def,
                        GenericArgs(generic_args.clone()),
                    ));
                    let fn_sig = fn_ty
                        .kind()
                        .fn_sig()
                        .expect("failed to get fn ptr signature");
                    let fn_ptr_ty = Ty::from_rigid_kind(RigidTy::FnPtr(fn_sig));
                    let fn_const =
                        MirConst::try_new_zero_sized(fn_ty).expect("failed to build fn const");
                    let fn_operand = MirOperand::Constant(ConstOperand {
                        span: expr.span,
                        user_ty: None,
                        const_: fn_const,
                    });
                    let tmp = self.new_temp(fn_ptr_ty, Mutability::Mut, expr.span);
                    self.stmts.push(MirStatement {
                        kind: MirStatementKind::Assign(
                            place(tmp),
                            Rvalue::Cast(
                                CastKind::PointerCoercion(PointerCoercion::ReifyFnPointer(
                                    Safety::Safe,
                                )),
                                fn_operand,
                                fn_ptr_ty,
                            ),
                        ),
                        span: expr.span,
                    });
                    MirOperand::Copy(place(tmp))
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
                    let src_ty = Ty::unsigned_ty(uint_ty);
                    if src_ty == expr.ty {
                        const_op
                    } else {
                        self.lower_cast(const_op, src_ty, expr.ty, expr.span)
                    }
                }
                ResolvedValue::Static(_) | ResolvedValue::StaticConst(_) => {
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
                if matches!(lhs.kind, HirExprKind::Bitfield { .. }) {
                    self.emit_bitfield_store(lhs, (**rhs).clone(), expr.span);
                    return self.lower_expr_to_operand(lhs);
                }
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
                binop_ty,
                return_semantic,
            } => {
                if matches!(lhs.kind, HirExprKind::Bitfield { .. }) {
                    let old_expr = if let HirExprKind::Bitfield {
                        base,
                        storage_index,
                        storage_ty,
                        bit_offset,
                        bit_width,
                        signed,
                    } = &lhs.kind
                    {
                        self.bitfield_read_expr(
                            base,
                            *storage_index,
                            *storage_ty,
                            *bit_offset,
                            *bit_width,
                            *signed,
                            lhs.ty,
                            lhs.span,
                        )
                    } else {
                        unreachable!()
                    };
                    let old_operand = matches!(return_semantic, ReturnSemantic::BeforeAssign)
                        .then(|| self.lower_expr_to_operand(&old_expr));
                    let bin_expr = HirExpr {
                        kind: HirExprKind::Binary {
                            op: *op,
                            lhs: Box::new(Self::emit_cast_expr(old_expr, *binop_ty)),
                            rhs: Box::new(Self::emit_cast_expr((**rhs).clone(), *binop_ty)),
                        },
                        ty: *binop_ty,
                        span: expr.span,
                    };
                    self.emit_bitfield_store(
                        lhs,
                        Self::emit_cast_expr(bin_expr, lhs.ty),
                        expr.span,
                    );
                    return match return_semantic {
                        ReturnSemantic::AfterAssign => self.lower_expr_to_operand(lhs),
                        ReturnSemantic::BeforeAssign => old_operand.unwrap(),
                    };
                }
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
                let new_val = self.new_temp(*binop_ty, Mutability::Mut, expr.span);
                let lhs_casted = self.lower_cast(
                    MirOperand::Copy(place(old_lhs)),
                    lhs.ty,
                    *binop_ty,
                    lhs.span,
                );
                self.stmts.push(MirStatement {
                    kind: MirStatementKind::Assign(
                        place(new_val),
                        Rvalue::BinaryOp(self.lower_bin_op(*op), lhs_casted, rhs_value),
                    ),
                    span: expr.span,
                });
                let new_val_casted = self.lower_cast(
                    MirOperand::Copy(place(new_val)),
                    *binop_ty,
                    lhs.ty,
                    lhs.span,
                );
                self.stmts.push(MirStatement {
                    kind: MirStatementKind::Assign(lhs_place.clone(), Rvalue::Use(new_val_casted)),
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
                let TyKind::RigidTy(RigidTy::RawPtr(pointee_ty, mutability)) = lhs.ty.kind() else {
                    panic!(
                        "ptr offset assignment lhs must be raw pointer, got {:?}",
                        lhs.ty
                    );
                };
                let new_ptr = self.emit_ptr_offset(
                    MirOperand::Copy(place(old_lhs)),
                    pointee_ty,
                    mutability,
                    rhs,
                    lhs.ty,
                    expr.span,
                );
                self.stmts.push(MirStatement {
                    kind: MirStatementKind::Assign(lhs_place.clone(), Rvalue::Use(new_ptr)),
                    span: expr.span,
                });
                match return_semantic {
                    ReturnSemantic::AfterAssign => MirOperand::Copy(lhs_place),
                    ReturnSemantic::BeforeAssign => MirOperand::Copy(place(old_lhs)),
                }
            }
            HirExprKind::AddrOf(inner) => {
                let TyKind::RigidTy(RigidTy::RawPtr(_, mutability)) = expr.ty.kind() else {
                    panic!(
                        "address-of expression must have raw pointer type, got {:?}",
                        expr.ty
                    );
                };
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
                            if mutability == Mutability::Mut {
                                RawPtrKind::Mut
                            } else {
                                RawPtrKind::Const
                            },
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
                self.lower_cast(inner_op, src_ty, dst_ty, expr.span)
            }
        }
    }

    fn lower_cast(
        &mut self,
        inner_op: MirOperand,
        src_ty: Ty,
        dst_ty: Ty,
        span: RustSpan,
    ) -> MirOperand {
        if src_ty == dst_ty {
            return inner_op;
        }
        let src_enum_payload = enum_payload_ty(src_ty);
        let dst_enum_payload = enum_payload_ty(dst_ty);
        if let Some(dst_payload_ty) = dst_enum_payload {
            let payload_op = if let Some(src_payload_ty) = src_enum_payload {
                let inner = self.read_enum_payload_operand(inner_op, src_ty, src_payload_ty, span);
                self.lower_cast(inner, src_payload_ty, dst_payload_ty, span)
            } else {
                self.lower_cast(inner_op, src_ty, dst_payload_ty, span)
            };
            return self.wrap_enum_payload_operand(payload_op, dst_payload_ty, dst_ty, span);
        }
        if let Some(src_payload_ty) = src_enum_payload {
            let inner = self.read_enum_payload_operand(inner_op, src_ty, src_payload_ty, span);
            return self.lower_cast(inner, src_payload_ty, dst_ty, span);
        }
        let src_is_bool = matches!(src_ty.kind(), TyKind::RigidTy(RigidTy::Bool));
        let dst_is_bool = matches!(dst_ty.kind(), TyKind::RigidTy(RigidTy::Bool));
        let src_is_int = matches!(
            src_ty.kind(),
            TyKind::RigidTy(RigidTy::Int(_) | RigidTy::Uint(_))
        );
        let dst_is_int = matches!(
            dst_ty.kind(),
            TyKind::RigidTy(RigidTy::Int(_) | RigidTy::Uint(_))
        );
        let src_is_float = matches!(src_ty.kind(), TyKind::RigidTy(RigidTy::Float(_)),);
        let dst_is_float = matches!(dst_ty.kind(), TyKind::RigidTy(RigidTy::Float(_)),);
        let src_is_ptr = matches!(src_ty.kind(), TyKind::RigidTy(RigidTy::RawPtr(_, _)));
        let dst_is_ptr = matches!(dst_ty.kind(), TyKind::RigidTy(RigidTy::RawPtr(_, _)));
        let src_is_ref = matches!(src_ty.kind(), TyKind::RigidTy(RigidTy::Ref(_, _, _)));
        let dst_is_ref = matches!(dst_ty.kind(), TyKind::RigidTy(RigidTy::Ref(_, _, _)));
        let src_is_fn_ptr = matches!(src_ty.kind(), TyKind::RigidTy(RigidTy::FnPtr(_)));
        let dst_is_fn_ptr = matches!(dst_ty.kind(), TyKind::RigidTy(RigidTy::FnPtr(_)));
        let src_is_fn_def = matches!(src_ty.kind(), TyKind::RigidTy(RigidTy::FnDef(_, _)));
        let src_mu_fn_ptr = maybe_uninit_fn_ptr_inner(src_ty);
        let dst_mu_fn_ptr = maybe_uninit_fn_ptr_inner(dst_ty);
        let dst_is_void =
            matches!(dst_ty.kind(), TyKind::RigidTy(RigidTy::Tuple(l)) if l.is_empty());
        if dst_is_void {
            let tmp = self.new_temp(dst_ty, Mutability::Mut, span);
            self.lower_zeroed_to_destination(place(tmp), span, dst_ty);
            return MirOperand::Copy(place(tmp));
        }
        if src_is_ref && dst_is_ptr {
            todo!()
        }
        if src_is_ptr && dst_is_ref {
            let TyKind::RigidTy(RigidTy::Ref(region, _, kind)) = dst_ty.kind() else {
                unreachable!();
            };
            let kind = match kind {
                Mutability::Not => BorrowKind::Shared,
                Mutability::Mut => BorrowKind::Mut {
                    kind: MutBorrowKind::Default,
                },
            };
            let tmp1 = self.new_temp(src_ty, Mutability::Mut, span);
            let tmp1_place = place(tmp1);
            let mut tmp1_deref = tmp1_place.clone();
            tmp1_deref.projection.push(MirProjection::Deref);
            self.stmts.push(MirStatement {
                kind: MirStatementKind::Assign(tmp1_place, Rvalue::Use(inner_op)),
                span,
            });
            let tmp2 = self.new_temp(dst_ty, Mutability::Mut, span);
            self.stmts.push(MirStatement {
                kind: MirStatementKind::Assign(place(tmp2), Rvalue::Ref(region, kind, tmp1_deref)),
                span,
            });
            return self.place_operand_for_ty(place(tmp2), dst_ty);
        }
        if dst_is_bool
            && (src_is_int
                || src_is_ptr
                || src_is_fn_ptr
                || src_is_fn_def
                || src_mu_fn_ptr.is_some())
        {
            let usize_ty = Ty::usize_ty();
            let cmp_op = if src_ty == usize_ty {
                inner_op
            } else {
                self.lower_cast(inner_op, src_ty, usize_ty, span)
            };
            let zero = MirOperand::Constant(ConstOperand {
                span,
                user_ty: None,
                const_: MirConst::try_from_uint(
                    0,
                    rustc_public_generative::rustc_public::ty::UintTy::Usize,
                )
                .expect("failed to build zero usize const"),
            });
            let bool_local = self.new_temp(Ty::bool_ty(), Mutability::Mut, span);
            self.stmts.push(MirStatement {
                kind: MirStatementKind::Assign(
                    place(bool_local),
                    Rvalue::BinaryOp(
                        rustc_public_generative::rustc_public::mir::BinOp::Ne,
                        cmp_op,
                        zero,
                    ),
                ),
                span,
            });
            return MirOperand::Copy(place(bool_local));
        }
        if src_is_int && dst_is_int {
            let tmp = self.new_temp(dst_ty, Mutability::Mut, span);
            self.stmts.push(MirStatement {
                kind: MirStatementKind::Assign(
                    place(tmp),
                    Rvalue::Cast(CastKind::IntToInt, inner_op, dst_ty),
                ),
                span,
            });
            return MirOperand::Copy(place(tmp));
        }
        if src_is_bool && dst_is_int {
            let tmp = self.new_temp(dst_ty, Mutability::Mut, span);
            self.stmts.push(MirStatement {
                kind: MirStatementKind::Assign(
                    place(tmp),
                    Rvalue::Cast(CastKind::IntToInt, inner_op, dst_ty),
                ),
                span,
            });
            return MirOperand::Copy(place(tmp));
        }
        if src_is_float && dst_is_int {
            let tmp = self.new_temp(dst_ty, Mutability::Mut, span);
            self.stmts.push(MirStatement {
                kind: MirStatementKind::Assign(
                    place(tmp),
                    Rvalue::Cast(CastKind::FloatToInt, inner_op, dst_ty),
                ),
                span,
            });
            return MirOperand::Copy(place(tmp));
        }
        if src_is_int && dst_is_float {
            let tmp = self.new_temp(dst_ty, Mutability::Mut, span);
            self.stmts.push(MirStatement {
                kind: MirStatementKind::Assign(
                    place(tmp),
                    Rvalue::Cast(CastKind::IntToFloat, inner_op, dst_ty),
                ),
                span,
            });
            return MirOperand::Copy(place(tmp));
        }
        if src_is_float && dst_is_float {
            let tmp = self.new_temp(dst_ty, Mutability::Mut, span);
            self.stmts.push(MirStatement {
                kind: MirStatementKind::Assign(
                    place(tmp),
                    Rvalue::Cast(CastKind::FloatToFloat, inner_op, dst_ty),
                ),
                span,
            });
            return MirOperand::Copy(place(tmp));
        }
        if src_is_fn_def && dst_is_fn_ptr {
            let src_sig = src_ty
                .kind()
                .fn_sig()
                .expect("fn def should have signature");
            let src_fn_ptr_ty = Ty::from_rigid_kind(RigidTy::FnPtr(src_sig));
            if !ty_matches_expected(dst_ty, src_fn_ptr_ty) {
                let fn_ptr_local = self.new_temp(src_fn_ptr_ty, Mutability::Mut, span);
                self.stmts.push(MirStatement {
                    kind: MirStatementKind::Assign(
                        place(fn_ptr_local),
                        Rvalue::Cast(
                            CastKind::PointerCoercion(PointerCoercion::ReifyFnPointer(
                                Safety::Safe,
                            )),
                            inner_op,
                            src_fn_ptr_ty,
                        ),
                    ),
                    span,
                });
                let dst_local = self.new_temp(dst_ty, Mutability::Mut, span);
                let generic_args = vec![
                    GenericArgKind::Type(src_fn_ptr_ty),
                    GenericArgKind::Type(dst_ty),
                ];
                self.emit_call_block(
                    fn_const_operand(self.wellknown_defs.transmute, generic_args, span),
                    vec![MirOperand::Copy(place(fn_ptr_local))],
                    place(dst_local),
                    span,
                );
                return MirOperand::Copy(place(dst_local));
            }
        }
        if src_is_fn_def && dst_is_fn_ptr {
            let tmp = self.new_temp(dst_ty, Mutability::Mut, span);
            self.stmts.push(MirStatement {
                kind: MirStatementKind::Assign(
                    place(tmp),
                    Rvalue::Cast(
                        CastKind::PointerCoercion(PointerCoercion::ReifyFnPointer(Safety::Safe)),
                        inner_op,
                        dst_ty,
                    ),
                ),
                span,
            });
            return MirOperand::Copy(place(tmp));
        }
        if let Some(fn_ptr_ty) = dst_mu_fn_ptr.filter(|_| src_is_fn_def) {
            let src_sig = src_ty
                .kind()
                .fn_sig()
                .expect("fn def should have signature");
            let src_fn_ptr_ty = Ty::from_rigid_kind(RigidTy::FnPtr(src_sig));
            let src_fn_ptr_local = self.new_temp(src_fn_ptr_ty, Mutability::Mut, span);
            self.stmts.push(MirStatement {
                kind: MirStatementKind::Assign(
                    place(src_fn_ptr_local),
                    Rvalue::Cast(
                        CastKind::PointerCoercion(PointerCoercion::ReifyFnPointer(Safety::Safe)),
                        inner_op,
                        src_fn_ptr_ty,
                    ),
                ),
                span,
            });
            let dst_fn_ptr_local = self.new_temp(fn_ptr_ty, Mutability::Mut, span);
            let generic_args = vec![
                GenericArgKind::Type(src_fn_ptr_ty),
                GenericArgKind::Type(fn_ptr_ty),
            ];
            self.emit_call_block(
                fn_const_operand(self.wellknown_defs.transmute, generic_args, span),
                vec![MirOperand::Copy(place(src_fn_ptr_local))],
                place(dst_fn_ptr_local),
                span,
            );
            return self.write_value_into_maybe_uninit_storage(
                dst_ty,
                MirOperand::Copy(place(dst_fn_ptr_local)),
                fn_ptr_ty,
                span,
            );
        }
        if let Some(fn_ptr_ty) = dst_mu_fn_ptr.filter(|_| src_is_fn_def) {
            let fn_ptr_local = self.new_temp(fn_ptr_ty, Mutability::Mut, span);
            self.stmts.push(MirStatement {
                kind: MirStatementKind::Assign(
                    place(fn_ptr_local),
                    Rvalue::Cast(
                        CastKind::PointerCoercion(PointerCoercion::ReifyFnPointer(Safety::Safe)),
                        inner_op,
                        fn_ptr_ty,
                    ),
                ),
                span,
            });
            return self.write_value_into_maybe_uninit_storage(
                dst_ty,
                MirOperand::Copy(place(fn_ptr_local)),
                fn_ptr_ty,
                span,
            );
        }
        if dst_is_ptr && src_is_fn_ptr {
            let tmp = self.new_temp(dst_ty, Mutability::Mut, span);
            self.stmts.push(MirStatement {
                kind: MirStatementKind::Assign(
                    place(tmp),
                    Rvalue::Cast(CastKind::FnPtrToPtr, inner_op, dst_ty),
                ),
                span,
            });
            return MirOperand::Copy(place(tmp));
        }
        if src_mu_fn_ptr.is_some() && dst_is_ptr {
            return self.read_maybe_uninit_as(inner_op, src_ty, dst_ty, span);
        }
        if dst_mu_fn_ptr.is_some() && src_is_fn_ptr {
            return self.write_value_into_maybe_uninit_storage(dst_ty, inner_op, src_ty, span);
        }
        if src_is_ptr && dst_is_ptr {
            let tmp = self.new_temp(dst_ty, Mutability::Mut, span);
            self.stmts.push(MirStatement {
                kind: MirStatementKind::Assign(
                    place(tmp),
                    Rvalue::Cast(CastKind::PtrToPtr, inner_op, dst_ty),
                ),
                span,
            });
            return MirOperand::Copy(place(tmp));
        }
        if (src_is_fn_def || src_is_fn_ptr) && dst_is_int {
            let fn_ptr_ty = if src_is_fn_def {
                let src_sig = src_ty
                    .kind()
                    .fn_sig()
                    .expect("fn def should have signature");
                Ty::from_rigid_kind(RigidTy::FnPtr(src_sig))
            } else {
                src_ty
            };
            let fn_ptr_op = if src_is_fn_def {
                let fn_ptr_local = self.new_temp(fn_ptr_ty, Mutability::Mut, span);
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
                    span,
                });
                MirOperand::Copy(place(fn_ptr_local))
            } else {
                inner_op
            };
            let raw_ptr_ty = Ty::new_ptr(Ty::signed_ty(IntTy::I8), Mutability::Not);
            let raw_ptr_local = self.new_temp(raw_ptr_ty, Mutability::Mut, span);
            self.stmts.push(MirStatement {
                kind: MirStatementKind::Assign(
                    place(raw_ptr_local),
                    Rvalue::Cast(CastKind::FnPtrToPtr, fn_ptr_op, raw_ptr_ty),
                ),
                span,
            });
            let usize_ty = Ty::usize_ty();
            let usize_tmp = self.new_temp(usize_ty, Mutability::Mut, span);
            self.stmts.push(MirStatement {
                kind: MirStatementKind::Assign(
                    place(usize_tmp),
                    Rvalue::Cast(
                        CastKind::PointerExposeAddress,
                        MirOperand::Copy(place(raw_ptr_local)),
                        usize_ty,
                    ),
                ),
                span,
            });
            if dst_ty == usize_ty {
                return MirOperand::Copy(place(usize_tmp));
            }
            let dst_tmp = self.new_temp(dst_ty, Mutability::Mut, span);
            self.stmts.push(MirStatement {
                kind: MirStatementKind::Assign(
                    place(dst_tmp),
                    Rvalue::Cast(
                        CastKind::IntToInt,
                        MirOperand::Copy(place(usize_tmp)),
                        dst_ty,
                    ),
                ),
                span,
            });
            return MirOperand::Copy(place(dst_tmp));
        }
        if src_is_ptr && dst_is_int {
            let usize_ty = Ty::usize_ty();
            let usize_tmp = self.new_temp(usize_ty, Mutability::Mut, span);
            self.stmts.push(MirStatement {
                kind: MirStatementKind::Assign(
                    place(usize_tmp),
                    Rvalue::Cast(CastKind::PointerExposeAddress, inner_op, usize_ty),
                ),
                span,
            });
            if dst_ty == usize_ty {
                return MirOperand::Copy(place(usize_tmp));
            }
            let dst_tmp = self.new_temp(dst_ty, Mutability::Mut, span);
            self.stmts.push(MirStatement {
                kind: MirStatementKind::Assign(
                    place(dst_tmp),
                    Rvalue::Cast(
                        CastKind::IntToInt,
                        MirOperand::Copy(place(usize_tmp)),
                        dst_ty,
                    ),
                ),
                span,
            });
            return MirOperand::Copy(place(dst_tmp));
        }
        if src_mu_fn_ptr.is_some() && dst_is_int {
            let usize_ty = Ty::usize_ty();
            let usize_op = self.read_maybe_uninit_as(inner_op, src_ty, usize_ty, span);
            if dst_ty == usize_ty {
                return usize_op;
            }
            let dst_tmp = self.new_temp(dst_ty, Mutability::Mut, span);
            self.stmts.push(MirStatement {
                kind: MirStatementKind::Assign(
                    place(dst_tmp),
                    Rvalue::Cast(CastKind::IntToInt, usize_op, dst_ty),
                ),
                span,
            });
            return MirOperand::Copy(place(dst_tmp));
        }
        if src_is_int && dst_is_ptr {
            let usize_ty = Ty::usize_ty();
            let usize_op = if src_ty == usize_ty {
                inner_op
            } else {
                let usize_tmp = self.new_temp(usize_ty, Mutability::Mut, span);
                self.stmts.push(MirStatement {
                    kind: MirStatementKind::Assign(
                        place(usize_tmp),
                        Rvalue::Cast(CastKind::IntToInt, inner_op, usize_ty),
                    ),
                    span,
                });
                MirOperand::Copy(place(usize_tmp))
            };
            let dst_tmp = self.new_temp(dst_ty, Mutability::Mut, span);
            self.stmts.push(MirStatement {
                kind: MirStatementKind::Assign(
                    place(dst_tmp),
                    Rvalue::Cast(CastKind::PointerWithExposedProvenance, usize_op, dst_ty),
                ),
                span,
            });
            return MirOperand::Copy(place(dst_tmp));
        }
        if src_is_ptr && dst_mu_fn_ptr.is_some() {
            return self.write_value_into_maybe_uninit_storage(dst_ty, inner_op, src_ty, span);
        }
        if src_is_int && dst_mu_fn_ptr.is_some() {
            let usize_ty = Ty::usize_ty();
            let usize_op = if src_ty == usize_ty {
                inner_op
            } else {
                let usize_tmp = self.new_temp(usize_ty, Mutability::Mut, span);
                let cast_kind = CastKind::IntToInt;
                self.stmts.push(MirStatement {
                    kind: MirStatementKind::Assign(
                        place(usize_tmp),
                        Rvalue::Cast(cast_kind, inner_op, usize_ty),
                    ),
                    span,
                });
                MirOperand::Copy(place(usize_tmp))
            };
            return self.write_value_into_maybe_uninit_storage(dst_ty, usize_op, usize_ty, span);
        }
        if src_mu_fn_ptr.is_some() && dst_is_fn_ptr {
            return self.read_maybe_uninit_as(inner_op, src_ty, dst_ty, span);
        }
        if src_mu_fn_ptr.is_some() && dst_mu_fn_ptr.is_some() {
            return self.write_value_into_maybe_uninit_storage(dst_ty, inner_op, src_ty, span);
        }
        panic!("unsupported cast from {src_ty:?} to {dst_ty:?}");
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
        MirOperand::Move(place(ret_local))
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

        debug_assert!(matches!(lhs.ty.kind(), TyKind::RigidTy(RigidTy::Bool)));
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
        debug_assert!(matches!(rhs.ty.kind(), TyKind::RigidTy(RigidTy::Bool)));
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
        debug_assert!(matches!(inner.ty.kind(), TyKind::RigidTy(RigidTy::Bool)));
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
        debug_assert!(matches!(cond.ty.kind(), TyKind::RigidTy(RigidTy::Bool)));
        let cond_op = self.lower_expr_to_operand(cond);
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
        let sig = callable_sig(self.ctx.normalize_ty_defaults(func.ty))
            .expect("call target has no fn signature");
        let sig = rustc_public_generative::erase_late_bound_regions_in_fn_sig(sig);

        let mut arg_ops = Vec::with_capacity(args.len());
        for (idx, arg) in args.iter().enumerate() {
            let op = if let Some(expected_ty) = sig.inputs().get(idx) {
                self.lower_call_arg(arg, *expected_ty)
            } else {
                self.lower_expr_to_operand(arg)
            };
            arg_ops.push(op);
        }
        if let HirExprKind::Path(ResolvedValue::Fn(fn_def, existing_generic_args)) = &func.kind {
            let generic_args =
                complete_fn_generic_args(*fn_def, &sig, args, ret_ty, existing_generic_args)
                    .into_iter()
                    .map(|arg| match arg {
                        GenericArgKind::Type(ty) => {
                            GenericArgKind::Type(self.ctx.normalize_ty_defaults(ty))
                        }
                        _ => arg,
                    })
                    .collect();
            self.emit_call_block(
                fn_const_operand(*fn_def, generic_args, span),
                arg_ops,
                destination,
                span,
            );
        } else {
            let func_op = if let Some(inner_fn_ptr) = maybe_uninit_fn_ptr_inner(func.ty) {
                let op = self.lower_expr_to_operand(func);
                self.read_maybe_uninit_as(op, func.ty, inner_fn_ptr, span)
            } else {
                self.lower_expr_to_operand(func)
            };
            self.emit_call_block(func_op, arg_ops, destination, span);
        }
    }

    pub(crate) fn lower_zeroed_to_destination(
        &mut self,
        destination: rustc_public_generative::rustc_public::mir::Place,
        span: RustSpan,
        ret_ty: Ty,
    ) {
        let zeroed_fn = self.wellknown_defs.zeroed;
        let sig = zeroed_fn
            .ty()
            .kind()
            .fn_sig()
            .expect("std::mem::zeroed has no signature")
            .skip_binder();
        let generic_args = infer_fn_generic_args(zeroed_fn, &sig, &[], ret_ty);
        self.emit_call_block(
            fn_const_operand(zeroed_fn, generic_args, span),
            vec![],
            destination,
            span,
        );
    }

    pub(crate) fn lower_call_arg(&mut self, arg: &HirExpr, expected_ty: Ty) -> MirOperand {
        if let TyKind::RigidTy(RigidTy::Adt(adt, _)) = expected_ty.kind()
            && adt == self.wellknown_defs.valist
        {
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

            let reg = Region {
                kind: RegionKind::ReErased,
            };
            let ref_ty = Ty::new_ref(reg.clone(), arg.ty, Mutability::Not);
            let ref_local = self.new_temp(ref_ty, Mutability::Not, arg.span);
            self.stmts.push(MirStatement {
                kind: MirStatementKind::Assign(
                    place(ref_local),
                    Rvalue::Ref(reg, BorrowKind::Shared, borrowed_place),
                ),
                span: arg.span,
            });

            let transmute_copy_fn = self.wellknown_defs.transmute_copy;
            let generic_args = vec![
                GenericArgKind::Type(arg.ty),
                GenericArgKind::Type(expected_ty),
            ];
            let tmp = self.new_temp(expected_ty, Mutability::Mut, arg.span);
            self.emit_call_block(
                fn_const_operand(transmute_copy_fn, generic_args, arg.span),
                vec![MirOperand::Copy(place(ref_local))],
                place(tmp),
                arg.span,
            );
            return MirOperand::Move(place(tmp));
        }

        self.lower_expr_to_operand(arg)
    }
}
