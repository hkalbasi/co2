use crate::{CrateSigCtx, LocalResolverBase};

impl LocalResolverBase {
    pub fn co2_span_to_rustc(
        &self,
        span: co2_ast::Span,
    ) -> rustc_public_generative::rustc_public::ty::Span {
        if let Some(mapped) = self.preprocessed.map_span(span) {
            return self
                .hir_ctx
                .span_in_file(self.file_ids[mapped.file_idx], mapped.start as u32, mapped.end as u32);
        }
        self.hir_ctx
            .span_in_file(self.file_id, span.start as u32, span.end as u32)
    }
}

impl CrateSigCtx<'_> {
    pub fn co2_span_to_rustc(
        &self,
        span: co2_ast::Span,
    ) -> rustc_public_generative::rustc_public::ty::Span {
        if let Some(mapped) = self.preprocessed.map_span(span) {
            return self
                .hir_ctx
                .span_in_file(self.file_ids[mapped.file_idx], mapped.start as u32, mapped.end as u32);
        }
        self.hir_ctx
            .span_in_file(self.file_id, span.start as u32, span.end as u32)
    }
}
