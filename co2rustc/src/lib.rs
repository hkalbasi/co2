#![feature(rustc_private)]

extern crate rustc_ast;
extern crate rustc_driver;
extern crate rustc_interface;
extern crate rustc_span;

mod detect;

use co2_driver_lib::{CompileMode, compile_co2_file};
use detect::{DetectResult, detect_co2};

pub fn main() -> std::process::ExitCode {
    main_with_args(std::env::args().collect())
}

pub fn main_with_args(args: Vec<String>) -> std::process::ExitCode {
    let args = maybe_force_json_diagnostics(args);
    let co2_file = match detect_co2(&args) {
        DetectResult::Continue(exit_code) => {
            return exit_code;
        }
        DetectResult::Co2(file) => file,
    };

    if let Err(payload) =
        std::panic::catch_unwind(|| compile_co2_file(CompileMode::RUST, &co2_file, args))
    {
        if let Some(msg) = payload.downcast_ref::<String>() {
            eprintln!("co2rustc panic: {msg}");
        } else if let Some(msg) = payload.downcast_ref::<&str>() {
            eprintln!("co2rustc panic: {msg}");
        } else {
            eprintln!("co2rustc panic: non-string payload");
        }
        return std::process::ExitCode::from(101);
    }

    std::process::ExitCode::SUCCESS
}

fn maybe_force_json_diagnostics(mut args: Vec<String>) -> Vec<String> {
    if std::env::var_os("CO2_FORCE_JSON_DIAGNOSTICS").is_some()
        && !args.iter().any(|arg| arg == "--error-format=json")
    {
        args.push("--error-format=json".to_owned());
    }
    args
}
