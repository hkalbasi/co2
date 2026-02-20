use std::sync::OnceLock;

use rustc_public_generative::{FileId, HirStructureCtx};

pub static FILE_ID: OnceLock<FileId> = OnceLock::new();

pub fn co2_span_to_rustc(
    ctx: &HirStructureCtx,
    span: co2_parser::Span,
) -> rustc_public_generative::rustc_public::ty::Span {
    ctx.span_in_file(*FILE_ID.get().unwrap(), span.start as u32, span.end as u32)
}
