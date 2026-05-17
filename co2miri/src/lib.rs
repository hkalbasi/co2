#![feature(rustc_private)]

extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_middle;
extern crate rustc_session;

use std::env;
use std::path::PathBuf;

use co2_driver_lib::{CompileMode, compile_co2_file, compile_co2_file_for_miri};
use co2rustc::{DetectResult, detect_co2};
use miri::{MIRI_DEFAULT_ARGS, MiriConfig, MiriEntryFnType, eval_entry};
use rustc_driver::Compilation;
use rustc_middle::ty::TyCtxt;

pub fn main() -> std::process::ExitCode {
    main_with_args(std::env::args().collect())
}

pub fn main_with_args(args: Vec<String>) -> std::process::ExitCode {
    if let Some(manifest_dir) = env::var_os("CARGO_MANIFEST_DIR") {
        co2_ast::set_diagnostic_base_path(Some(PathBuf::from(manifest_dir)));
    }

    if let Some(crate_kind) = env::var_os("MIRI_BE_RUSTC") {
        be_rustc_mode(args, crate_kind == "target")
    } else {
        interpreter_mode(args)
    }
}

fn split_program_args(args: Vec<String>) -> (Vec<String>, Vec<String>) {
    let mut rustc_args = Vec::new();
    let mut program_args = Vec::new();
    let mut after_dashdash = false;

    for arg in args {
        if after_dashdash {
            program_args.push(arg);
        } else if arg == "--" {
            after_dashdash = true;
        } else {
            rustc_args.push(arg);
        }
    }

    (rustc_args, program_args)
}

// Splice MIRI_DEFAULT_ARGS after argv[0].
fn splice_miri_default_args(mut args: Vec<String>) -> Vec<String> {
    if !args.is_empty() {
        args.splice(1..1, MIRI_DEFAULT_ARGS.iter().map(ToString::to_string));
    }
    args
}

/// Acting as a rustc compiler for dependency crates (MIRI_BE_RUSTC mode).
fn be_rustc_mode(args: Vec<String>, target_crate: bool) -> std::process::ExitCode {
    // Only splice MIRI_DEFAULT_ARGS (including --cfg=miri) for target crates,
    // not host crates (build scripts, proc macros).
    let args = if target_crate {
        splice_miri_default_args(args)
    } else {
        args
    };

    co2_ast::set_force_json_diagnostics(rustc_requests_json_diagnostics(&args));

    match detect_co2(&args) {
        DetectResult::Continue(exit_code) => exit_code,
        DetectResult::Co2(co2_file) => run_co2_compile(&co2_file, args),
    }
}

/// Interpreter mode: compile co2 source and run under miri.
fn interpreter_mode(args: Vec<String>) -> std::process::ExitCode {
    let (rustc_args, program_args) = split_program_args(args);
    co2_ast::set_force_json_diagnostics(rustc_requests_json_diagnostics(&rustc_args));

    // Detect whether this is a co2 source file.
    // detect_co2 runs rustc up to after_crate_root_parsing; for co2 files it stops early
    // and returns the .co2 path. For non-co2 files it runs rustc fully and returns Continue.
    let co2_file = match detect_co2(&rustc_args) {
        DetectResult::Continue(exit_code) => {
            // Not a co2 file. This shouldn't happen for co2 projects but handle gracefully.
            return exit_code;
        }
        DetectResult::Co2(file) => file,
    };

    // Snapshot environment before we mutate it.
    let env_snapshot: Vec<_> = env::vars_os().collect();
    let miri_config = MiriConfig {
        env: env_snapshot,
        args: program_args,
        ..Default::default()
    };

    // Splice MIRI_DEFAULT_ARGS and run co2 pipeline + miri interpretation.
    let miri_args = splice_miri_default_args(rustc_args);

    if let Err(payload) = std::panic::catch_unwind(|| {
        compile_co2_file_for_miri(
            &co2_file,
            miri_args,
            Box::new(move |tcx| interpret_with_miri(tcx, miri_config)),
        );
    }) {
        if co2_ast::is_diagnostic_abort(payload.as_ref()) {
            return std::process::ExitCode::from(5);
        }
        if let Some(msg) = payload.downcast_ref::<String>() {
            eprintln!("co2miri panic: {msg}");
        } else if let Some(msg) = payload.downcast_ref::<&str>() {
            eprintln!("co2miri panic: {msg}");
        } else {
            eprintln!("co2miri panic: non-string payload");
        }
        return std::process::ExitCode::from(101);
    }

    std::process::ExitCode::SUCCESS
}

/// Called from after_analysis: run miri::eval_entry on the compiled TyCtxt.
fn interpret_with_miri(tcx: TyCtxt<'_>, mut config: MiriConfig) -> Compilation {
    if tcx.sess.dcx().has_errors_or_delayed_bugs().is_some() {
        tcx.dcx()
            .fatal("miri cannot run: the program failed to compile");
    }

    let (entry_def_id, entry_type) = if let Some((id, ty)) = tcx.entry_fn(()) {
        (id, MiriEntryFnType::Rustc(ty))
    } else {
        tcx.dcx()
            .fatal("miri can only run programs that have a main function");
    };

    // Pass the filestem as argv[0] to the interpreted program.
    config
        .args
        .insert(0, tcx.sess.io.input.filestem().to_string());

    let exit_code = match eval_entry(tcx, entry_def_id, entry_type, &config, None) {
        Ok(()) => 0,
        Err(code) => code.get(),
    };

    std::process::exit(exit_code);
}

fn run_co2_compile(co2_file: &std::path::Path, args: Vec<String>) -> std::process::ExitCode {
    if let Err(payload) =
        std::panic::catch_unwind(|| compile_co2_file(CompileMode::RUST, co2_file, args))
    {
        if co2_ast::is_diagnostic_abort(payload.as_ref()) {
            return std::process::ExitCode::from(5);
        }
        if let Some(msg) = payload.downcast_ref::<String>() {
            eprintln!("co2miri (be_rustc) panic: {msg}");
        } else if let Some(msg) = payload.downcast_ref::<&str>() {
            eprintln!("co2miri (be_rustc) panic: {msg}");
        } else {
            eprintln!("co2miri (be_rustc) panic: non-string payload");
        }
        return std::process::ExitCode::from(101);
    }
    std::process::ExitCode::SUCCESS
}

fn rustc_requests_json_diagnostics(args: &[String]) -> bool {
    args.iter().enumerate().any(|(idx, arg)| {
        arg == "--error-format=json"
            || (arg == "--error-format" && args.get(idx + 1).is_some_and(|v| v == "json"))
    })
}
