#![feature(rustc_private)]

use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use co2_driver_lib::{CompileMode, compile_co2_source};

#[derive(Clone, Copy)]
enum LinkOutputKind {
    Executable,
    SharedLib,
}

struct CcArgs {
    emit_obj_only: bool,
    link_kind: LinkOutputKind,
    pic: bool,
    inputs: Vec<PathBuf>,
    output: Option<PathBuf>,
    opt_level: Option<String>,
    debuginfo: Option<u8>,
    cpp_args: Vec<String>,
    linker_args: Vec<String>,
}

#[derive(Debug)]
enum ParseArgsError {
    MissingInputFile,
    MissingArgumentAfter(String),
    InvalidObjectEmissionInputs,
}

impl fmt::Display for ParseArgsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingInputFile => f.write_str("missing input file"),
            Self::MissingArgumentAfter(flag) => write!(f, "missing argument after {flag}"),
            Self::InvalidObjectEmissionInputs => {
                f.write_str("object emission mode expects exactly one C input file")
            }
        }
    }
}

pub fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().collect();
    main_with_args(&args)
}

pub fn main_with_args(args: &[String]) -> std::process::ExitCode {
    let cc_args = match parse_args(args) {
        Ok(args) => args,
        Err(msg) => {
            eprintln!("co2cc: {msg}");
            return std::process::ExitCode::from(2);
        }
    };

    if let Err(payload) = std::panic::catch_unwind(|| run_co2c(&cc_args)) {
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

fn run_co2c(args: &CcArgs) {
    let temp_dir = make_temp_dir();
    let mut has_stdin = false;
    let force_pic = args.pic || matches!(args.link_kind, LinkOutputKind::SharedLib);

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
            force_pic,
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
            force_pic,
        );
        object_paths.push(object_path);
    }

    link_objects(
        &object_paths,
        &args.linker_args,
        args.output.as_deref(),
        args.opt_level.as_deref(),
        args.debuginfo,
        args.link_kind,
    );
    let _ = fs::remove_dir_all(&temp_dir);
}

fn parse_args(args: &[String]) -> Result<CcArgs, ParseArgsError> {
    if args.len() < 2 {
        return Err(ParseArgsError::MissingInputFile);
    }

    let mut emit_obj_only = false;
    let mut link_kind = LinkOutputKind::Executable;
    let mut pic = false;
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
            "-shared" => {
                link_kind = LinkOutputKind::SharedLib;
            }
            "-x" => {
                i += 1;
                let _lang = args
                    .get(i)
                    .ok_or_else(|| ParseArgsError::MissingArgumentAfter("-x".to_owned()))?;
            }
            "-o" => {
                i += 1;
                let out = args
                    .get(i)
                    .ok_or_else(|| ParseArgsError::MissingArgumentAfter("-o".to_owned()))?;
                output = Some(PathBuf::from(out));
            }
            "-I" | "-D" | "-U" | "-include" | "-isystem" | "-iquote" => {
                let flag = arg.clone();
                i += 1;
                let val = args
                    .get(i)
                    .ok_or_else(|| ParseArgsError::MissingArgumentAfter(flag.clone()))?;
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
                    .ok_or_else(|| ParseArgsError::MissingArgumentAfter(flag.clone()))?;
                linker_args.push(flag);
                linker_args.push(val.clone());
            }
            "-fPIC" | "-fpic" => pic = true,
            "-g0" => debuginfo = Some(0),
            "-g1" => debuginfo = Some(1),
            "-g" | "-g2" | "-g3" => debuginfo = Some(2),
            "-O0" | "-Og" => opt_level = Some("0".to_owned()),
            "-O1" => opt_level = Some("1".to_owned()),
            "-O2" => opt_level = Some("2".to_owned()),
            "-O3" => opt_level = Some("3".to_owned()),
            "-Os" => opt_level = Some("s".to_owned()),
            "-Oz" => opt_level = Some("z".to_owned()),
            "-" => {
                inputs.push(PathBuf::from(arg));
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
                match path.extension().and_then(|ext| ext.to_str()) {
                    Some("c" | "o") => inputs.push(path),
                    _ => linker_args.push(arg.clone()),
                }
            }
        }
        i += 1;
    }

    if inputs.is_empty() {
        return Err(ParseArgsError::MissingInputFile);
    }
    if emit_obj_only {
        let c_count = inputs
            .iter()
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("c") || p.as_os_str() == "-")
            .count();
        if c_count != 1 {
            return Err(ParseArgsError::InvalidObjectEmissionInputs);
        }
    }
    Ok(CcArgs {
        emit_obj_only,
        link_kind,
        pic,
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
    pic: bool,
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

    rustc_args.push(no_main_host_file().to_string_lossy().into_owned());

    if let Some(level) = opt_level {
        rustc_args.push("-C".to_owned());
        rustc_args.push(format!("opt-level={level}"));
    }

    let dbg = debuginfo.unwrap_or(0);
    rustc_args.push("-C".to_owned());
    rustc_args.push(format!("debuginfo={dbg}"));

    if pic {
        rustc_args.push("-C".to_owned());
        rustc_args.push("relocation-model=pic".to_owned());
    }

    rustc_args.extend(shared_rust_flags());

    rustc_args
}

fn no_main_host_file() -> PathBuf {
    let path = std::env::temp_dir().join(format!("co2cc-no-main-{}.rs", std::process::id()));
    let _ = fs::write(&path, "#![no_main]\n");
    path
}

fn build_link_stub_rustc_args(
    link_stub: &Path,
    output: &Path,
    debuginfo: Option<u8>,
) -> Vec<String> {
    let mut rustc_args = vec![
        "--crate-name".to_owned(),
        "co2c_link_stub".to_owned(),
        "--crate-type=bin".to_owned(),
        "--edition=2024".to_owned(),
        "--emit=obj".to_owned(),
        "-o".to_owned(),
        output.to_string_lossy().into_owned(),
    ];

    rustc_args.push("-C".to_owned());
    rustc_args.push("opt-level=2".to_owned());

    let dbg = debuginfo.unwrap_or(0);
    rustc_args.push("-C".to_owned());
    rustc_args.push(format!("debuginfo={dbg}"));

    rustc_args.push("-C".to_owned());
    rustc_args.push("panic=abort".to_owned());

    rustc_args.extend(shared_rust_flags());
    rustc_args.push(link_stub.to_string_lossy().into_owned());
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
        "-o".to_owned(),
        output
            .unwrap_or_else(|| Path::new("a.out"))
            .to_string_lossy()
            .into_owned(),
    ];

    for object in objects {
        rustc_args.push("-C".to_owned());
        rustc_args.push(format!("link-arg={}", object.to_string_lossy()));
    }
    rustc_args.push("-C".to_owned());
    rustc_args.push("link-arg=-lc".to_owned());
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

    rustc_args.push("-C".to_owned());
    rustc_args.push("panic=abort".to_owned());

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
                .map(std::borrow::ToOwned::to_owned)
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
    pic: bool,
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
    if pic {
        cmd.arg("-fPIC");
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

fn should_try_direct_cc_link(linker_args: &[String]) -> bool {
    linker_args
        .iter()
        .any(|arg| Path::new(arg).extension().and_then(|ext| ext.to_str()) == Some("a"))
}

fn link_objects(
    objects: &[PathBuf],
    linker_args: &[String],
    output: Option<&Path>,
    opt_level: Option<&str>,
    debuginfo: Option<u8>,
    link_kind: LinkOutputKind,
) {
    if matches!(link_kind, LinkOutputKind::SharedLib) {
        link_shared_objects(objects, linker_args, output);
        return;
    }

    if !should_try_direct_cc_link(linker_args) {
        let temp_dir = make_temp_dir();
        let rustc_link_stub = temp_dir.join("co2c_link.rs");
        fs::write(&rustc_link_stub, CO2C_LINK_STUB).expect("failed to write rustc linker stub");
        let rustc_link_args = build_link_rustc_args(
            &rustc_link_stub,
            objects,
            linker_args,
            output,
            opt_level,
            debuginfo,
        );
        let exe = std::env::var_os("CO2_RUN_SCRIPT")
            .map(PathBuf::from)
            .or_else(current_invocation_path)
            .or_else(|| std::env::current_exe().ok())
            .expect("failed to locate co2c executable");
        let mut rustc_link_cmd = Command::new(&exe);
        rustc_link_cmd.args(&rustc_link_args);
        rustc_link_cmd.env("CO2_APPLET_OVERRIDE", "co2rustc");

        let status = rustc_link_cmd
            .status()
            .expect("failed to execute co2rustc link step");
        let _ = fs::remove_file(&rustc_link_stub);
        let _ = fs::remove_dir_all(&temp_dir);
        assert!(status.success(), "rustc link failed with status {status}");
        return;
    }

    let temp_dir = make_temp_dir();
    let cc_link_stub = temp_dir.join("co2c_cc_link.rs");
    let cc_link_stub_object = temp_dir.join("co2c_cc_link.o");
    fs::write(&cc_link_stub, CO2C_CC_LINK_STUB).expect("failed to write linker stub");

    let rustc_args = build_link_stub_rustc_args(&cc_link_stub, &cc_link_stub_object, debuginfo);
    let exe = std::env::var_os("CO2_RUN_SCRIPT")
        .map(PathBuf::from)
        .or_else(current_invocation_path)
        .or_else(|| std::env::current_exe().ok())
        .expect("failed to locate co2c executable");

    let mut stub_cmd = Command::new(&exe);
    stub_cmd.args(&rustc_args);
    stub_cmd.env("CO2_APPLET_OVERRIDE", "co2rustc");

    let stub_status = stub_cmd
        .status()
        .expect("failed to compile co2c panic stub object");
    assert!(
        stub_status.success(),
        "rustc panic stub compile failed with status {stub_status}"
    );

    let mut link_cmd = Command::new("cc");
    for object in objects {
        link_cmd.arg(object);
    }
    link_cmd.arg(&cc_link_stub_object);
    for arg in linker_args {
        link_cmd.arg(arg);
    }
    link_cmd.arg("-o");
    link_cmd.arg(output.unwrap_or_else(|| Path::new("a.out")));

    let link_output = link_cmd
        .output()
        .expect("failed to execute executable link step");
    if link_output.status.success() {
        let _ = fs::remove_file(&cc_link_stub);
        let _ = fs::remove_file(&cc_link_stub_object);
        let _ = fs::remove_dir_all(&temp_dir);
        return;
    }

    let cc_stderr = String::from_utf8_lossy(&link_output.stderr);
    if !cc_stderr.contains("undefined reference to `core::")
        && !cc_stderr.contains("undefined symbol: core::")
    {
        let _ = fs::remove_file(&cc_link_stub);
        let _ = fs::remove_file(&cc_link_stub_object);
        let _ = fs::remove_dir_all(&temp_dir);
        panic!(
            "executable link failed with status {}\n{}",
            link_output.status, cc_stderr
        );
    }

    let rustc_link_stub = temp_dir.join("co2c_link.rs");
    fs::write(&rustc_link_stub, CO2C_LINK_STUB).expect("failed to write rustc linker stub");
    let rustc_link_args = build_link_rustc_args(
        &rustc_link_stub,
        objects,
        linker_args,
        output,
        opt_level,
        debuginfo,
    );
    let mut rustc_link_cmd = Command::new(&exe);
    rustc_link_cmd.args(&rustc_link_args);
    rustc_link_cmd.env("CO2_APPLET_OVERRIDE", "co2rustc");

    let status = rustc_link_cmd
        .status()
        .expect("failed to execute co2rustc link step");
    let _ = fs::remove_file(&cc_link_stub);
    let _ = fs::remove_file(&cc_link_stub_object);
    let _ = fs::remove_file(&rustc_link_stub);
    let _ = fs::remove_dir_all(&temp_dir);
    assert!(status.success(), "rustc link failed with status {status}");
}

const CO2C_CC_LINK_STUB: &str = r#"#![no_std]
#![no_main]

use core::ffi::{c_int, c_void};

unsafe extern "C" {
    fn fwrite(ptr: *const c_void, size: usize, nmemb: usize, stream: *mut c_void) -> usize;
    fn exit(code: c_int) -> !;

    static mut stderr: *mut c_void;
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    unsafe {
        fwrite(c"co2cc panic\n".as_ptr().cast(), 1, 12, stderr);
        exit(134);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_eh_personality() {}
"#;

const CO2C_LINK_STUB: &str = r#"#![no_std]
#![no_main]

use core::ffi::{c_int, c_void, c_char};

unsafe extern "C" {
    fn fprintf(stream: *mut c_void, fmt: *const c_char, ...) -> c_int;
    fn exit(code: c_int) -> !;
    fn getpid() -> c_int;

    static mut stderr: *mut c_void;
}


#[panic_handler]
fn panic(info: &core::panic::PanicInfo<'_>) -> ! {
    unsafe {
        let pid = getpid();

        if let Some(loc) = info.location() {
            let file = loc.file();

            fprintf(
                stderr,
                c"thread '<unnamed>' (%d) panicked at %.*s:%d:%d:\n".as_ptr(),
                pid,
                file.len() as c_int,
                file.as_ptr(),
                loc.line(),
                loc.column(),
            );
        } else {
            fprintf(
                stderr,
                c"thread '<unnamed>' (%d) panicked:\n".as_ptr(),
                pid,
            );
        }

        if let Some(msg) = info.message().as_str() {
            fprintf(
                stderr,
                c"%.*s\n".as_ptr(),
                msg.len() as c_int,
                msg.as_ptr(),
            );
        } else {
            fprintf(
                stderr,
                c"<non-string panic>\n".as_ptr(),
            );
        }

        fprintf(
            stderr,
            c"%s\n".as_ptr(),
            c"thread caused non-unwinding panic. aborting.".as_ptr(),
        );

        exit(134);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_eh_personality() {}
"#;

fn link_shared_objects(objects: &[PathBuf], linker_args: &[String], output: Option<&Path>) {
    let mut cmd = Command::new("cc");
    cmd.arg("-shared");

    for object in objects {
        cmd.arg(object);
    }
    for arg in linker_args {
        cmd.arg(arg);
    }

    cmd.arg("-o");
    cmd.arg(output.unwrap_or_else(|| Path::new("a.out")));

    let status = cmd
        .status()
        .expect("failed to execute shared library link step");
    assert!(
        status.success(),
        "shared library link failed with status {status}"
    );
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
