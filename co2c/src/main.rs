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
    let filtered = filter_to_main_file(&preprocessed, &args.input);
    let rustc_args = build_rustc_args(&args);
    compile_co2_source(CompileMode::C, args.input, filtered, rustc_args);
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
                let val = args.get(i).ok_or_else(|| format!("missing argument after {flag}"))?;
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
    cmd.arg("-fdirectives-only");
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

fn filter_to_main_file(preprocessed: &str, input: &Path) -> String {
    let input_canon = input.canonicalize().unwrap_or_else(|_| input.to_path_buf());
    let input_name = input.file_name().map(|s| s.to_string_lossy().into_owned());
    let mut keep = true;
    let mut out = String::new();

    for line in preprocessed.lines() {
        if let Some(path) = parse_line_marker(line) {
            keep = matches_input(&path, &input_canon, input_name.as_deref());
            continue;
        }
        if keep {
            out.push_str(line);
            out.push('\n');
        }
    }

    out
}

fn parse_line_marker(line: &str) -> Option<PathBuf> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('#') {
        return None;
    }
    let mut parts = trimmed.split('"');
    parts.next()?;
    let file = parts.next()?;
    Some(PathBuf::from(file))
}

fn matches_input(marker: &Path, input_canon: &Path, input_name: Option<&str>) -> bool {
    if marker == input_canon {
        return true;
    }
    if marker.ends_with(input_canon) {
        return true;
    }
    if let Some(name) = input_name {
        if marker.file_name().map(|s| s.to_string_lossy()) == Some(name.into()) {
            return true;
        }
    }
    let canon = marker.canonicalize().ok();
    canon.as_deref() == Some(input_canon)
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
    rustc_args
}
