use rustc_public_generative::rustc_public::{
    DefId,
    ty::{AdtDef, FnDef, Span},
};

use crate::LocalResolver;

#[derive(Debug, Clone)]
pub enum MirOwnerInfo {
    CloneMethod(AdtDef),
    Const,
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
        param_names: Vec<(usize, String, Span)>,
        resolver: LocalResolver,
        body: co2_ast::Spanned<co2_ast::CompoundStatement<LocalResolver>>,
    },
    FnBodyError {
        def: FnDef,
        body_span: co2_ast::Span,
    },
    /// A weak-alias forwarder: a function that simply forwards all of its
    /// arguments to `target` and returns the result. Used to implement the GNU
    /// `__attribute__((alias("target")))` extension for functions.
    ForwardingFn {
        def: FnDef,
        target: FnDef,
        param_names: Vec<(usize, String, Span)>,
        resolver: LocalResolver,
    },
}
