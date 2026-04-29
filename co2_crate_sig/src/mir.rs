use rustc_public_generative::rustc_public::{
    DefId,
    ty::{AdtDef, FnDef, Span},
};

use crate::LocalResolver;

#[derive(Debug, Clone)]
pub enum MirOwnerInfo {
    CloneMethod(AdtDef),
    StaticZeroed,
    EnumConstZeroed,
    EnumConstPrevPlus(DefId, Span),
    EnumConstExplicit {
        resolver: LocalResolver,
        initializer: co2_ast::Spanned<co2_ast::Expression<LocalResolver>>,
    },
    Static {
        resolver: LocalResolver,
        initializer: co2_ast::Spanned<co2_ast::Initializer<LocalResolver>>,
    },
    StaticWithArrayLen {
        resolver: LocalResolver,
        initializer: co2_ast::Spanned<co2_ast::Initializer<LocalResolver>>,
        array_len: co2_ast::Spanned<co2_ast::Initializer<LocalResolver>>,
    },
    Fn {
        def: FnDef,
        function_name: String,
        param_names: Vec<(usize, String)>,
        resolver: LocalResolver,
        body: co2_ast::Spanned<co2_ast::CompoundStatement<LocalResolver>>,
    },
    FnBodyError {
        def: FnDef,
        body_span: co2_ast::Span,
    },
}
