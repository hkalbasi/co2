#![feature(rustc_private)]

extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_session;

use std::env;
use std::num::NonZero;
use std::path::PathBuf;

use co2_driver_lib::{
    CompileMode, compile_co2_file, compile_co2_file_for_miri, force_extern_crates,
};
use co2rustc::{DetectResult, detect_co2};
use miri::{
    AlignmentCheck, AllocId, BacktraceStyle, BorTag, BorrowTrackerMethod, FloatRoundingErrorMode,
    IsolatedOp, MIRI_DEFAULT_ARGS, MiriConfig, ProvenanceMode, RejectOpWith, TreeBorrowsParams,
    ValidationMode, entry_fn, eval_entry,
};
use rustc_driver::Compilation;
use rustc_middle::ty::TyCtxt;

pub fn main() -> std::process::ExitCode {
    main_with_args(std::env::args().collect())
}

fn co2_version() -> String {
    std::env::var("CO2_VERSION").unwrap_or_else(|_| "unknown".to_owned())
}

pub fn main_with_args(args: Vec<String>) -> std::process::ExitCode {
    rustc_driver::install_ice_hook("https://github.com/HKalbasi/co2", |_| ());
    if let Some(manifest_dir) = env::var_os("CARGO_MANIFEST_DIR") {
        co2_ast::set_diagnostic_base_path(Some(PathBuf::from(manifest_dir)));
    }

    if let Some(crate_kind) = env::var_os("MIRI_BE_RUSTC") {
        be_rustc_mode(args, crate_kind == "target")
    } else {
        // Show CO2 version when --version is passed in interpreter mode.
        // Only check rustc-side args (before `--`).
        let rustc_only = args
            .iter()
            .take_while(|a| a.as_str() != "--")
            .collect::<Vec<_>>();
        if rustc_only.iter().any(|a| *a == "--version" || *a == "-V") {
            println!("co2miri {}", co2_version());
            return std::process::ExitCode::SUCCESS;
        }
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
        DetectResult::Co2(co2_file) => {
            // Eagerly load --extern crates (as co2rustc does): co2's resolver
            // reads the already-loaded extern set, so lazily-loaded deps would
            // resolve as "unresolved item".
            let mut args = args;
            force_extern_crates(&mut args);
            run_co2_compile(&co2_file, args)
        }
    }
}

/// Interpreter mode: compile co2 source and run under miri.
fn interpreter_mode(args: Vec<String>) -> std::process::ExitCode {
    // Snapshot environment before we mutate it.
    let env_snapshot: Vec<_> = env::vars_os().collect();

    let (rustc_args, program_args) = split_program_args(args);
    let (miri_config, rustc_args) = parse_miri_config(rustc_args, program_args, env_snapshot);

    co2_ast::set_force_json_diagnostics(rustc_requests_json_diagnostics(&rustc_args));

    // Plain Rust files have no co2 source to preprocess: interpret them directly.
    // (detect_co2 would natively compile them, which cannot link against the
    // miri sysroot whose rlibs reference miri intrinsics.)
    if !crate_root_looks_like_co2(&rustc_args) {
        return interpret_rust_file(rustc_args, miri_config);
    }

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

    // Splice MIRI_DEFAULT_ARGS and run co2 pipeline + miri interpretation.
    let mut miri_args = splice_miri_default_args(rustc_args);
    force_extern_crates(&mut miri_args);

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

/// Best-effort crate-root lookup: last `.rs` input file in the rustc args.
/// Only used to pick co2 vs plain-Rust interpretation; unknown files fall
/// back to the co2 path so detect_co2 decides as before.
fn crate_root_file(rustc_args: &[String]) -> Option<&str> {
    rustc_args
        .iter()
        .filter(|a| {
            let a = a.as_str();
            !a.starts_with('-') && (a.ends_with(".rs") || a.ends_with(".co2"))
        })
        .next_back()
        .map(String::as_str)
}

/// Valid co2 host files contain exactly `#![language(co2)]`; anything else is
/// plain Rust. Whitespace-insensitive text check, no parsing needed.
fn crate_root_looks_like_co2(rustc_args: &[String]) -> bool {
    let Some(path) = crate_root_file(rustc_args) else {
        return true;
    };
    let Ok(src) = std::fs::read_to_string(path) else {
        return true;
    };
    let compact: String = src.split_whitespace().collect();
    compact.contains("language(co2)")
}

/// Interpreter mode for plain Rust files: skip the co2 pipeline and run miri
/// on the crate directly, like stock miri does.
fn interpret_rust_file(rustc_args: Vec<String>, miri_config: MiriConfig) -> std::process::ExitCode {
    let mut miri_args = splice_miri_default_args(rustc_args);
    force_extern_crates(&mut miri_args);

    struct InterpretCallbacks {
        config: Option<MiriConfig>,
    }
    impl rustc_driver::Callbacks for InterpretCallbacks {
        fn after_analysis<'tcx>(
            &mut self,
            _compiler: &rustc_interface::interface::Compiler,
            tcx: TyCtxt<'tcx>,
        ) -> Compilation {
            interpret_with_miri(
                tcx,
                self.config.take().expect("miri after_analysis ran twice"),
            )
        }
    }

    let mut callbacks = InterpretCallbacks {
        config: Some(miri_config),
    };
    // interpret_with_miri exits the process on success; falling through here
    // means the program failed to compile.
    rustc_driver::catch_with_exit_code(|| rustc_driver::run_compiler(&miri_args, &mut callbacks))
}

/// Parse `-Zmiri-*` flags from `args`, configure `MiriConfig`, and return the
/// filtered `rustc_args` (with miri flags removed).
fn parse_miri_config(
    args: Vec<String>,
    program_args: Vec<String>,
    env: Vec<(std::ffi::OsString, std::ffi::OsString)>,
) -> (MiriConfig, Vec<String>) {
    let mut miri_config = MiriConfig {
        env,
        args: program_args,
        ..Default::default()
    };

    let mut rustc_args: Vec<String> = Vec::new();

    for arg in args {
        if arg == "-Zmiri-disable-validation" {
            miri_config.validation = ValidationMode::No;
        } else if arg == "-Zmiri-recursive-validation" {
            miri_config.validation = ValidationMode::Deep;
        } else if arg == "-Zmiri-disable-stacked-borrows" {
            miri_config.borrow_tracker = None;
        } else if arg == "-Zmiri-tree-borrows" {
            miri_config.borrow_tracker =
                Some(BorrowTrackerMethod::TreeBorrows(TreeBorrowsParams {
                    precise_interior_mut: true,
                    implicit_writes: false,
                    box_custom_allocator_unique: true,
                }));
        } else if arg == "-Zmiri-tree-borrows-no-precise-interior-mut" {
            match &mut miri_config.borrow_tracker {
                Some(BorrowTrackerMethod::TreeBorrows(params)) => {
                    params.precise_interior_mut = false;
                }
                _ => {
                    eprintln!(
                        "`-Zmiri-tree-borrows` is required before `-Zmiri-tree-borrows-no-precise-interior-mut`"
                    );
                    std::process::exit(1);
                }
            };
        } else if arg == "-Zmiri-tree-borrows-implicit-writes" {
            match &mut miri_config.borrow_tracker {
                Some(BorrowTrackerMethod::TreeBorrows(params)) => {
                    params.implicit_writes = true;
                }
                _ => {
                    eprintln!(
                        "`-Zmiri-tree-borrows` is required before `-Zmiri-tree-borrows-implicit-writes`"
                    );
                    std::process::exit(1);
                }
            };
        } else if arg == "-Zmiri-tree-borrows-relax-custom-allocator-uniqueness" {
            match &mut miri_config.borrow_tracker {
                Some(BorrowTrackerMethod::TreeBorrows(params)) => {
                    params.box_custom_allocator_unique = false;
                }
                _ => {
                    eprintln!(
                        "`-Zmiri-tree-borrows` is required before `-Zmiri-tree-borrows-relax-custom-allocator-uniqueness`"
                    );
                    std::process::exit(1);
                }
            };
        } else if arg == "-Zmiri-disable-data-race-detector" {
            miri_config.data_race_detector = false;
            miri_config.weak_memory_emulation = false;
        } else if arg == "-Zmiri-disable-alignment-check" {
            miri_config.check_alignment = AlignmentCheck::None;
        } else if arg == "-Zmiri-symbolic-alignment-check" {
            miri_config.check_alignment = AlignmentCheck::Symbolic;
        } else if arg == "-Zmiri-disable-isolation" {
            miri_config.isolated_op = IsolatedOp::Allow;
        } else if arg == "-Zmiri-disable-leak-backtraces" {
            miri_config.collect_leak_backtraces = false;
        } else if arg == "-Zmiri-disable-weak-memory-emulation" {
            miri_config.weak_memory_emulation = false;
        } else if arg == "-Zmiri-track-weak-memory-loads" {
            miri_config.track_outdated_loads = true;
        } else if let Some(param) = arg.strip_prefix("-Zmiri-isolation-error=") {
            miri_config.isolated_op = match param {
                "abort" => IsolatedOp::Reject(RejectOpWith::Abort),
                "hide" => IsolatedOp::Reject(RejectOpWith::NoWarning),
                "warn" => IsolatedOp::Reject(RejectOpWith::Warning),
                "warn-nobacktrace" => IsolatedOp::Reject(RejectOpWith::WarningWithoutBacktrace),
                _ => {
                    eprintln!(
                        "-Zmiri-isolation-error must be `abort`, `hide`, `warn`, or `warn-nobacktrace`"
                    );
                    std::process::exit(1);
                }
            };
        } else if arg == "-Zmiri-ignore-leaks" {
            miri_config.ignore_leaks = true;
            miri_config.collect_leak_backtraces = false;
        } else if arg == "-Zmiri-deterministic-floats" {
            miri_config.float_nondet = false;
        } else if arg == "-Zmiri-no-extra-rounding-error" {
            miri_config.float_rounding_error = FloatRoundingErrorMode::None;
        } else if arg == "-Zmiri-max-extra-rounding-error" {
            miri_config.float_rounding_error = FloatRoundingErrorMode::Max;
        } else if arg == "-Zmiri-no-short-fd-operations" {
            miri_config.short_fd_operations = false;
        } else if arg == "-Zmiri-strict-provenance" {
            miri_config.provenance_mode = ProvenanceMode::Strict;
        } else if arg == "-Zmiri-permissive-provenance" {
            miri_config.provenance_mode = ProvenanceMode::Permissive;
        } else if arg == "-Zmiri-mute-stdout-stderr" {
            miri_config.mute_stdout_stderr = true;
        } else if arg == "-Zmiri-fixed-schedule" {
            miri_config.fixed_scheduling = true;
        } else if arg == "-Zmiri-deterministic-concurrency" {
            miri_config.fixed_scheduling = true;
            miri_config.address_reuse_cross_thread_rate = 0.0;
            miri_config.cmpxchg_weak_failure_rate = 0.0;
            miri_config.weak_memory_emulation = false;
        } else if let Some(param) = arg.strip_prefix("-Zmiri-seed=") {
            let seed = param.parse::<u64>().unwrap_or_else(|_| {
                eprintln!("-Zmiri-seed must be an integer that fits into u64");
                std::process::exit(1);
            });
            miri_config.seed = Some(seed);
        } else if let Some(param) = arg.strip_prefix("-Zmiri-env-forward=") {
            miri_config.forwarded_env_vars.push(param.to_owned());
        } else if let Some(param) = arg.strip_prefix("-Zmiri-env-set=") {
            let Some((name, value)) = param.split_once('=') else {
                eprintln!("-Zmiri-env-set requires an argument of the form <name>=<value>");
                std::process::exit(1);
            };
            miri_config
                .set_env_vars
                .insert(name.to_owned(), value.to_owned());
        } else if let Some(param) = arg.strip_prefix("-Zmiri-track-pointer-tag=") {
            let ids: Vec<u64> = param
                .split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect();
            for id in ids {
                if let Some(tag) = BorTag::new(id) {
                    miri_config.tracked_pointer_tags.insert(tag);
                }
            }
        } else if let Some(param) = arg.strip_prefix("-Zmiri-track-alloc-id=") {
            let ids: Vec<NonZero<u64>> = param
                .split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect();
            miri_config
                .tracked_alloc_ids
                .extend(ids.into_iter().map(AllocId));
        } else if arg == "-Zmiri-track-alloc-accesses" {
            miri_config.track_alloc_accesses = true;
        } else if let Some(param) = arg.strip_prefix("-Zmiri-provenance-gc=") {
            let interval = param.parse::<u32>().unwrap_or_else(|_| {
                eprintln!("-Zmiri-provenance-gc requires a `u32`");
                std::process::exit(1);
            });
            miri_config.gc_interval = interval;
        } else if let Some(param) = arg.strip_prefix("-Zmiri-backtrace=") {
            miri_config.backtrace_style = match param {
                "0" => BacktraceStyle::Off,
                "1" => BacktraceStyle::Short,
                "full" => BacktraceStyle::Full,
                _ => {
                    eprintln!("-Zmiri-backtrace may only be 0, 1, or full");
                    std::process::exit(1);
                }
            };
        } else if let Some(param) = arg.strip_prefix("-Zmiri-num-cpus=") {
            let num_cpus = param.parse::<u32>().unwrap_or_else(|_| {
                eprintln!("-Zmiri-num-cpus requires a `u32`");
                std::process::exit(1);
            });
            miri_config.num_cpus = num_cpus;
        } else if arg.starts_with("-Zmiri-") {
            eprintln!(
                "error: unknown unstable option: `{}`",
                arg.strip_prefix("-Z").unwrap_or(&arg)
            );
            std::process::exit(1);
        } else {
            rustc_args.push(arg);
        }
    }

    if miri_config.validation == ValidationMode::No {
        miri_config.borrow_tracker = None;
    }

    (miri_config, rustc_args)
}

/// Called from after_analysis: run miri::eval_entry on the compiled TyCtxt.
fn interpret_with_miri(tcx: TyCtxt<'_>, mut config: MiriConfig) -> Compilation {
    if tcx.sess.dcx().has_errors_or_delayed_bugs().is_some() {
        tcx.dcx()
            .fatal("miri cannot run: the program failed to compile");
    }

    if co2_ast::diagnostics_were_emitted() {
        std::process::exit(5);
    }

    let (entry_def_id, entry_type) = entry_fn(tcx);

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
