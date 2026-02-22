use co2_hir::LocalId;
use rustc_public_generative::rustc_public::{
    mir::{LocalDecl as MirLocalDecl, Mutability},
    ty::{Span as RustSpan, Ty},
};

use crate::build::Builder;

impl Builder<'_> {
    pub(crate) fn new_temp(&mut self, ty: Ty, mutability: Mutability, span: RustSpan) -> usize {
        let local = self.locals.len() + self.extra_locals.len();
        self.extra_locals.push(MirLocalDecl {
            ty,
            span,
            mutability,
        });
        local
    }

    pub(crate) fn local_to_index(&self, local: LocalId) -> usize {
        *self
            .local_indices
            .get(&local)
            .unwrap_or_else(|| panic!("missing MIR local mapping for {local:?}"))
    }
}
