#![feature(rustc_private)]

mod allocation;
mod basic_block;
mod build;
mod operand;
mod place;
mod rvalue;

pub use build::build_mir_for_body;
