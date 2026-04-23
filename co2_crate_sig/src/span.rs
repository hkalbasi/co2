use crate::{CrateSigCtx, LocalResolverBase};

impl LocalResolverBase {
    pub fn co2_span_to_rustc(
        &self,
        span: co2_ast::Span,
    ) -> rustc_public_generative::rustc_public::ty::Span {
        let file_id = *self
            .file_ids
            .get(&span.context)
            .unwrap_or_else(|| panic!("missing rustc file id for {:?}", span.context));
        self.hir_ctx
            .span_in_file(file_id, span.start as u32, span.end as u32)
    }
}

impl CrateSigCtx<'_> {
    pub fn co2_span_to_rustc(
        &self,
        span: co2_ast::Span,
    ) -> rustc_public_generative::rustc_public::ty::Span {
        let file_id = *self
            .file_ids
            .get(&span.context)
            .unwrap_or_else(|| panic!("missing rustc file id for {:?}", span.context));
        self.hir_ctx
            .span_in_file(file_id, span.start as u32, span.end as u32)
    }
}
