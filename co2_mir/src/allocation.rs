use co2_hir::LocalId;
use rustc_public_generative::rustc_public::{
    CrateDefType,
    mir::{
        CastKind, ConstOperand, LocalDecl as MirLocalDecl, Mutability, Operand as MirOperand,
        Rvalue, Statement as MirStatement, StatementKind as MirStatementKind,
    },
    ty::{GenericArgKind, RigidTy, Span as RustSpan, Ty, TyKind, UintTy},
};

use crate::{
    build::{Builder, fn_const_operand, infer_fn_generic_args},
    place::place,
};

fn is_maybe_uninit_fn_ptr_ty(ty: Ty) -> bool {
    let TyKind::RigidTy(RigidTy::Adt(_, args)) = ty.kind() else {
        return false;
    };
    if args.0.len() != 1 {
        return false;
    }
    let GenericArgKind::Type(inner) = args.0[0] else {
        return false;
    };
    matches!(inner.kind(), TyKind::RigidTy(RigidTy::FnPtr(_)))
}

impl<'ctx, 'tcx> Builder<'ctx, 'tcx> {
    pub(crate) fn new_temp(&mut self, ty: Ty, mutability: Mutability, span: RustSpan) -> usize {
        let local = self.locals.len() + self.extra_locals.len();
        self.extra_locals.push(MirLocalDecl {
            ty,
            span,
            mutability,
        });
        if matches!(
            ty.kind(),
            TyKind::RigidTy(RigidTy::Int(_) | RigidTy::Uint(_))
        ) {
            let (uint_ty, bits) = crate::rvalue::int_literal_bits(0, ty);
            let c =
                rustc_public_generative::rustc_public::ty::MirConst::try_from_uint(bits, uint_ty)
                    .expect("failed to build zero const");
            let const_op = MirOperand::Constant(ConstOperand {
                span,
                user_ty: None,
                const_: c,
            });
            let value = if matches!(ty.kind(), TyKind::RigidTy(RigidTy::Uint(_))) {
                Rvalue::Use(const_op)
            } else {
                Rvalue::Cast(CastKind::IntToInt, const_op, ty)
            };
            self.stmts.push(MirStatement {
                kind: MirStatementKind::Assign(place(local), value),
                span,
            });
        } else if matches!(ty.kind(), TyKind::RigidTy(RigidTy::RawPtr(_, _))) {
            let c = rustc_public_generative::rustc_public::ty::MirConst::try_from_uint(
                0,
                UintTy::Usize,
            )
            .expect("failed to build zero usize const");
            self.stmts.push(MirStatement {
                kind: MirStatementKind::Assign(
                    place(local),
                    Rvalue::Cast(
                        CastKind::PointerWithExposedProvenance,
                        MirOperand::Constant(ConstOperand {
                            span,
                            user_ty: None,
                            const_: c,
                        }),
                        ty,
                    ),
                ),
                span,
            });
        } else if is_maybe_uninit_fn_ptr_ty(ty) {
            let uninit_fn = self.wellknown_defs.maybe_uninit_uninit;
            let sig = uninit_fn
                .ty()
                .kind()
                .fn_sig()
                .expect("MaybeUninit::uninit has no signature")
                .skip_binder();
            let generic_args = infer_fn_generic_args(uninit_fn, &sig, &[], ty);
            self.emit_call_block(
                fn_const_operand(uninit_fn, generic_args, span),
                vec![],
                place(local),
                span,
            );
        }
        local
    }

    pub(crate) fn local_to_index(&self, local: LocalId) -> usize {
        *self
            .local_indices
            .get(&local)
            .unwrap_or_else(|| panic!("missing MIR local mapping for {local:?}"))
    }
}
