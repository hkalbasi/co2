#![feature(rustc_private)]

extern crate rustc_data_structures;

mod ast_resolver;
mod attr;
mod ctx;
mod imports;
mod lowering;
mod mir;
mod resolver;
mod span;
mod struct_manager;
mod ty;

pub(crate) use ctx::CrateSigCtx;

pub use ast_resolver::{
    DefOrLocal, LocalResolver, LocalResolverBase, MethodResolutionKind, PendingExtern,
    RegisteredArrayLenConst, expr_contains_label_address, initializer_contains_label_address,
};
pub use attr::{Co2Attr, co2_attrs_to_generated};
pub use lowering::{WellknownDefs, lower_crate_sig};
pub use mir::MirOwnerInfo;
pub use resolver::{ResolveError, Resolver};
pub use struct_manager::{LogicalAdtFieldInfo, LogicalAdtFieldKind};
pub use ty::{CTy, CompressedTypeSpecifier, PrimitiveTy};
