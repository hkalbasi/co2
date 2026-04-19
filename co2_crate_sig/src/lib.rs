#![feature(rustc_private)]

mod ast_resolver;
mod ctx;
mod lowering;
mod mir;
mod resolver;
mod span;
mod struct_manager;
mod ty;

pub(crate) use ctx::CrateSigCtx;

pub use ast_resolver::{
    DefOrLocal, LocalResolver, LocalResolverBase, MethodResolutionKind, RegisteredArrayLenConst,
    eval_registered_array_len_const,
};
pub use lowering::{WellknownDefs, lower_crate_sig};
pub use mir::MirOwnerInfo;
pub use resolver::Resolver;
pub use ty::{CTy, CompressedTypeSpecifier, PrimitiveTy};
