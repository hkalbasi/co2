#![feature(rustc_private)]

mod decl;
mod expr;
mod initializer_tree;
mod item;
mod resolver;
mod stmt;
mod ty;

pub use decl::HirDecl;
pub use expr::{HirBinOp, HirExpr, HirExprKind, HirLogicalOp, ReturnSemantic};
pub use item::{
    HirBody, HirLabel, HirLocal, LabelId, LocalId, lower_function_body, lower_static_body,
};
pub use resolver::{HirCtx, ResolvedValue};
pub use stmt::HirStmt;
pub use ty::primitive_type;
pub use co2_crate_sig::WellknownDefs;