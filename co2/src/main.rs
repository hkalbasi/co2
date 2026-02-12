#![feature(rustc_private)]

extern crate rustc_ast;
extern crate rustc_driver;
extern crate rustc_interface;
extern crate rustc_span;

mod detect;

use co2_driver_lib::{CompileMode, compile_co2_file};
use detect::{DetectResult, detect_co2};

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let co2_file = match detect_co2(&args) {
        DetectResult::Continue(exit_code) => {
            std::process::exit(exit_code);
        }
        DetectResult::Co2(file) => file,
    };

    if let Err(payload) = std::panic::catch_unwind(|| compile_co2_file(CompileMode::RUST, &co2_file)) {
        if let Some(msg) = payload.downcast_ref::<String>() {
            eprintln!("co2 panic: {msg}");
        } else if let Some(msg) = payload.downcast_ref::<&str>() {
            eprintln!("co2 panic: {msg}");
        } else {
            eprintln!("co2 panic: non-string payload");
        }
        std::process::exit(101);
    }
}
