//! Two-step attribute lowering.
//!
//! * Raw C/rust-style attributes (`co2_ast::RustAttribute`) are lowered into
//!   [`Co2Attr`] — a co2-internal representation that may carry attributes which
//!   have no rustc equivalent (e.g. `Alias`, used to build ELF symbol aliases
//!   via a forwarding definition).
//! * [`Co2Attr`] is then lowered into
//!   [`rustc_public_generative::GeneratedAttr`](GeneratedAttr) for the subset
//!   that actually maps to something rustc can emit. co2-only attributes such as
//!   `Alias` are intentionally *not* part of `GeneratedAttr`, which lives in the
//!   co2-agnostic `rustc_public_generative` backend.

use rustc_public_generative::{GeneratedAttr, InlineHint};

/// co2-internal attribute representation (the "raw → co2" step).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Co2Attr {
    Word {
        path: Vec<String>,
    },
    DocComment {
        comment: String,
        inner: bool,
    },
    InlineHint(InlineHint),
    /// Emit `#[linkage = "weak"]` so the symbol has weak linkage and may be
    /// overridden by a strong definition at link time.
    Weak,
    /// A GNU `alias("target")` attribute. co2 implements this as a weak
    /// forwarding definition; `target` is the aliased symbol name. This has no
    /// rustc equivalent and is therefore excluded from [`GeneratedAttr`].
    Alias(String),
}

impl Co2Attr {
    /// Lower a co2 attribute into a rustc-emittable `GeneratedAttr`, returning
    /// `None` for attributes that have no rustc representation (e.g. `Alias`).
    pub fn to_generated(&self) -> Option<GeneratedAttr> {
        match self {
            Co2Attr::Word { path } => Some(GeneratedAttr::Word { path: path.clone() }),
            Co2Attr::DocComment { comment, inner } => Some(GeneratedAttr::DocComment {
                comment: comment.clone(),
                inner: *inner,
            }),
            Co2Attr::InlineHint(hint) => Some(GeneratedAttr::InlineHint(*hint)),
            Co2Attr::Weak => Some(GeneratedAttr::Weak),
            Co2Attr::Alias(_) => None,
        }
    }
}

/// Lower a list of co2 attributes into rustc-emittable `GeneratedAttr`s,
/// dropping co2-only attributes (e.g. `Alias`).
pub fn co2_attrs_to_generated(attrs: &[Co2Attr]) -> Vec<GeneratedAttr> {
    attrs.iter().filter_map(Co2Attr::to_generated).collect()
}
