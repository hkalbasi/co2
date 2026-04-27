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
    if let Some(manifest_dir) = std::env::var_os("CARGO_MANIFEST_DIR") {
        co2_ast::set_diagnostic_base_path(Some(std::path::PathBuf::from(manifest_dir)));
    }

    co2_ast::set_force_json_diagnostics(rustc_requests_json_diagnostics(&args));

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
        if co2_ast::is_diagnostic_abort(payload.as_ref()) {
            return std::process::ExitCode::from(5);
        }
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

fn rustc_requests_json_diagnostics(args: &[String]) -> bool {
    args.iter().enumerate().any(|(idx, arg)| {
        arg == "--error-format=json"
            || (arg == "--error-format" && args.get(idx + 1).is_some_and(|value| value == "json"))
    })
}
