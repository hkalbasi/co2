#![feature(rustc_private)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use co2_driver_lib::{CompileMode, compile_co2_source};

struct CcArgs {
    emit_obj_only: bool,
    inputs: Vec<PathBuf>,
    output: Option<PathBuf>,
    cpp_args: Vec<String>,
    linker_args: Vec<String>,
}

pub fn main() -> std::process::ExitCode {
    main_with_args(std::env::args().collect())
}

pub fn main_with_args(args: Vec<String>) -> std::process::ExitCode {
    let cc_args = match parse_args(&args) {
        Ok(args) => args,
        Err(msg) => {
            eprintln!("co2cc: {msg}");
            return std::process::ExitCode::from(2);
        }
    };

    if let Err(payload) = std::panic::catch_unwind(|| run_co2c(cc_args)) {
        if co2_ast::is_diagnostic_abort(payload.as_ref()) {
            return std::process::ExitCode::from(5);
        }
        if let Some(msg) = payload.downcast_ref::<String>() {
            eprintln!("co2cc panic: {msg}");
        } else if let Some(msg) = payload.downcast_ref::<&str>() {
            eprintln!("co2cc panic: {msg}");
        } else {
            eprintln!("co2cc panic: non-string payload");
        }
        return std::process::ExitCode::from(101);
    }

    std::process::ExitCode::SUCCESS
}

fn run_co2c(args: CcArgs) {
    if args.emit_obj_only {
        let input = args
            .inputs
            .first()
            .cloned()
            .expect("missing C input file for object emission");
        let preprocessed = Arc::new(co2_preprocessor::preprocess(&input, &args.cpp_args));
        let rustc_args = build_rustc_object_args(&input, args.output.as_deref());
        compile_co2_source(CompileMode::C, input, preprocessed, rustc_args);
        return;
    }

    let temp_dir = make_temp_dir();
    let mut object_paths = Vec::with_capacity(args.inputs.len());
    for input in &args.inputs {
        let object_path = temp_dir.join(
            input
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("co2c_object")
                .replace('-', "_")
                + ".o",
        );
        compile_c_to_object(input, &object_path, &args.cpp_args);
        object_paths.push(object_path);
    }

    link_objects(&object_paths, &args.linker_args, args.output.as_deref());
    let _ = fs::remove_dir_all(&temp_dir);
}

fn parse_args(args: &[String]) -> Result<CcArgs, String> {
    if args.len() < 2 {
        return Err("missing input file".to_owned());
    }

    let mut emit_obj_only = false;
    let mut inputs = Vec::new();
    let mut output = None;
    let mut cpp_args = Vec::new();
    let mut linker_args = Vec::new();

    let mut i = 1usize;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "--co2c-emit-obj" => {
                emit_obj_only = true;
            }
            "-o" => {
                i += 1;
                let out = args.get(i).ok_or("missing argument after -o")?;
                output = Some(PathBuf::from(out));
            }
            "-I" | "-D" | "-U" | "-include" | "-isystem" | "-iquote" => {
                let flag = arg.clone();
                i += 1;
                let val = args
                    .get(i)
                    .ok_or_else(|| format!("missing argument after {flag}"))?;
                cpp_args.push(flag);
                cpp_args.push(val.clone());
            }
            "-nostdinc" | "-undef" => {
                cpp_args.push(arg.clone());
            }
            "-l" | "-L" | "-Wl" | "-Xlinker" => {
                let flag = arg.clone();
                i += 1;
                let val = args
                    .get(i)
                    .ok_or_else(|| format!("missing argument after {flag}"))?;
                linker_args.push(flag);
                linker_args.push(val.clone());
            }
            _ if arg.starts_with("-I")
                || arg.starts_with("-D")
                || arg.starts_with("-U")
                || arg.starts_with("-std=")
                || arg.starts_with("-isystem")
                || arg.starts_with("-iquote")
                || arg.starts_with("-include") =>
            {
                cpp_args.push(arg.clone());
            }
            _ if arg.starts_with("-l")
                || arg.starts_with("-L")
                || arg.starts_with("-Wl,")
                || arg == "-pthread" =>
            {
                linker_args.push(arg.clone());
            }
            _ if arg.starts_with('-') => {}
            _ => {
                let path = PathBuf::from(arg);
                if path.extension().and_then(|ext| ext.to_str()) == Some("c") {
                    inputs.push(path);
                } else {
                    linker_args.push(arg.clone());
                }
            }
        }
        i += 1;
    }

    if inputs.is_empty() {
        return Err("missing C input file".to_owned());
    }
    if emit_obj_only && inputs.len() != 1 {
        return Err("object emission mode expects exactly one C input file".to_owned());
    }
    Ok(CcArgs {
        emit_obj_only,
        inputs,
        output,
        cpp_args,
        linker_args,
    })
}

fn build_rustc_object_args(input: &Path, output: Option<&Path>) -> Vec<String> {
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("co2c_out")
        .replace('-', "_");

    let mut rustc_args = vec![
        "rustc".to_owned(),
        "--crate-name".to_owned(),
        stem,
        "--crate-type=bin".to_owned(),
        "--edition=2024".to_owned(),
        "--emit=obj".to_owned(),
    ];

    if let Some(out) = output {
        rustc_args.push("-o".to_owned());
        rustc_args.push(out.to_string_lossy().into_owned());
    }

    rustc_args.push("/dev/null".to_owned());

    rustc_args.extend(shared_rust_flags());

    rustc_args
}

fn build_link_rustc_args(
    link_stub: &Path,
    objects: &[PathBuf],
    linker_args: &[String],
    output: Option<&Path>,
) -> Vec<String> {
    let mut rustc_args = vec![
        "--crate-name".to_owned(),
        "co2c_link".to_owned(),
        "--crate-type=bin".to_owned(),
        "--edition=2024".to_owned(),
    ];

    if let Some(out) = output {
        rustc_args.push("-o".to_owned());
        rustc_args.push(out.to_string_lossy().into_owned());
    } else {
        rustc_args.push("-o".to_owned());
        rustc_args.push("a.out".to_owned());
    }

    for object in objects {
        rustc_args.push("-C".to_owned());
        rustc_args.push(format!("link-arg={}", object.to_string_lossy()));
    }
    for arg in linker_args {
        rustc_args.push("-C".to_owned());
        rustc_args.push(format!("link-arg={arg}"));
    }

    rustc_args.extend(shared_rust_flags());
    rustc_args.push(link_stub.to_string_lossy().into_owned());
    rustc_args
}

fn shared_rust_flags() -> Vec<String> {
    let mut flags = std::env::var("RUSTFLAGS")
        .ok()
        .map(|flags| {
            flags
                .split_ascii_whitespace()
                .map(|x| x.to_owned())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    if std::env::var_os("CO2_FORCE_JSON_DIAGNOSTICS").is_some()
        && !flags.iter().any(|arg| arg == "--error-format=json")
    {
        flags.push("--error-format=json".to_owned());
    }

    flags
}

fn compile_c_to_object(input: &Path, output: &Path, cpp_args: &[String]) {
    let exe = current_invocation_path()
        .or_else(|| std::env::current_exe().ok())
        .expect("failed to locate co2c executable");
    let mut cmd = Command::new(exe);
    cmd.arg("--co2c-emit-obj").arg(input).arg("-o").arg(output);
    for arg in cpp_args {
        cmd.arg(arg);
    }

    let status = cmd.status().expect("failed to execute co2c object compile");
    if !status.success() {
        if status.code() == Some(5) {
            co2_ast::panic_with_diagnostic_abort();
        }
        panic!("co2c object compile failed with status {status}");
    }
}

fn current_invocation_path() -> Option<PathBuf> {
    std::env::args_os().next().map(PathBuf::from)
}

fn link_objects(objects: &[PathBuf], linker_args: &[String], output: Option<&Path>) {
    let temp_dir = make_temp_dir();
    let link_stub = temp_dir.join("co2c_link.rs");
    fs::write(&link_stub, "#![no_main]\n").expect("failed to write linker stub");

    let rustc_args = build_link_rustc_args(&link_stub, objects, linker_args, output);
    let exe = std::env::var_os("CO2_RUN_SCRIPT")
        .map(PathBuf::from)
        .or_else(|| current_invocation_path())
        .or_else(|| std::env::current_exe().ok())
        .expect("failed to locate co2c executable");

    let mut cmd = Command::new(exe);
    cmd.args(&rustc_args);
    // Force the applet name to co2rustc
    cmd.env("CO2_APPLET_OVERRIDE", "co2rustc");

    let status = cmd.status().expect("failed to execute co2rustc link step");
    let _ = fs::remove_file(&link_stub);
    let _ = fs::remove_dir_all(&temp_dir);
    if !status.success() {
        panic!("rustc link failed with status {status}");
    }
}

fn make_temp_dir() -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(".co2c-{}-{unique}", std::process::id()));
    fs::create_dir_all(&path).expect("failed to create temporary directory");
    path
}
