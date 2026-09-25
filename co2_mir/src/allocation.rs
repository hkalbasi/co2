use co2_hir::LocalId;
use rustc_public_generative::rustc_public::{
    mir::{LocalDecl as MirLocalDecl, Mutability},
    ty::{Span as RustSpan, Ty},
};

use crate::{build::Builder, operand::maybe_uninit_fn_ptr_inner, place::place};

impl Builder<'_, '_> {
    pub(crate) fn new_temp(&mut self, ty: Ty, mutability: Mutability, span: RustSpan) -> usize {
        let ty = self.ctx.normalize_ty_defaults(ty);
        let local = self.locals.len() + self.extra_locals.len();
        self.extra_locals.push(MirLocalDecl {
            ty,
            span,
            mutability,
        });
        if maybe_uninit_fn_ptr_inner(ty).is_some() {
            self.emit_nullary_call(
                self.wellknown_defs.maybe_uninit_uninit,
                ty,
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
