use crate::CrateSigCtx;

impl CrateSigCtx<'_> {
    pub fn co2_span_to_rustc(
        &self,
        span: co2_ast::Span,
    ) -> rustc_public_generative::rustc_public::ty::Span {
        self.hir_ctx
            .span_in_file(self.file_id, span.start as u32, span.end as u32)
    }
}
