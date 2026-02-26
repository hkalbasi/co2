use std::sync::OnceLock;

use rustc_public_generative::FileId;

use crate::CrateSigCtx;

pub static FILE_ID: OnceLock<FileId> = OnceLock::new();

pub fn co2_span_to_rustc(
    ctx: &CrateSigCtx,
    span: co2_parser::Span,
) -> rustc_public_generative::rustc_public::ty::Span {
    ctx.hir_ctx
        .span_in_file(*FILE_ID.get().unwrap(), span.start as u32, span.end as u32)
}
