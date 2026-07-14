#![feature(rustc_private)]

mod decl;
mod expr;
mod initializer_tree;
mod item;
mod resolver;
mod stmt;
mod ty;

pub use co2_crate_sig::{LocalResolver, WellknownDefs};
pub use decl::HirDecl;
pub use expr::{HirBinOp, HirExpr, HirExprKind, HirLogicalOp, ReturnSemantic};
pub(crate) use item::infer_array_len_from_initializer_in_scope;
pub use item::{
    HirBody, HirLabel, HirLocal, LabelId, LocalId, eval_usize_initializer,
    infer_array_len_from_initializer, lower_function_body, lower_static_body,
    lower_static_body_for_ty,
};
pub use resolver::{HirCtx, ResolvedValue};
pub use stmt::HirStmt;
pub use ty::format_ty;
pub use ty::is_unsized_ty;
