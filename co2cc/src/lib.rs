#![feature(rustc_private)]

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use co2_driver_lib::{CompileMode, compile_co2_source};

struct CcArgs {
    emit_obj_only: bool,
    inputs: Vec<PathBuf>,
    output: Option<PathBuf>,
    opt_level: Option<String>,
    debuginfo: Option<u8>,
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
    let temp_dir = make_temp_dir();
    let mut has_stdin = false;

    let mut resolve_stdin = |input: &Path| -> PathBuf {
        if input == Path::new("-") {
            has_stdin = true;
            let stdin_path = temp_dir.join("stdin.c");
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .expect("failed to read stdin");
            fs::write(&stdin_path, &buf).expect("failed to write stdin temp file");
            stdin_path
        } else {
            input.to_path_buf()
        }
    };

    if args.emit_obj_only {
        let input = args
            .inputs
            .first()
            .cloned()
            .expect("missing C input file for object emission");
        let resolved = resolve_stdin(&input);
        let preprocessed = Arc::new(co2_preprocessor::preprocess(&resolved, &args.cpp_args));
        let rustc_args = build_rustc_object_args(
            &resolved,
            args.output.as_deref(),
            args.opt_level.as_deref(),
            args.debuginfo,
        );
        compile_co2_source(CompileMode::C, resolved, preprocessed, rustc_args);
        if has_stdin {
            let _ = fs::remove_dir_all(&temp_dir);
        }
        return;
    }

    let mut object_paths = Vec::with_capacity(args.inputs.len());
    for input in &args.inputs {
        let resolved = resolve_stdin(input);
        if resolved.extension().and_then(|e| e.to_str()) == Some("o") {
            object_paths.push(resolved);
            continue;
        }
        let object_path = temp_dir.join(
            resolved
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("co2c_object")
                .replace('-', "_")
                + ".o",
        );
        compile_c_to_object(
            &resolved,
            &object_path,
            &args.cpp_args,
            args.opt_level.as_deref(),
            args.debuginfo,
        );
        object_paths.push(object_path);
    }

    link_objects(
        &object_paths,
        &args.linker_args,
        args.output.as_deref(),
        args.opt_level.as_deref(),
        args.debuginfo,
    );
    let _ = fs::remove_dir_all(&temp_dir);
}

fn parse_args(args: &[String]) -> Result<CcArgs, String> {
    if args.len() < 2 {
        return Err("missing input file".to_owned());
    }

    let mut emit_obj_only = false;
    let mut inputs = Vec::new();
    let mut output = None;
    let mut opt_level = None;
    let mut debuginfo = None;
    let mut cpp_args = Vec::new();
    let mut linker_args = Vec::new();

    let mut i = 1usize;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "-c" => {
                emit_obj_only = true;
            }
            "-x" => {
                i += 1;
                let _lang = args.get(i).ok_or("missing argument after -x")?;
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
            _ if arg == "-g0" => debuginfo = Some(0),
            _ if arg == "-g1" => debuginfo = Some(1),
            _ if arg == "-g" || arg == "-g2" || arg == "-g3" => debuginfo = Some(2),
            _ if arg == "-O0" => opt_level = Some("0".to_owned()),
            _ if arg == "-O1" => opt_level = Some("1".to_owned()),
            _ if arg == "-O2" => opt_level = Some("2".to_owned()),
            _ if arg == "-O3" => opt_level = Some("3".to_owned()),
            _ if arg == "-Os" => opt_level = Some("s".to_owned()),
            _ if arg == "-Oz" => opt_level = Some("z".to_owned()),
            _ if arg == "-Og" => opt_level = Some("0".to_owned()),
            "-" => {
                inputs.push(PathBuf::from(arg));
            }
            _ if arg.starts_with('-') => {}
            _ => {
                let path = PathBuf::from(arg);
                match path.extension().and_then(|ext| ext.to_str()) {
                    Some("c" | "o") => inputs.push(path),
                    _ => linker_args.push(arg.clone()),
                }
            }
        }
        i += 1;
    }

    if inputs.is_empty() {
        return Err("missing input file".to_owned());
    }
    if emit_obj_only {
        let c_count = inputs
            .iter()
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("c") || p.as_os_str() == "-")
            .count();
        if c_count != 1 {
            return Err("object emission mode expects exactly one C input file".to_owned());
        }
    }
    Ok(CcArgs {
        emit_obj_only,
        inputs,
        output,
        opt_level,
        debuginfo,
        cpp_args,
        linker_args,
    })
}

fn build_rustc_object_args(
    input: &Path,
    output: Option<&Path>,
    opt_level: Option<&str>,
    debuginfo: Option<u8>,
) -> Vec<String> {
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

    if let Some(level) = opt_level {
        rustc_args.push("-C".to_owned());
        rustc_args.push(format!("opt-level={level}"));
    }

    let dbg = debuginfo.unwrap_or(0);
    rustc_args.push("-C".to_owned());
    rustc_args.push(format!("debuginfo={dbg}"));

    rustc_args.extend(shared_rust_flags());

    rustc_args
}

fn build_link_rustc_args(
    link_stub: &Path,
    objects: &[PathBuf],
    linker_args: &[String],
    output: Option<&Path>,
    opt_level: Option<&str>,
    debuginfo: Option<u8>,
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

    if let Some(level) = opt_level {
        rustc_args.push("-C".to_owned());
        rustc_args.push(format!("opt-level={level}"));
    }

    let dbg = debuginfo.unwrap_or(0);
    rustc_args.push("-C".to_owned());
    rustc_args.push(format!("debuginfo={dbg}"));

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

fn compile_c_to_object(
    input: &Path,
    output: &Path,
    cpp_args: &[String],
    opt_level: Option<&str>,
    debuginfo: Option<u8>,
) {
    let exe = current_invocation_path()
        .or_else(|| std::env::current_exe().ok())
        .expect("failed to locate co2c executable");
    let mut cmd = Command::new(exe);
    cmd.arg("-c").arg(input).arg("-o").arg(output);
    if let Some(level) = opt_level {
        cmd.arg(format!("-O{level}"));
    }
    if debuginfo.is_some() {
        cmd.arg("-g");
    }
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

fn link_objects(
    objects: &[PathBuf],
    linker_args: &[String],
    output: Option<&Path>,
    opt_level: Option<&str>,
    debuginfo: Option<u8>,
) {
    let temp_dir = make_temp_dir();
    let link_stub = temp_dir.join("co2c_link.rs");
    fs::write(&link_stub, "#![no_main]\n").expect("failed to write linker stub");

    let rustc_args = build_link_rustc_args(
        &link_stub,
        objects,
        linker_args,
        output,
        opt_level,
        debuginfo,
    );
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
