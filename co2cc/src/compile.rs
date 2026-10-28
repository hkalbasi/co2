use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use co2_driver_lib::{CompileMode, compile_co2_source};

use crate::args::{CcArgs, LinkOutputKind, preprocess_args};
use crate::depfile::write_dep_file;
use crate::link::current_invocation_path;
use crate::rustc::{build_rustc_asm_args, build_rustc_object_args};

pub(crate) fn run_co2c(args: &CcArgs) {
    let temp_dir = tempfile::Builder::new()
        .prefix(".co2cc-")
        .tempdir()
        .expect("failed to create temporary directory");
    let force_pic = args.pic || matches!(args.link_kind, LinkOutputKind::SharedLib);

    // `cc -E -` resolves quoted includes relative to CWD, but stdin is
    // spilled to $TMPDIR/stdin.c so its dir would mis-resolve. Prepend
    // CWD as -iquote so stdin behaves like gcc.
    let mut stdin_cpp_args = Vec::new();
    if args.inputs.iter().any(|p| p == Path::new("-"))
        && let Ok(cwd) = std::env::current_dir()
    {
        stdin_cpp_args.push("-iquote".to_owned());
        stdin_cpp_args.push(cwd.to_string_lossy().into_owned());
        stdin_cpp_args.extend(args.cpp_args.iter().cloned());
    }
    let cpp_args: &[String] = if stdin_cpp_args.is_empty() {
        &args.cpp_args
    } else {
        &stdin_cpp_args
    };

    // rustc and the linker create temporary files next to the output path,
    // which fails for special files like /dev/null. Redirect the output to a
    // temp file in that case; compilation still runs and its result is
    // discarded.
    let output = if args.output.as_deref() == Some(Path::new("/dev/null")) {
        Some(temp_dir.path().join("co2c_dev_null_output"))
    } else {
        args.output.clone()
    };

    let resolve_stdin = |input: &Path| -> PathBuf {
        if input == Path::new("-") {
            let stdin_path = temp_dir.path().join("stdin.c");
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

    if args.emit_preprocess_only {
        let input = args
            .inputs
            .first()
            .cloned()
            .expect("missing C input file for preprocess-only");
        let resolved = resolve_stdin(&input);
        let preprocessed =
            co2_preprocessor::preprocess(&resolved, &preprocess_args(args, cpp_args));
        match &args.output {
            Some(path) => fs::write(path, preprocessed.raw_src.as_ref())
                .expect("failed to write preprocessed output"),
            None => print!("{}", preprocessed.raw_src),
        }
        return;
    }

    if args.emit_asm_only {
        let input = args
            .inputs
            .first()
            .cloned()
            .expect("missing C input file for asm emission");
        let resolved = resolve_stdin(&input);

        start_timing(args.time_report);
        let preprocessed = Arc::new(co2_preprocessor::preprocess(
            &resolved,
            &preprocess_args(args, cpp_args),
        ));
        write_dep_file(
            &preprocessed,
            args.output.as_deref(),
            &args.dep_args,
            cpp_args,
        );
        finish_preprocessing(args.time_report);

        let rustc_args = build_rustc_asm_args(
            &resolved,
            output.as_deref(),
            args.opt_level.as_deref(),
            args.debuginfo,
            force_pic,
            args.asm_flavor.as_deref(),
            args.emit_llvm,
            args,
        );
        compile_co2_source(CompileMode::C, resolved, preprocessed, rustc_args);
        finish_timing(args.time_report);
        return;
    }

    if args.emit_obj_only {
        let input = args
            .inputs
            .first()
            .cloned()
            .expect("missing C input file for object emission");
        let resolved = resolve_stdin(&input);

        if matches!(
            resolved.extension().and_then(|e| e.to_str()),
            Some("s" | "S")
        ) {
            compile_asm_to_object(
                &resolved,
                output.as_deref().unwrap_or(&temp_dir.path().join("out.o")),
                force_pic,
                args.time_report,
                &args.arch_flags,
            );
            return;
        }

        start_timing(args.time_report);
        let preprocessed = Arc::new(co2_preprocessor::preprocess(
            &resolved,
            &preprocess_args(args, cpp_args),
        ));
        write_dep_file(
            &preprocessed,
            args.output.as_deref(),
            &args.dep_args,
            cpp_args,
        );
        finish_preprocessing(args.time_report);

        let rustc_args = build_rustc_object_args(
            &resolved,
            output.as_deref(),
            args.opt_level.as_deref(),
            args.debuginfo,
            force_pic,
            args.emit_llvm,
            args,
        );
        compile_co2_source(CompileMode::C, resolved, preprocessed, rustc_args);
        finish_timing(args.time_report);
        return;
    }

    let mut object_paths = Vec::with_capacity(args.inputs.len());
    for input in &args.inputs {
        let resolved = resolve_stdin(input);
        let ext = resolved.extension().and_then(|e| e.to_str());
        if ext == Some("o") {
            object_paths.push(resolved);
            continue;
        }
        let object_path = temp_dir.path().join(object_stem(&resolved) + ".o");
        if matches!(ext, Some("s" | "S")) {
            compile_asm_to_object(
                &resolved,
                &object_path,
                force_pic,
                args.time_report,
                &args.arch_flags,
            );
        } else {
            compile_c_to_object(
                &resolved,
                &object_path,
                cpp_args,
                args.opt_level.as_deref(),
                args.debuginfo,
                force_pic,
                args.time_report,
                &args.dep_args,
                &args.arch_flags,
                &args.sanitizers,
                args.lto,
            );
        }
        object_paths.push(object_path);
    }

    crate::link::link_objects(
        &object_paths,
        &args.linker_args,
        output.as_deref(),
        args.opt_level.as_deref(),
        args.debuginfo,
        args.link_kind,
        &args.sanitizers,
        args.lto,
    );
}

fn object_stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("co2c_object")
        .replace('-', "_")
}

fn compile_asm_to_object(
    input: &Path,
    output: &Path,
    pic: bool,
    time_report: bool,
    arch_flags: &[String],
) {
    let assembler = std::env::var("CO2_AS").unwrap_or_else(|_| "gcc".to_owned());
    let mut cmd = Command::new(&assembler);
    cmd.arg("-c").arg(input).arg("-o").arg(output);
    for flag in arch_flags {
        cmd.arg(flag);
    }
    if pic {
        cmd.arg("-fPIC");
    }
    if time_report {
        cmd.arg("-ftime-report");
    }

    let status = cmd
        .status()
        .unwrap_or_else(|e| panic!("failed to execute assembler {assembler}: {e}"));
    if !status.success() {
        panic!(
            "co2cc assembly of {} failed with status {status}",
            input.display()
        );
    }
}

fn compile_c_to_object(
    input: &Path,
    output: &Path,
    cpp_args: &[String],
    opt_level: Option<&str>,
    debuginfo: Option<u8>,
    pic: bool,
    time_report: bool,
    dep_args: &crate::args::DepFileArgs,
    arch_flags: &[String],
    sanitizers: &[String],
    lto: bool,
) {
    let exe = current_invocation_path()
        .or_else(|| std::env::current_exe().ok())
        .expect("failed to locate co2cc executable");
    let mut cmd = Command::new(exe);
    cmd.arg("-c").arg(input).arg("-o").arg(output);
    for flag in arch_flags {
        cmd.arg(flag);
    }
    if !sanitizers.is_empty() {
        cmd.arg(format!("-fsanitize={}", sanitizers.join(",")));
    }
    if lto {
        cmd.arg("-flto");
    }
    if let Some(level) = opt_level {
        cmd.arg(format!("-O{level}"));
    }
    if debuginfo.is_some() {
        cmd.arg("-g");
    }
    if pic {
        cmd.arg("-fPIC");
    }
    if time_report {
        cmd.arg("-ftime-report");
    }
    if dep_args.generate {
        cmd.arg(if dep_args.exclude_system {
            "-MMD"
        } else {
            "-MD"
        });
    }
    if dep_args.phony {
        cmd.arg("-MP");
    }
    if let Some(ref dep_out) = dep_args.output {
        cmd.arg("-MF");
        cmd.arg(dep_out);
    }
    if let Some(ref target) = dep_args.target {
        cmd.arg(if dep_args.target_quoted { "-MQ" } else { "-MT" });
        cmd.arg(target);
    }
    for arg in cpp_args {
        cmd.arg(arg);
    }

    let status = cmd
        .status()
        .expect("failed to execute co2cc object compile");
    if !status.success() {
        if status.code() == Some(5) {
            co2_ast::panic_with_diagnostic_abort();
        }
        panic!("co2cc object compile failed with status {status}");
    }
}

fn start_timing(enabled: bool) {
    if enabled {
        co2_driver_lib::time_report::enable_timing();
    }
}

fn finish_preprocessing(enabled: bool) {
    if enabled {
        co2_driver_lib::time_report::mark_preprocess_done();
    }
}

fn finish_timing(enabled: bool) {
    if !enabled {
        return;
    }
    co2_driver_lib::time_report::finalize_timing();
    let Some(t) = co2_driver_lib::time_report::take_timing() else {
        return;
    };
    eprintln!("Time report:");
    eprintln!("  Preprocess:    {:.3}s", t.preprocess.as_secs_f64());
    eprintln!(
        "  Parse:         {:.3}s + {:.3}s",
        t.parse.as_secs_f64(),
        t.parse_slack.as_secs_f64()
    );
    eprintln!("  Lowering:      {:.3}s", t.lowering.as_secs_f64());
    eprintln!("    Body Parse:  {:.3}s", t.body_parse.as_secs_f64());
    eprintln!("    Sig Span:    {:.3}s", t.sig_call.as_secs_f64());
    eprintln!("    Borrowck:    {:.3}s", t.borrowck.as_secs_f64());
    eprintln!("    HIR:         {:.3}s", t.hir_lowering.as_secs_f64());
    eprintln!("    MIR:         {:.3}s", t.mir_lowering.as_secs_f64());
    eprintln!("  Codegen:       {:.3}s", t.codegen.as_secs_f64());
}
