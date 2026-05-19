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
};
pub use lowering::{WellknownDefs, lower_crate_sig};
pub use mir::MirOwnerInfo;
pub use resolver::Resolver;
pub use struct_manager::{LogicalAdtFieldInfo, LogicalAdtFieldKind};
pub use ty::{CTy, CompressedTypeSpecifier, PrimitiveTy};
