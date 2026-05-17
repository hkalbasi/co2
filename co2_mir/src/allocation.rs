use co2_hir::LocalId;
use rustc_public_generative::rustc_public::{
    CrateDefType,
    mir::{LocalDecl as MirLocalDecl, Mutability},
    ty::{GenericArgKind, RigidTy, Span as RustSpan, Ty, TyKind},
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

impl Builder<'_, '_> {
    pub(crate) fn new_temp(&mut self, ty: Ty, mutability: Mutability, span: RustSpan) -> usize {
        let ty = self.ctx.normalize_ty_defaults(ty);
        let local = self.locals.len() + self.extra_locals.len();
        self.extra_locals.push(MirLocalDecl {
            ty,
            span,
            mutability,
        });
        if is_maybe_uninit_fn_ptr_ty(ty) {
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
