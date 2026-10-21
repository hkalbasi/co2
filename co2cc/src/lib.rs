#![feature(rustc_private)]

extern crate rustc_driver;

mod args;
mod cli;
mod compile;
mod depfile;
mod link;
mod rustc;

pub fn main_with_args(args: &[String]) -> std::process::ExitCode {
    cli::main_with_args(args)
}
