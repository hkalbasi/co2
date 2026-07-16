use co2_hir::HirBinOp;
use rustc_public_generative::rustc_public::{
    mir::{
        BinOp as MirBinOp, CastKind, ConstOperand, Mutability, Operand as MirOperand,
        Place as MirPlace, ProjectionElem as MirProjection, RawPtrKind, Rvalue, SourceInfo,
        Statement as MirStatement, StatementKind as MirStatementKind, WithRetag,
    },
    ty::{IntTy, MirConst, RigidTy, Span as RustSpan, Ty, TyKind, UintTy},
};

use crate::{build::Builder, place::place};

pub(crate) fn int_literal_bits(value: i128, target_ty: Ty) -> (UintTy, u128) {
    let TyKind::RigidTy(rigid) = target_ty.kind() else {
        return (UintTy::U32, u128::from(value as i32 as u32));
    };

    match rigid {
        rustc_public_generative::rustc_public::ty::RigidTy::Int(IntTy::I8) => {
            (UintTy::U8, u128::from(value as i8 as u8))
        }
        rustc_public_generative::rustc_public::ty::RigidTy::Int(IntTy::I16) => {
            (UintTy::U16, u128::from(value as i16 as u16))
        }
        rustc_public_generative::rustc_public::ty::RigidTy::Int(IntTy::I32) => {
            (UintTy::U32, u128::from(value as i32 as u32))
        }
        rustc_public_generative::rustc_public::ty::RigidTy::Int(IntTy::I64)
        | rustc_public_generative::rustc_public::ty::RigidTy::Uint(UintTy::U64) => {
            (UintTy::U64, u128::from(value as u64))
        }
        rustc_public_generative::rustc_public::ty::RigidTy::Int(IntTy::I128)
        | rustc_public_generative::rustc_public::ty::RigidTy::Uint(UintTy::U128) => {
            (UintTy::U128, value as u128)
        }
        rustc_public_generative::rustc_public::ty::RigidTy::Int(IntTy::Isize) => {
            (UintTy::Usize, value as isize as usize as u128)
        }
        rustc_public_generative::rustc_public::ty::RigidTy::Uint(UintTy::U8) => {
            (UintTy::U8, u128::from(value as u8))
        }
        rustc_public_generative::rustc_public::ty::RigidTy::Uint(UintTy::U16) => {
            (UintTy::U16, u128::from(value as u16))
        }
        rustc_public_generative::rustc_public::ty::RigidTy::Uint(UintTy::U32) => {
            (UintTy::U32, u128::from(value as u32))
        }
        rustc_public_generative::rustc_public::ty::RigidTy::Uint(UintTy::Usize) => {
            (UintTy::Usize, value as usize as u128)
        }
        _ => (UintTy::U32, u128::from(value as i32 as u32)),
    }
}

impl Builder<'_, '_> {
    pub(crate) fn lower_bin_op(&self, op: HirBinOp) -> MirBinOp {
        match op {
            HirBinOp::Add => MirBinOp::Add,
            HirBinOp::Sub => MirBinOp::Sub,
            HirBinOp::Mul => MirBinOp::Mul,
            HirBinOp::Div => MirBinOp::Div,
            HirBinOp::Rem => MirBinOp::Rem,
            HirBinOp::BitOr => MirBinOp::BitOr,
            HirBinOp::BitXor => MirBinOp::BitXor,
            HirBinOp::BitAnd => MirBinOp::BitAnd,
            HirBinOp::Eq => MirBinOp::Eq,
            HirBinOp::Lt => MirBinOp::Lt,
            HirBinOp::Le => MirBinOp::Le,
            HirBinOp::Ne => MirBinOp::Ne,
            HirBinOp::Ge => MirBinOp::Ge,
            HirBinOp::Gt => MirBinOp::Gt,
            HirBinOp::Shl => MirBinOp::Shl,
            HirBinOp::Shr => MirBinOp::Shr,
        }
    }

    pub(crate) fn lower_const_string(
        &mut self,
        s: &[u8],
        ptr_ty: Ty,
        span: RustSpan,
    ) -> MirOperand {
        let mut bytes = s.to_vec();
        let needs_nul = !matches!(ptr_ty.kind(), TyKind::RigidTy(RigidTy::Ref(_, _, _)));
        if needs_nul {
            bytes.push(0);
        }

        // Allocate string bytes in static (rodata) memory via a &'static str constant.
        // TODO: This unsafe is super invalid. C allow arbitrary string literal, not just utf8.
        //       The whole code here is nonsense.
        let str_const = MirConst::from_str(unsafe { std::str::from_utf8_unchecked(&bytes) });
        let str_ref_ty = str_const.ty(); // &'static str

        // If the requested type is a reference (e.g. &str for s"..." literals),
        // return the &'static str constant directly without the raw-pointer
        // round-trip.
        if matches!(ptr_ty.kind(), TyKind::RigidTy(RigidTy::Ref(_, _, _))) {
            let str_ref_local = self.new_temp(str_ref_ty, Mutability::Mut, span);
            self.stmts.push(MirStatement {
                kind: MirStatementKind::Assign(
                    place(str_ref_local),
                    Rvalue::Use(
                        MirOperand::Constant(ConstOperand {
                            span,
                            user_ty: None,
                            const_: str_const,
                        }),
                        WithRetag::Yes,
                    ),
                ),
                source_info: SourceInfo {
                    span,
                    scope: self.current_scope(),
                },
            });
            return MirOperand::Copy(place(str_ref_local));
        }

        // Use Mutability::Mut so that if this assignment is inside a loop body
        // (a basic block executed multiple times), rustc does not emit E0384.
        let str_ref_local = self.new_temp(str_ref_ty, Mutability::Mut, span);
        self.stmts.push(MirStatement {
            kind: MirStatementKind::Assign(
                place(str_ref_local),
                Rvalue::Use(
                    MirOperand::Constant(ConstOperand {
                        span,
                        user_ty: None,
                        const_: str_const,
                    }),
                    WithRetag::Yes,
                ),
            ),
            source_info: SourceInfo {
                span,
                scope: self.current_scope(),
            },
        });

        // Deref the &str reference to produce a `str` DST place, then take its raw
        // address.  This yields `*const str` — a fat pointer whose data component
        // is the address of the bytes in the static allocation.
        let str_ty = Ty::from_rigid_kind(RigidTy::Str);
        let ptr_str_ty = Ty::new_ptr(str_ty, Mutability::Not); // *const str (fat)
        let ptr_str_local = self.new_temp(ptr_str_ty, Mutability::Mut, span);
        let deref_place = MirPlace {
            local: str_ref_local,
            projection: vec![MirProjection::Deref],
        };
        self.stmts.push(MirStatement {
            kind: MirStatementKind::Assign(
                place(ptr_str_local),
                Rvalue::AddressOf(RawPtrKind::Const, deref_place),
            ),
            source_info: SourceInfo {
                span,
                scope: self.current_scope(),
            },
        });

        // Cast *const str (fat) → *const u8 (thin, data component only).
        let elem_ty = Ty::unsigned_ty(UintTy::U8);
        let ptr_u8_ty = Ty::new_ptr(elem_ty, Mutability::Not);
        let ptr_u8_local = self.new_temp(ptr_u8_ty, Mutability::Mut, span);
        self.stmts.push(MirStatement {
            kind: MirStatementKind::Assign(
                place(ptr_u8_local),
                Rvalue::Cast(
                    CastKind::PtrToPtr,
                    MirOperand::Copy(place(ptr_str_local)),
                    ptr_u8_ty,
                ),
            ),
            source_info: SourceInfo {
                span,
                scope: self.current_scope(),
            },
        });

        let ptr_local = self.new_temp(ptr_ty, Mutability::Mut, span);
        self.stmts.push(MirStatement {
            kind: MirStatementKind::Assign(
                place(ptr_local),
                Rvalue::Cast(
                    CastKind::PtrToPtr,
                    MirOperand::Copy(place(ptr_u8_local)),
                    ptr_ty,
                ),
            ),
            source_info: SourceInfo {
                span,
                scope: self.current_scope(),
            },
        });

        MirOperand::Copy(place(ptr_local))
    }
}
