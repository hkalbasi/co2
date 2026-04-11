#![feature(rustc_private)]

use std::path::{Path, PathBuf};
use std::process::Command;

use co2_driver_lib::{CompileMode, compile_co2_source};

struct CcArgs {
    input: PathBuf,
    output: Option<PathBuf>,
    cpp_args: Vec<String>,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cc_args = parse_args(&args).unwrap_or_else(|msg| {
        eprintln!("co2c: {msg}");
        std::process::exit(2);
    });

    if let Err(payload) = std::panic::catch_unwind(|| run_co2c(cc_args)) {
        if let Some(msg) = payload.downcast_ref::<String>() {
            eprintln!("co2c panic: {msg}");
        } else if let Some(msg) = payload.downcast_ref::<&str>() {
            eprintln!("co2c panic: {msg}");
        } else {
            eprintln!("co2c panic: non-string payload");
        }
        std::process::exit(101);
    }
}

fn run_co2c(args: CcArgs) {
    let preprocessed = preprocess(&args.input, &args.cpp_args);
    let normalized = normalize_preprocessed(&preprocessed);
    let rustc_args = build_rustc_args(&args);
    compile_co2_source(CompileMode::C, args.input, normalized, rustc_args);
}

fn parse_args(args: &[String]) -> Result<CcArgs, String> {
    if args.len() < 2 {
        return Err("missing input file".to_owned());
    }

    let mut input = None;
    let mut output = None;
    let mut cpp_args = Vec::new();

    let mut i = 1usize;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
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
            _ if arg.starts_with('-') => {}
            _ => {
                input = Some(PathBuf::from(arg));
            }
        }
        i += 1;
    }

    let input = input.ok_or("missing C input file")?;
    Ok(CcArgs {
        input,
        output,
        cpp_args,
    })
}

fn preprocess(input: &Path, cpp_args: &[String]) -> String {
    let mut cmd = Command::new("gcc");
    cmd.arg("-E");
    cmd.arg(input);
    for arg in cpp_args {
        cmd.arg(arg);
    }

    let out = cmd.output().expect("failed to execute gcc -E");
    if !out.status.success() {
        eprintln!("{}", String::from_utf8_lossy(&out.stderr));
        panic!("gcc -E failed with status {}", out.status);
    }
    String::from_utf8(out.stdout).expect("gcc -E produced non-utf8 output")
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

fn build_rustc_args(args: &CcArgs) -> Vec<String> {
    let stem = args
        .input
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
    ];

    if let Some(out) = &args.output {
        rustc_args.push("-o".to_owned());
        rustc_args.push(out.to_string_lossy().into_owned());
    }

    rustc_args.push("-l".to_owned());
    rustc_args.push("c".to_owned());
    rustc_args.push("/dev/null".to_owned());

    if let Ok(rust_flags) = std::env::var("RUSTFLAGS") {
        rustc_args.extend(rust_flags.split_ascii_whitespace().map(|x| x.to_owned()));
    }

    rustc_args
}
