use co2_hir::HirBinOp;
use rustc_public_generative::rustc_public::{
    mir::{
        AggregateKind, BinOp as MirBinOp, CastKind, ConstOperand, Mutability,
        Operand as MirOperand, RawPtrKind, Rvalue, Statement as MirStatement,
        StatementKind as MirStatementKind,
    },
    ty::{IntTy, MirConst, Span as RustSpan, Ty, TyKind, UintTy},
};

use crate::{build::Builder, place::place};

pub(crate) fn int_literal_bits(value: i128, target_ty: Ty) -> (UintTy, u128) {
    let TyKind::RigidTy(rigid) = target_ty.kind() else {
        return (UintTy::U32, value as i32 as u32 as u128);
    };

    match rigid {
        rustc_public_generative::rustc_public::ty::RigidTy::Int(IntTy::I8) => {
            (UintTy::U8, value as i8 as u8 as u128)
        }
        rustc_public_generative::rustc_public::ty::RigidTy::Int(IntTy::I16) => {
            (UintTy::U16, value as i16 as u16 as u128)
        }
        rustc_public_generative::rustc_public::ty::RigidTy::Int(IntTy::I32) => {
            (UintTy::U32, value as i32 as u32 as u128)
        }
        rustc_public_generative::rustc_public::ty::RigidTy::Int(IntTy::I64) => {
            (UintTy::U64, value as u64 as u128)
        }
        rustc_public_generative::rustc_public::ty::RigidTy::Int(IntTy::I128) => {
            (UintTy::U128, value as i128 as u128)
        }
        rustc_public_generative::rustc_public::ty::RigidTy::Int(IntTy::Isize) => {
            (UintTy::Usize, value as isize as usize as u128)
        }
        rustc_public_generative::rustc_public::ty::RigidTy::Uint(UintTy::U8) => {
            (UintTy::U8, value as u8 as u128)
        }
        rustc_public_generative::rustc_public::ty::RigidTy::Uint(UintTy::U16) => {
            (UintTy::U16, value as u16 as u128)
        }
        rustc_public_generative::rustc_public::ty::RigidTy::Uint(UintTy::U32) => {
            (UintTy::U32, value as u32 as u128)
        }
        rustc_public_generative::rustc_public::ty::RigidTy::Uint(UintTy::U64) => {
            (UintTy::U64, value as u64 as u128)
        }
        rustc_public_generative::rustc_public::ty::RigidTy::Uint(UintTy::U128) => {
            (UintTy::U128, value as u128)
        }
        rustc_public_generative::rustc_public::ty::RigidTy::Uint(UintTy::Usize) => {
            (UintTy::Usize, value as usize as u128)
        }
        _ => (UintTy::U32, value as i32 as u32 as u128),
    }
}

impl<'ctx, 'tcx> Builder<'ctx, 'tcx> {
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

    pub(crate) fn lower_const_string(&mut self, s: &[u8], span: RustSpan) -> MirOperand {
        let mut bytes = s.to_vec();
        if bytes.last().copied() != Some(0) {
            bytes.push(0);
        }

        let elem_ty = Ty::unsigned_ty(UintTy::U8);
        let array_ty = Ty::try_new_array(elem_ty, bytes.len() as u64)
            .expect("failed to build array type for string literal");
        let array_local = self.new_temp(array_ty, Mutability::Mut, span);
        let operands = bytes
            .iter()
            .map(|&byte| {
                MirOperand::Constant(ConstOperand {
                    span,
                    user_ty: None,
                    const_: MirConst::try_from_uint(byte as u128, UintTy::U8)
                        .expect("failed to build string literal byte constant"),
                })
            })
            .collect();
        self.stmts.push(MirStatement {
            kind: MirStatementKind::Assign(
                place(array_local),
                Rvalue::Aggregate(AggregateKind::Array(elem_ty), operands),
            ),
            span,
        });

        let ptr_array_ty = Ty::new_ptr(array_ty, Mutability::Not);
        let ptr_array_local = self.new_temp(ptr_array_ty, Mutability::Mut, span);
        self.stmts.push(MirStatement {
            kind: MirStatementKind::Assign(
                place(ptr_array_local),
                Rvalue::AddressOf(RawPtrKind::Const, place(array_local)),
            ),
            span,
        });

        let ptr_u8_ty = Ty::new_ptr(elem_ty, Mutability::Not);
        let ptr_u8_local = self.new_temp(ptr_u8_ty, Mutability::Mut, span);
        self.stmts.push(MirStatement {
            kind: MirStatementKind::Assign(
                place(ptr_u8_local),
                Rvalue::Cast(
                    CastKind::PtrToPtr,
                    MirOperand::Copy(place(ptr_array_local)),
                    ptr_u8_ty,
                ),
            ),
            span,
        });

        let ptr_i8_ty = Ty::new_ptr(Ty::signed_ty(IntTy::I8), Mutability::Mut);
        let ptr_i8_local = self.new_temp(ptr_i8_ty, Mutability::Mut, span);
        self.stmts.push(MirStatement {
            kind: MirStatementKind::Assign(
                place(ptr_i8_local),
                Rvalue::Cast(
                    CastKind::PtrToPtr,
                    MirOperand::Copy(place(ptr_u8_local)),
                    ptr_i8_ty,
                ),
            ),
            span,
        });

        MirOperand::Copy(place(ptr_i8_local))
    }
}
