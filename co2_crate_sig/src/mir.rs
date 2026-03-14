use rustc_public_generative::rustc_public::{
    DefId,
    ty::{AdtDef, FnDef, Span},
};

use crate::LocalResolver;

pub enum MirOwnerInfo {
    CloneMethod(AdtDef),
    StaticZeroed,
    EnumConstZeroed,
    EnumConstPrevPlus(DefId, Span),
    EnumConstExplicit {
        initializer: co2_ast::Spanned<co2_ast::Expression<LocalResolver>>,
    },
    Static {
        initializer: co2_ast::Spanned<co2_ast::Initializer<LocalResolver>>,
    },
    StaticArraySizeInference {
        span: Span,
    },
    Fn {
        def: FnDef,
        param_names: Vec<(usize, String)>,
        body: co2_ast::Spanned<co2_ast::CompoundStatement<LocalResolver>>,
    },
}
