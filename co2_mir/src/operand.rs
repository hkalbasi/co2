use co2_hir::{
    HirExpr, HirExprKind, HirLogicalOp, ResolvedValue, ReturnSemantic, WellknownDefs,
    enum_payload_ty,
};
use rustc_public_generative::{
    DependencyConstValue,
    rustc_public::{
        CrateDefType,
        mir::{
            AggregateKind, BorrowKind, CastKind, ConstOperand, MutBorrowKind, Mutability,
            Operand as MirOperand, PointerCoercion, ProjectionElem as MirProjection, RawPtrKind,
            Rvalue, Safety, SourceInfo, StatementKind as MirStatementKind, SwitchTargets,
            Terminator as MirTerminator, TerminatorKind, WithRetag,
        },
        ty::{
            FloatTy, GenericArgKind, GenericArgs, IntTy, MirConst, Region, RegionKind, RigidTy,
            Span as RustSpan, Ty, TyKind, UintTy,
        },
    },
};

use crate::{
    build::{
        Builder, complete_fn_generic_args, fn_const_operand, ty_matches_expected, variant_idx,
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

fn ptr_offset_generic_args(func_ty: Ty, pointee_ty: Ty) -> Vec<GenericArgKind> {
    match func_ty.kind() {
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
    }
}

pub(crate) fn maybe_uninit_fn_ptr_inner(ty: Ty) -> Option<Ty> {
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

/// Cast pointer-like values (raw/fn pointers, fn items, and C function
/// pointers which allow null) to `usize` so they can be compared or used
/// as a `SwitchInt` discriminant. Other types pass through unchanged.
pub(crate) fn ptr_like_to_usize_expr(expr: &HirExpr) -> HirExpr {
    if matches!(
        expr.ty.kind(),
        TyKind::RigidTy(RigidTy::RawPtr(_, _) | RigidTy::FnPtr(_) | RigidTy::FnDef(_, _))
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
        self.emit_assign_use(place(tmp), enum_op, span);
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
        self.push_statement(
            MirStatementKind::Assign(
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
        );
        MirOperand::Copy(place(tmp))
    }

    fn bitfield_mask_expr(&self, width: usize, storage_ty: Ty, span: RustSpan) -> HirExpr {
        if width >= 128 {
            Self::emit_cast_expr(
                Self::const_int_expr(-1, Ty::signed_ty(IntTy::I128), span),
                storage_ty,
            )
        } else {
            let mask_ty = if width >= 64 {
                Ty::signed_ty(IntTy::I128)
            } else if width >= 32 {
                Ty::signed_ty(IntTy::I64)
            } else {
                Ty::signed_ty(IntTy::I32)
            };
            Self::emit_cast_expr(
                Self::const_int_expr(((1u128 << width) - 1) as i128, mask_ty, span),
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
        let is_void = matches!(
            pointee_ty.kind(),
            TyKind::RigidTy(RigidTy::Tuple(l)) if l.is_empty()
        );
        let mut base_op = base_op;
        let mut pointee_ty = pointee_ty;
        let mut out_ty_inner = out_ty;
        if is_void {
            let byte_ty = Ty::from_rigid_kind(RigidTy::Uint(UintTy::U8));
            let src_ptr_ty = Ty::new_ptr(pointee_ty, ptr_mutability);
            let byte_ptr_ty = Ty::new_ptr(byte_ty, ptr_mutability);
            base_op = self.lower_cast(base_op, src_ptr_ty, byte_ptr_ty, span);
            pointee_ty = byte_ty;
            out_ty_inner = byte_ptr_ty;
        }
        let isize_ty = Ty::signed_ty(IntTy::Isize);
        let idx_ty = index.ty;
        let idx_op = self.lower_expr_to_operand(index);
        let idx_op = self.lower_cast(idx_op, idx_ty, isize_ty, span);

        let offset = find_ptr_offset_fn(&self.wellknown_defs, ptr_mutability);
        let generic_args = ptr_offset_generic_args(offset.ty(), pointee_ty);

        let ret_local = self.new_temp(out_ty_inner, Mutability::Mut, span);
        self.emit_call_block(
            fn_const_operand(offset, generic_args, span),
            vec![base_op, idx_op],
            place(ret_local),
            span,
        );
        let result = MirOperand::Copy(place(ret_local));
        if is_void && out_ty_inner != out_ty {
            self.lower_cast(result, out_ty_inner, out_ty, span)
        } else {
            result
        }
    }

    fn reinterpret_place(
        &mut self,
        src_place: rustc_public_generative::rustc_public::mir::Place,
        src_ty: Ty,
        dst_ty: Ty,
        span: RustSpan,
    ) -> rustc_public_generative::rustc_public::mir::Place {
        let ptr_src_ty = Ty::new_ptr(src_ty, Mutability::Mut);
        let ptr_src_local = self.new_temp(ptr_src_ty, Mutability::Mut, span);
        self.push_statement(
            MirStatementKind::Assign(
                place(ptr_src_local),
                Rvalue::AddressOf(RawPtrKind::Mut, src_place),
            ),
            span,
        );
        let ptr_dst_ty = Ty::new_ptr(dst_ty, Mutability::Mut);
        let ptr_dst_local = self.new_temp(ptr_dst_ty, Mutability::Mut, span);
        self.push_statement(
            MirStatementKind::Assign(
                place(ptr_dst_local),
                Rvalue::Cast(
                    CastKind::PtrToPtr,
                    MirOperand::Copy(place(ptr_src_local)),
                    ptr_dst_ty,
                ),
            ),
            span,
        );
        rustc_public_generative::rustc_public::mir::Place {
            local: ptr_dst_local,
            projection: vec![MirProjection::Deref],
        }
    }

    fn write_value_into_maybe_uninit_storage(
        &mut self,
        dst_maybe_ty: Ty,
        value_op: MirOperand,
        value_ty: Ty,
        span: RustSpan,
    ) -> MirOperand {
        let dst_local = self.new_temp(dst_maybe_ty, Mutability::Mut, span);
        let value_place = self.reinterpret_place(place(dst_local), dst_maybe_ty, value_ty, span);
        self.emit_assign_use(value_place, value_op, span);
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
            self.emit_assign_use(place(tmp), op, span);
            place(tmp)
        };
        let value_place = self.reinterpret_place(src_place, op_ty, value_ty, span);
        let out_local = self.new_temp(value_ty, Mutability::Mut, span);
        self.emit_assign_use(place(out_local), MirOperand::Copy(value_place), span);
        MirOperand::Copy(place(out_local))
    }

    fn lower_va_copy(
        &mut self,
        src: rustc_public_generative::rustc_public::mir::Place,
        src_ty: Ty,
        dst: rustc_public_generative::rustc_public::mir::Place,
        dst_ty: Ty,
        result_ty: Ty,
        span: RustSpan,
    ) -> MirOperand {
        let reg = Region {
            kind: RegionKind::ReErased,
        };
        let src_ref_ty = Ty::new_ref(reg.clone(), src_ty, Mutability::Not);
        let src_ref_local = self.new_temp(src_ref_ty, Mutability::Not, span);
        self.push_statement(
            MirStatementKind::Assign(
                place(src_ref_local),
                Rvalue::Ref(reg, BorrowKind::Shared, src),
            ),
            span,
        );

        let clone_local = self.new_temp(src_ty, Mutability::Mut, span);
        self.emit_call_block(
            fn_const_operand(
                self.wellknown_defs.clone,
                vec![GenericArgKind::Type(src_ty)],
                span,
            ),
            vec![MirOperand::Copy(place(src_ref_local))],
            place(clone_local),
            span,
        );

        let generic_args = vec![GenericArgKind::Type(src_ty), GenericArgKind::Type(dst_ty)];
        self.emit_call_block(
            fn_const_operand(self.wellknown_defs.transmute, generic_args, span),
            vec![MirOperand::Move(place(clone_local))],
            dst,
            span,
        );
        self.zeroed_operand(result_ty, span)
    }

    fn zeroed_operand(&mut self, ty: Ty, span: RustSpan) -> MirOperand {
        let temp = self.new_temp(ty, Mutability::Mut, span);
        self.lower_zeroed_to_destination(place(temp), span, ty);
        MirOperand::Copy(place(temp))
    }

    pub(crate) fn lower_expr_to_operand(&mut self, expr: &HirExpr) -> MirOperand {
        match &expr.kind {
            HirExprKind::ArrayToPointer(inner) => {
                let base_place = self.lower_expr_to_place_or_temp(inner);
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
                self.push_statement(
                    MirStatementKind::Assign(
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
                    expr.span,
                );
                let ptr_local = self.new_temp(expr.ty, Mutability::Mut, expr.span);
                self.push_statement(
                    MirStatementKind::Assign(
                        place(ptr_local),
                        Rvalue::Cast(
                            CastKind::PtrToPtr,
                            MirOperand::Copy(place(array_ptr_local)),
                            expr.ty,
                        ),
                    ),
                    expr.span,
                );
                MirOperand::Copy(place(ptr_local))
            }
            HirExprKind::VaStart(args) => {
                let args_ty = args.ty;
                let Some(args) = self.lower_expr_to_place(args) else {
                    panic!("VaStart operand was not lvalue");
                };
                let src_local = self.c_variadic_local.unwrap();
                let src_ty = self.locals[src_local].ty;
                self.lower_va_copy(place(src_local), src_ty, args, args_ty, expr.ty, expr.span)
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
                    self.push_statement(
                        MirStatementKind::Assign(
                            place(tmp),
                            Rvalue::Ref(
                                reg.clone(),
                                BorrowKind::Mut {
                                    kind: MutBorrowKind::Default,
                                },
                                args,
                            ),
                        ),
                        expr.span,
                    );
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

                self.lower_va_copy(src, src_ty, dest, dest_ty, expr.ty, expr.span)
            }
            HirExprKind::VaEnd(args) => {
                let Some(_args) = self.lower_expr_to_place(args) else {
                    panic!("VaEnd operand was not lvalue");
                };
                self.zeroed_operand(expr.ty, expr.span)
            }

            HirExprKind::Zeroed => self.zeroed_operand(expr.ty, expr.span),
            HirExprKind::Local(local) | HirExprKind::LocalConst(local) => {
                let local_index = self.local_to_index(*local);
                self.place_operand_for_ty(place(local_index), self.locals[local_index].ty)
            }
            HirExprKind::LabelAddress(label) => {
                let discr = match self.label_discriminants.get(label) {
                    Some(discr) => *discr,
                    None => self.terminate_with_error(
                        expr.span,
                        "label is not a valid target of a computed goto",
                    ),
                };
                self.lower_expr_to_operand(&HirExpr {
                    kind: HirExprKind::ConstInt(discr as i128),
                    ty: expr.ty,
                    span: expr.span,
                })
            }
            HirExprKind::ConstInt(v) => self.make_int_const(*v, expr.ty, expr.span),
            HirExprKind::ConstFloat(v) => {
                let TyKind::RigidTy(RigidTy::Float(_)) = expr.ty.kind() else {
                    panic!("float const must have float type, got {:?}", expr.ty);
                };
                self.make_float_const(*v, expr.ty, expr.span)
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
                let TyKind::RigidTy(RigidTy::RawPtr(mut pointee_ty, _)) = lhs.ty.kind() else {
                    panic!("ptr diff lhs must be raw pointer, got {:?}", lhs.ty);
                };
                if matches!(
                    pointee_ty.kind(),
                    TyKind::RigidTy(RigidTy::Tuple(l)) if l.is_empty()
                ) {
                    pointee_ty = Ty::from_rigid_kind(RigidTy::Uint(UintTy::U8));
                }
                let isize_ty = Ty::signed_ty(IntTy::Isize);
                let ret_local = self.new_temp(isize_ty, Mutability::Mut, expr.span);
                let offset_from = {
                    let deps: &WellknownDefs = &self.wellknown_defs;
                    deps.offset_from
                };
                let const_ptr_ty = Ty::new_ptr(pointee_ty, Mutability::Not);
                let lhs_cast = self.new_temp(const_ptr_ty, Mutability::Mut, expr.span);
                self.push_statement(
                    MirStatementKind::Assign(
                        place(lhs_cast),
                        Rvalue::Cast(CastKind::PtrToPtr, lhs_op, const_ptr_ty),
                    ),
                    expr.span,
                );
                let rhs_cast = self.new_temp(const_ptr_ty, Mutability::Mut, expr.span);
                self.push_statement(
                    MirStatementKind::Assign(
                        place(rhs_cast),
                        Rvalue::Cast(CastKind::PtrToPtr, rhs_op, const_ptr_ty),
                    ),
                    expr.span,
                );
                let generic_args = ptr_offset_generic_args(offset_from.ty(), pointee_ty);
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
                    let lhs = self.lower_expr_to_operand(&ptr_like_to_usize_expr(lhs));
                    let rhs = self.lower_expr_to_operand(&ptr_like_to_usize_expr(rhs));
                    let bool_local = self.new_temp(Ty::bool_ty(), Mutability::Mut, expr.span);
                    self.push_statement(
                        MirStatementKind::Assign(
                            place(bool_local),
                            Rvalue::BinaryOp(self.lower_bin_op(*op), lhs, rhs),
                        ),
                        expr.span,
                    );

                    let tmp = self.new_temp(expr.ty, Mutability::Mut, expr.span);
                    self.push_statement(
                        MirStatementKind::Assign(
                            place(tmp),
                            Rvalue::Cast(
                                CastKind::IntToInt,
                                MirOperand::Copy(place(bool_local)),
                                expr.ty,
                            ),
                        ),
                        expr.span,
                    );
                    return MirOperand::Copy(place(tmp));
                }
                let lhs = self.lower_expr_to_operand(lhs);
                let rhs = self.lower_expr_to_operand(rhs);
                let tmp = self.new_temp(expr.ty, Mutability::Mut, expr.span);
                self.push_statement(
                    MirStatementKind::Assign(
                        place(tmp),
                        Rvalue::BinaryOp(self.lower_bin_op(*op), lhs, rhs),
                    ),
                    expr.span,
                );
                MirOperand::Copy(place(tmp))
            }
            HirExprKind::Logical { op, lhs, rhs } => {
                self.lower_logical_expr(*op, lhs, rhs, expr.span, expr.ty)
            }
            HirExprKind::Conditional {
                cond,
                then_expr,
                else_expr,
            } => self.lower_conditional_expr(
                cond,
                then_expr.as_deref(),
                else_expr,
                expr.span,
                expr.ty,
            ),
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
                let mut inner_op = self.lower_expr_to_operand(inner);
                let enum_ty = enum_payload_ty(expr.ty);
                let mut tmp_ty = expr.ty;
                if let Some(payload_ty) = enum_ty {
                    inner_op =
                        self.read_enum_payload_operand(inner_op, expr.ty, payload_ty, expr.span);
                    tmp_ty = payload_ty;
                }
                let tmp = self.new_temp(tmp_ty, Mutability::Mut, expr.span);
                self.push_statement(
                    MirStatementKind::Assign(
                        place(tmp),
                        Rvalue::UnaryOp(
                            rustc_public_generative::rustc_public::mir::UnOp::Not,
                            inner_op,
                        ),
                    ),
                    expr.span,
                );
                let mut result = MirOperand::Copy(place(tmp));

                if let Some(payload_ty) = enum_ty {
                    result = self.wrap_enum_payload_operand(result, payload_ty, expr.ty, expr.span);
                }

                result
            }
            HirExprKind::Aggregate { args } => match expr.ty.kind() {
                TyKind::RigidTy(RigidTy::Adt(adt, adt_args)) => {
                    let mut operands = Vec::with_capacity(args.len());
                    for arg in args {
                        operands.push(self.lower_expr_to_operand(arg));
                    }
                    let tmp = self.new_temp(expr.ty, Mutability::Mut, expr.span);
                    self.push_statement(
                        MirStatementKind::Assign(
                            place(tmp),
                            Rvalue::Aggregate(
                                AggregateKind::Adt(adt, variant_idx(0), adt_args, None, None),
                                operands,
                            ),
                        ),
                        expr.span,
                    );
                    MirOperand::Copy(place(tmp))
                }
                TyKind::RigidTy(RigidTy::Array(elem, _)) => {
                    let mut operands = Vec::with_capacity(args.len());
                    for arg in args {
                        operands.push(self.lower_expr_to_operand(arg));
                    }
                    let tmp = self.new_temp(expr.ty, Mutability::Mut, expr.span);
                    self.push_statement(
                        MirStatementKind::Assign(
                            place(tmp),
                            Rvalue::Aggregate(AggregateKind::Array(elem), operands),
                        ),
                        expr.span,
                    );
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
                self.push_statement(
                    MirStatementKind::Assign(
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
                    expr.span,
                );
                MirOperand::Copy(place(tmp))
            }
            HirExprKind::ConstStr(s) => self.lower_const_string(s, expr.ty, expr.span),
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
                    self.push_statement(
                        MirStatementKind::Assign(
                            place(tmp),
                            Rvalue::Cast(
                                CastKind::PointerCoercion(PointerCoercion::ReifyFnPointer(
                                    Safety::Safe,
                                )),
                                fn_operand,
                                fn_ptr_ty,
                            ),
                        ),
                        expr.span,
                    );
                    MirOperand::Copy(place(tmp))
                }
                ResolvedValue::ConstInt(v) => self.make_int_const(*v, expr.ty, expr.span),
                ResolvedValue::Static(def) | ResolvedValue::StaticConst(def) => {
                    if let Some(const_value) = self.ctx.dependency_const_value(*def) {
                        return self.lower_dependency_const_value(const_value, expr.ty, expr.span);
                    }
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
                self.emit_assign_use(lhs_place.clone(), rhs_value, expr.span);
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
                self.emit_assign_use(
                    place(old_lhs),
                    MirOperand::Copy(lhs_place.clone()),
                    expr.span,
                );
                let new_val = self.new_temp(*binop_ty, Mutability::Mut, expr.span);
                let lhs_casted = self.lower_cast(
                    MirOperand::Copy(place(old_lhs)),
                    lhs.ty,
                    *binop_ty,
                    lhs.span,
                );
                self.push_statement(
                    MirStatementKind::Assign(
                        place(new_val),
                        Rvalue::BinaryOp(self.lower_bin_op(*op), lhs_casted, rhs_value),
                    ),
                    expr.span,
                );
                let new_val_casted = self.lower_cast(
                    MirOperand::Copy(place(new_val)),
                    *binop_ty,
                    lhs.ty,
                    lhs.span,
                );
                self.emit_assign_use(lhs_place.clone(), new_val_casted, expr.span);
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
                self.emit_assign_use(
                    place(old_lhs),
                    MirOperand::Copy(lhs_place.clone()),
                    expr.span,
                );
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
                self.emit_assign_use(lhs_place.clone(), new_ptr, expr.span);
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
                let target_place = self.lower_expr_to_place_or_temp(inner);
                let tmp = self.new_temp(expr.ty, Mutability::Mut, expr.span);
                self.push_statement(
                    MirStatementKind::Assign(
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
                    expr.span,
                );
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
        if matches!(src_ty.kind(), TyKind::RigidTy(RigidTy::Never)) {
            let tmp = self.new_temp(dst_ty, Mutability::Mut, span);
            return MirOperand::Copy(place(tmp));
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
            return self.zeroed_operand(dst_ty, span);
        }
        if src_is_ref && (dst_is_ptr || dst_is_ref) {
            let tmp = self.new_temp(dst_ty, Mutability::Mut, span);
            self.push_statement(
                MirStatementKind::Assign(
                    place(tmp),
                    Rvalue::Cast(CastKind::Transmute, inner_op, dst_ty),
                ),
                span,
            );
            return self.place_operand_for_ty(place(tmp), dst_ty);
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
            self.emit_assign_use(tmp1_place, inner_op, span);
            let tmp2 = self.new_temp(dst_ty, Mutability::Mut, span);
            self.push_statement(
                MirStatementKind::Assign(place(tmp2), Rvalue::Ref(region, kind, tmp1_deref)),
                span,
            );
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
            self.push_statement(
                MirStatementKind::Assign(
                    place(bool_local),
                    Rvalue::BinaryOp(
                        rustc_public_generative::rustc_public::mir::BinOp::Ne,
                        cmp_op,
                        zero,
                    ),
                ),
                span,
            );
            return MirOperand::Copy(place(bool_local));
        }
        if dst_is_bool && src_is_float {
            let TyKind::RigidTy(RigidTy::Float(float_ty)) = src_ty.kind() else {
                unreachable!("src_is_float implies float type");
            };
            let zero = MirOperand::Constant(ConstOperand {
                span,
                user_ty: None,
                const_: MirConst::try_from_float(0, float_ty).expect("failed to build float zero"),
            });
            let bool_local = self.new_temp(Ty::bool_ty(), Mutability::Mut, span);
            self.push_statement(
                MirStatementKind::Assign(
                    place(bool_local),
                    Rvalue::BinaryOp(
                        rustc_public_generative::rustc_public::mir::BinOp::Ne,
                        inner_op,
                        zero,
                    ),
                ),
                span,
            );
            return MirOperand::Copy(place(bool_local));
        }
        if src_is_int && dst_is_int {
            return self.emit_cast_copy(CastKind::IntToInt, inner_op, dst_ty, span);
        }
        if src_is_bool && dst_is_int {
            return self.emit_cast_copy(CastKind::IntToInt, inner_op, dst_ty, span);
        }
        if src_is_float && dst_is_int {
            return self.emit_cast_copy(CastKind::FloatToInt, inner_op, dst_ty, span);
        }
        if src_is_int && dst_is_float {
            return self.emit_cast_copy(CastKind::IntToFloat, inner_op, dst_ty, span);
        }
        if src_is_float && dst_is_float {
            return self.emit_cast_copy(CastKind::FloatToFloat, inner_op, dst_ty, span);
        }
        if src_is_fn_def && dst_is_fn_ptr {
            let src_sig = src_ty
                .kind()
                .fn_sig()
                .expect("fn def should have signature");
            let src_fn_ptr_ty = Ty::from_rigid_kind(RigidTy::FnPtr(src_sig));
            if !ty_matches_expected(dst_ty, src_fn_ptr_ty) {
                let fn_ptr_local = self.new_temp(src_fn_ptr_ty, Mutability::Mut, span);
                self.push_statement(
                    MirStatementKind::Assign(
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
                );
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
            self.push_statement(
                MirStatementKind::Assign(
                    place(tmp),
                    Rvalue::Cast(
                        CastKind::PointerCoercion(PointerCoercion::ReifyFnPointer(Safety::Safe)),
                        inner_op,
                        dst_ty,
                    ),
                ),
                span,
            );
            return MirOperand::Copy(place(tmp));
        }
        if src_is_fn_def && let Some(fn_ptr_ty) = dst_mu_fn_ptr {
            let src_sig = src_ty
                .kind()
                .fn_sig()
                .expect("fn def should have signature");
            let src_fn_ptr_ty = Ty::from_rigid_kind(RigidTy::FnPtr(src_sig));
            let src_fn_ptr_local = self.new_temp(src_fn_ptr_ty, Mutability::Mut, span);
            self.push_statement(
                MirStatementKind::Assign(
                    place(src_fn_ptr_local),
                    Rvalue::Cast(
                        CastKind::PointerCoercion(PointerCoercion::ReifyFnPointer(Safety::Safe)),
                        inner_op,
                        src_fn_ptr_ty,
                    ),
                ),
                span,
            );
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
        if dst_is_ptr && src_is_fn_ptr {
            return self.emit_cast_copy(CastKind::FnPtrToPtr, inner_op, dst_ty, span);
        }
        if src_mu_fn_ptr.is_some() && dst_is_ptr {
            return self.read_maybe_uninit_as(inner_op, src_ty, dst_ty, span);
        }
        if dst_mu_fn_ptr.is_some() && src_is_fn_ptr {
            return self.write_value_into_maybe_uninit_storage(dst_ty, inner_op, src_ty, span);
        }
        if src_is_ptr && dst_is_ptr {
            return self.emit_cast_copy(CastKind::PtrToPtr, inner_op, dst_ty, span);
        }
        if src_is_fn_def {
            let middle_ty = {
                let src_sig = src_ty
                    .kind()
                    .fn_sig()
                    .expect("fn def should have signature");
                Ty::from_rigid_kind(RigidTy::FnPtr(src_sig))
            };
            let middle_op = self.lower_cast(inner_op, src_ty, middle_ty, span);
            return self.lower_cast(middle_op, middle_ty, dst_ty, span);
        }
        if src_is_fn_ptr && dst_is_int {
            let raw_ptr_ty = Ty::new_ptr(Ty::signed_ty(IntTy::I8), Mutability::Not);
            let raw_ptr_op = self.emit_cast_copy(CastKind::FnPtrToPtr, inner_op, raw_ptr_ty, span);
            let usize_ty = Ty::usize_ty();
            let usize_op =
                self.emit_cast_copy(CastKind::PointerExposeAddress, raw_ptr_op, usize_ty, span);
            if dst_ty == usize_ty {
                return usize_op;
            }
            return self.emit_cast_copy(CastKind::IntToInt, usize_op, dst_ty, span);
        }
        if src_is_ptr && dst_is_int {
            let usize_ty = Ty::usize_ty();
            let usize_op =
                self.emit_cast_copy(CastKind::PointerExposeAddress, inner_op, usize_ty, span);
            if dst_ty == usize_ty {
                return usize_op;
            }
            return self.emit_cast_copy(CastKind::IntToInt, usize_op, dst_ty, span);
        }
        if src_mu_fn_ptr.is_some() && dst_is_int {
            let usize_ty = Ty::usize_ty();
            let usize_op = self.read_maybe_uninit_as(inner_op, src_ty, usize_ty, span);
            if dst_ty == usize_ty {
                return usize_op;
            }
            return self.emit_cast_copy(CastKind::IntToInt, usize_op, dst_ty, span);
        }
        if src_is_int && dst_is_ptr {
            let usize_ty = Ty::usize_ty();
            let usize_op = if src_ty == usize_ty {
                inner_op
            } else {
                self.emit_cast_copy(CastKind::IntToInt, inner_op, usize_ty, span)
            };
            return self.emit_cast_copy(
                CastKind::PointerWithExposedProvenance,
                usize_op,
                dst_ty,
                span,
            );
        }
        if src_is_ptr && dst_mu_fn_ptr.is_some() {
            return self.write_value_into_maybe_uninit_storage(dst_ty, inner_op, src_ty, span);
        }
        if src_is_int && dst_mu_fn_ptr.is_some() {
            let usize_ty = Ty::usize_ty();
            let usize_op = if src_ty == usize_ty {
                inner_op
            } else {
                self.emit_cast_copy(CastKind::IntToInt, inner_op, usize_ty, span)
            };
            return self.write_value_into_maybe_uninit_storage(dst_ty, usize_op, usize_ty, span);
        }
        if src_mu_fn_ptr.is_some() && dst_is_fn_ptr {
            return self.read_maybe_uninit_as(inner_op, src_ty, dst_ty, span);
        }
        if src_mu_fn_ptr.is_some() && dst_mu_fn_ptr.is_some() {
            return self.write_value_into_maybe_uninit_storage(dst_ty, inner_op, src_ty, span);
        }
        panic!(
            "unsupported cast from {} to {}",
            self.format_ty(src_ty),
            self.format_ty(dst_ty)
        );
    }

    fn lower_dependency_const_value(
        &mut self,
        value: DependencyConstValue,
        target_ty: Ty,
        span: RustSpan,
    ) -> MirOperand {
        match value {
            DependencyConstValue::Bool(v) => self.make_int_const(i128::from(v), target_ty, span),
            DependencyConstValue::Char(v) => self.make_int_const(v as i128, target_ty, span),
            DependencyConstValue::I8(v) => self.make_int_const(v as i128, target_ty, span),
            DependencyConstValue::I16(v) => self.make_int_const(v as i128, target_ty, span),
            DependencyConstValue::I32(v) => self.make_int_const(v as i128, target_ty, span),
            DependencyConstValue::I64(v) => self.make_int_const(v as i128, target_ty, span),
            DependencyConstValue::I128(v) => self.make_int_const(v, target_ty, span),
            DependencyConstValue::Isize(v) => self.make_int_const(v as i128, target_ty, span),
            DependencyConstValue::U8(v) => self.make_int_const(v as i128, target_ty, span),
            DependencyConstValue::U16(v) => self.make_int_const(v as i128, target_ty, span),
            DependencyConstValue::U32(v) => self.make_int_const(v as i128, target_ty, span),
            DependencyConstValue::U64(v) => self.make_int_const(v as i128, target_ty, span),
            DependencyConstValue::U128(v) => self.make_int_const(v as i128, target_ty, span),
            DependencyConstValue::Usize(v) => self.make_int_const(v as i128, target_ty, span),
            DependencyConstValue::F32(v) => self.make_float_const(v as f64, target_ty, span),
            DependencyConstValue::F64(v) => self.make_float_const(v, target_ty, span),
        }
    }

    fn make_int_const(&mut self, v: i128, target_ty: Ty, span: RustSpan) -> MirOperand {
        let (uint_ty, bits) = crate::rvalue::int_literal_bits(v, target_ty);
        let c = MirConst::try_from_uint(bits, uint_ty).expect("int const");
        let const_op = MirOperand::Constant(ConstOperand {
            span,
            user_ty: None,
            const_: c,
        });
        let src_ty = Ty::unsigned_ty(uint_ty);
        if src_ty == target_ty {
            const_op
        } else {
            self.lower_cast(const_op, src_ty, target_ty, span)
        }
    }

    fn make_float_const(&mut self, v: f64, target_ty: Ty, span: RustSpan) -> MirOperand {
        let c = MirConst::try_from_float(v.to_bits() as u128, FloatTy::F64).expect("float const");
        let const_op = MirOperand::Constant(ConstOperand {
            span,
            user_ty: None,
            const_: c,
        });
        self.lower_cast(
            const_op,
            Ty::from_rigid_kind(RigidTy::Float(FloatTy::F64)),
            target_ty,
            span,
        )
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
        self.assign_const(result_local, 0, ty, span);

        match op {
            HirLogicalOp::And => self.lower_condition(
                lhs,
                span,
                |b| {
                    b.lower_condition(
                        rhs,
                        span,
                        |b| b.assign_const(result_local, 1, ty, span),
                        |_| {},
                    );
                },
                |_| {},
            ),
            HirLogicalOp::Or => self.lower_condition(
                lhs,
                span,
                |b| {
                    b.assign_const(result_local, 1, ty, span);
                },
                |b| {
                    b.lower_condition(
                        rhs,
                        span,
                        |b| b.assign_const(result_local, 1, ty, span),
                        |_| {},
                    );
                },
            ),
        }

        MirOperand::Copy(place(result_local))
    }

    fn lower_logical_not_expr(&mut self, inner: &HirExpr, span: RustSpan, ty: Ty) -> MirOperand {
        debug_assert!(matches!(inner.ty.kind(), TyKind::RigidTy(RigidTy::Bool)));
        let inner_op = self.lower_expr_to_operand(inner);
        let bool_ty = Ty::bool_ty();
        let bool_tmp = self.new_temp(bool_ty, Mutability::Not, span);
        self.push_statement(
            MirStatementKind::Assign(
                place(bool_tmp),
                Rvalue::UnaryOp(
                    rustc_public_generative::rustc_public::mir::UnOp::Not,
                    inner_op,
                ),
            ),
            span,
        );
        if bool_ty == ty {
            return MirOperand::Copy(place(bool_tmp));
        }
        self.lower_cast(MirOperand::Copy(place(bool_tmp)), bool_ty, ty, span)
    }

    fn assign_const(&mut self, local: usize, value: i128, ty: Ty, span: RustSpan) {
        let operand = self.lower_expr_to_operand(&HirExpr {
            kind: HirExprKind::ConstInt(value),
            ty,
            span,
        });
        self.emit_assign_use(place(local), operand, span);
    }

    pub(crate) fn emit_assign_use(
        &mut self,
        dst: rustc_public_generative::rustc_public::mir::Place,
        op: MirOperand,
        span: RustSpan,
    ) {
        self.push_statement(
            MirStatementKind::Assign(dst, Rvalue::Use(op, WithRetag::Yes)),
            span,
        );
    }

    fn emit_cast_copy(
        &mut self,
        cast_kind: CastKind,
        inner_op: MirOperand,
        dst_ty: Ty,
        span: RustSpan,
    ) -> MirOperand {
        let tmp = self.new_temp(dst_ty, Mutability::Mut, span);
        self.push_statement(
            MirStatementKind::Assign(place(tmp), Rvalue::Cast(cast_kind, inner_op, dst_ty)),
            span,
        );
        MirOperand::Copy(place(tmp))
    }

    fn lower_conditional_expr(
        &mut self,
        cond: &HirExpr,
        then_expr: Option<&HirExpr>,
        else_expr: &HirExpr,
        span: RustSpan,
        ty: Ty,
    ) -> MirOperand {
        let result_local = self.new_temp(ty, Mutability::Mut, span);
        if let Some(then_expr) = then_expr {
            self.lower_condition(
                cond,
                span,
                |b| {
                    let op = b.lower_expr_to_operand(then_expr);
                    b.emit_assign_use(place(result_local), op, then_expr.span);
                },
                |b| {
                    let op = b.lower_expr_to_operand(else_expr);
                    b.emit_assign_use(place(result_local), op, else_expr.span);
                },
            );
            return MirOperand::Copy(place(result_local));
        }
        // GNU elvis `a ?: b` : `a` evaluated once, condition local reused as then
        let cond_operand = self.lower_expr_to_operand(cond);
        let cond_tmp = self.new_temp(cond.ty, Mutability::Mut, cond.span);
        self.emit_assign_use(place(cond_tmp), cond_operand, cond.span);
        let cond_copy = MirOperand::Copy(place(cond_tmp));
        let bool_operand = self.lower_cast(cond_copy.clone(), cond.ty, Ty::bool_ty(), span);
        let entry_kind = TerminatorKind::SwitchInt {
            discr: bool_operand,
            targets: SwitchTargets::new(vec![(0, usize::MAX)], usize::MAX),
        };
        let entry_bb = self.blocks.len();
        self.blocks
            .push(rustc_public_generative::rustc_public::mir::BasicBlock {
                statements: std::mem::take(&mut self.stmts),
                terminator: MirTerminator {
                    kind: entry_kind,
                    source_info: SourceInfo {
                        span,
                        scope: self.current_scope(),
                    },
                },
            });
        let then_start = self.blocks.len();
        {
            let then_op = self.lower_cast(cond_copy.clone(), cond.ty, ty, span);
            self.emit_assign_use(place(result_local), then_op, cond.span);
        }
        let then_exit = self.push_terminator(TerminatorKind::Goto { target: usize::MAX }, span);
        let else_start = self.blocks.len();
        {
            let else_op = self.lower_expr_to_operand(else_expr);
            let else_op = if else_expr.ty == ty {
                else_op
            } else {
                self.lower_cast(else_op, else_expr.ty, ty, span)
            };
            self.emit_assign_use(place(result_local), else_op, else_expr.span);
        }
        let else_exit = self.push_terminator(TerminatorKind::Goto { target: usize::MAX }, span);
        let join_bb = self.blocks.len();
        self.patch_goto_target(then_exit, join_bb);
        self.patch_goto_target(else_exit, join_bb);
        self.patch_switch_targets(entry_bb, then_start, else_start);
        MirOperand::Copy(place(result_local))
    }

    fn emit_call_for_ret_ty(
        &mut self,
        func: MirOperand,
        args: Vec<MirOperand>,
        destination: rustc_public_generative::rustc_public::mir::Place,
        span: RustSpan,
        ret_ty: Ty,
    ) {
        if matches!(
            self.ctx.normalize_ty_defaults(ret_ty).kind(),
            TyKind::RigidTy(RigidTy::Never)
        ) {
            self.emit_diverging_call_block(func, args, destination, span);
        } else {
            self.emit_call_block(func, args, destination, span);
        }
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
            self.emit_call_for_ret_ty(
                fn_const_operand(*fn_def, generic_args, span),
                arg_ops,
                destination,
                span,
                ret_ty,
            );
        } else {
            let func_op = if let Some(inner_fn_ptr) = maybe_uninit_fn_ptr_inner(func.ty) {
                let op = self.lower_expr_to_operand(func);
                self.read_maybe_uninit_as(op, func.ty, inner_fn_ptr, span)
            } else {
                self.lower_expr_to_operand(func)
            };
            self.emit_call_for_ret_ty(func_op, arg_ops, destination, span, ret_ty);
        }
    }

    pub(crate) fn lower_zeroed_to_destination(
        &mut self,
        destination: rustc_public_generative::rustc_public::mir::Place,
        span: RustSpan,
        ret_ty: Ty,
    ) {
        self.emit_nullary_call(self.wellknown_defs.zeroed, ret_ty, destination, span);
    }

    pub(crate) fn lower_call_arg(&mut self, arg: &HirExpr, expected_ty: Ty) -> MirOperand {
        if let TyKind::RigidTy(RigidTy::Adt(adt, _)) = expected_ty.kind()
            && adt == self.wellknown_defs.valist
        {
            let borrowed_place = self.lower_expr_to_place_or_temp(arg);

            let reg = Region {
                kind: RegionKind::ReErased,
            };
            let ref_ty = Ty::new_ref(reg.clone(), arg.ty, Mutability::Not);
            let ref_local = self.new_temp(ref_ty, Mutability::Not, arg.span);
            self.push_statement(
                MirStatementKind::Assign(
                    place(ref_local),
                    Rvalue::Ref(reg, BorrowKind::Shared, borrowed_place),
                ),
                arg.span,
            );

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
