#![feature(rustc_private)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
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
        let preprocessed = preprocess(&input, &args.cpp_args);
        let normalized = normalize_preprocessed(&preprocessed);
        let rustc_args = build_rustc_object_args(&input, args.output.as_deref());
        compile_co2_source(CompileMode::C, input, normalized, rustc_args);
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

fn preprocess(input: &Path, cpp_args: &[String]) -> String {
    let rewritten_input = rewrite_main_source_for_preprocess(input);
    let mut cmd = Command::new("gcc");
    cmd.arg("-E");
    cmd.arg(&rewritten_input);
    for arg in cpp_args {
        cmd.arg(arg);
    }

    let out = cmd.output().expect("failed to execute gcc -E");
    let _ = fs::remove_file(&rewritten_input);
    if !out.status.success() {
        eprintln!("{}", String::from_utf8_lossy(&out.stderr));
        panic!("gcc -E failed with status {}", out.status);
    }
    String::from_utf8(out.stdout).expect("gcc -E produced non-utf8 output")
}

fn rewrite_main_source_for_preprocess(input: &Path) -> PathBuf {
    let source = fs::read_to_string(input).expect("failed to read C input for preprocessing");
    let mut rewritten = String::from("#define __CO2__ 1\n");
    for line in source.lines() {
        if line.trim_start().starts_with('#') {
            rewritten.push_str(&line.replace("__GNUC__", "__CO2_HIDDEN_GNUC__").replace(
                "__clang__",
                "__CO2_HIDDEN_CLANG__",
            ));
        } else {
            rewritten.push_str(line);
        }
        rewritten.push('\n');
    }

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_nanos();
    let rewritten_path = input.with_file_name(format!(
        ".co2c-preprocess-{}-{unique}.c",
        std::process::id()
    ));
    fs::write(&rewritten_path, rewritten).expect("failed to write rewritten C input");
    rewritten_path
}

fn normalize_preprocessed(text: &str) -> String {
    let text = strip_line_markers(text);
    let text = strip_gnu_attributes(&text);
    let text = strip_gnu_asm_annotations(&text);
    let text = replace_gnu_typeof_with_usize(&text);
    strip_extension_keywords(&text)
}

fn strip_line_markers(preprocessed: &str) -> String {
    let mut out = String::new();
    for line in preprocessed.lines() {
        if !is_line_marker(line) {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

fn is_line_marker(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with('#')
}

fn strip_gnu_attributes(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0usize;

    while i < bytes.len() {
        if src[i..].starts_with("__attribute__") {
            let mut j = i + "__attribute__".len();
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'(' {
                let mut depth = 0usize;
                while j < bytes.len() {
                    let b = bytes[j];
                    if b == b'(' {
                        depth += 1;
                    } else if b == b')' {
                        depth = depth.saturating_sub(1);
                        if depth == 0 {
                            j += 1;
                            break;
                        }
                    }
                    j += 1;
                }
                out.push(' ');
                i = j;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }

    out
}

fn strip_extension_keywords(src: &str) -> String {
    fn is_ident_start(b: u8) -> bool {
        b.is_ascii_alphabetic() || b == b'_'
    }
    fn is_ident_continue(b: u8) -> bool {
        b.is_ascii_alphanumeric() || b == b'_'
    }

    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0usize;

    while i < bytes.len() {
        if is_ident_start(bytes[i]) {
            let start = i;
            i += 1;
            while i < bytes.len() && is_ident_continue(bytes[i]) {
                i += 1;
            }
            let ident = &src[start..i];
            let strip = matches!(
                ident,
                "__extension__"
                    | "__inline"
                    | "__inline__"
                    | "__restrict"
                    | "__restrict__"
                    | "_Complex"
                    | "_Noreturn"
            );
            if strip {
                out.push(' ');
            } else {
                out.push_str(ident);
            }
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }

    out
}

fn strip_gnu_asm_annotations(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0usize;

    while i < bytes.len() {
        if src[i..].starts_with("__asm__") || src[i..].starts_with("__asm") {
            let kw_len = if src[i..].starts_with("__asm__") {
                "__asm__".len()
            } else {
                "__asm".len()
            };
            let mut j = i + kw_len;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'(' {
                let mut depth = 0usize;
                while j < bytes.len() {
                    let b = bytes[j];
                    if b == b'(' {
                        depth += 1;
                    } else if b == b')' {
                        depth = depth.saturating_sub(1);
                        if depth == 0 {
                            j += 1;
                            break;
                        }
                    }
                    j += 1;
                }
                out.push(' ');
                i = j;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }

    out
}

fn replace_gnu_typeof_with_usize(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0usize;

    while i < bytes.len() {
        let kw_len = if src[i..].starts_with("__typeof__") {
            Some("__typeof__".len())
        } else if src[i..].starts_with("__typeof") {
            Some("__typeof".len())
        } else if src[i..].starts_with("typeof") {
            Some("typeof".len())
        } else {
            None
        };
        if let Some(kw_len) = kw_len {
            let mut j = i + kw_len;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'(' {
                let mut depth = 0usize;
                while j < bytes.len() {
                    let b = bytes[j];
                    if b == b'(' {
                        depth += 1;
                    } else if b == b')' {
                        depth = depth.saturating_sub(1);
                        if depth == 0 {
                            j += 1;
                            break;
                        }
                    }
                    j += 1;
                }
                out.push_str("usize");
                i = j;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }

    out
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
    std::env::var("RUSTFLAGS")
        .ok()
        .map(|flags| {
            flags
                .split_ascii_whitespace()
                .map(|x| x.to_owned())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn compile_c_to_object(input: &Path, output: &Path, cpp_args: &[String]) {
    let exe = current_invocation_path()
        .or_else(|| std::env::current_exe().ok())
        .expect("failed to locate co2c executable");
    let mut cmd = Command::new(exe);
    cmd.arg("--co2c-emit-obj")
        .arg(input)
        .arg("-o")
        .arg(output);
    for arg in cpp_args {
        cmd.arg(arg);
    }

    let status = cmd.status().expect("failed to execute co2c object compile");
    if !status.success() {
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
