#![feature(rustc_private)]

mod allocation;
mod basic_block;
mod build;
mod initializer_tree;
mod operand;
mod optimization;
mod place;
mod rvalue;

pub use build::build_mir_for_body;
