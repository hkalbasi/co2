use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use tempfile::Builder as TempDirBuilder;

use crate::cli::SuiteArg;
use crate::compiler::{CompileResult, compile_test};
use crate::error::{TestError, UiTestError, render_test_error, render_ui_error};
use crate::test_case::{
    Mode, TestCase, TestKind, TestOutcome, collect_tests, directive_args, directive_i32,
    directive_text,
};
use crate::ui::{
    UiSpanExpectation, check_compile_warnings, check_ui, format_named_output,
    parse_ui_span_expectations,
};
use crate::util::{copy_dir_all, normalize, unescape_text};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Suite {
    Ui,
    Run,
    Debuginfo,
}

impl Suite {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ui => "ui",
            Self::Run => "run",
            Self::Debuginfo => "debuginfo",
        }
    }
}

pub fn suites_from_arg(arg: SuiteArg) -> Vec<Suite> {
    match arg {
        SuiteArg::All => vec![Suite::Ui, Suite::Run, Suite::Debuginfo],
        SuiteArg::Ui => vec![Suite::Ui],
        SuiteArg::Run => vec![Suite::Run],
        SuiteArg::Debuginfo => vec![Suite::Debuginfo],
    }
}

#[derive(Default)]
pub struct Stats {
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
}

pub fn run_suite(root: &Path, suite: Suite, filter: Option<&str>, stats: &mut Stats) -> Result<()> {
    let dir = root.join("tests").join("compiletest").join(suite.as_str());
    let tests = collect_tests(&dir, filter)?;
    eprintln!(
        "running {} tests for suite `{}`",
        tests.len(),
        suite.as_str()
    );

    for test in tests {
        let name = test.path.strip_prefix(root).unwrap_or(&test.path).display();
        match run_test(root, suite, &test) {
            Ok(TestOutcome::Pass) => {
                stats.passed += 1;
                eprintln!("ok   {name}");
            }
            Ok(TestOutcome::Skip(reason)) => {
                stats.skipped += 1;
                eprintln!("skip {name} ({reason})");
            }
            Err(err) => {
                stats.failed += 1;
                let name = test.path.strip_prefix(root).unwrap_or(&test.path).display();
                if let Some(e) = err.downcast_ref::<TestError>() {
                    eprintln!("FAIL {name}");
                    render_test_error(&test.path, e);
                } else if let Some(e) = err.downcast_ref::<UiTestError>() {
                    eprintln!("FAIL {name}");
                    render_ui_error(e);
                } else {
                    eprintln!("FAIL {name}\n{err:#}");
                }
            }
        }
    }
    Ok(())
}

fn run_test(root: &Path, suite: Suite, test: &TestCase) -> Result<TestOutcome> {
    if let Some(reason) = directive_text(test, "skip") {
        return Ok(TestOutcome::Skip(reason));
    }

    if test.kind == TestKind::NuDir {
        if suite != Suite::Run {
            bail!(
                "directory Nushell tests are only supported in the `run` suite: {}",
                test.path.display()
            );
        }
        run_nu_dir_test(root, test)?;
        return Ok(TestOutcome::Pass);
    }

    let mode = Mode::from_directive(
        test.directives
            .get("mode")
            .and_then(|v| v.first())
            .context("missing `//@ mode: c|co2|rust` directive")?,
    )?;

    let (compile_annotations, sources) = collect_compile_annotations(test, mode)?;
    let directive_warning_expectations = test
        .directives
        .get("compile-warning")
        .cloned()
        .unwrap_or_default();
    let compile = compile_test(root, suite, mode, test, true)?;
    match suite {
        Suite::Ui => {
            check_ui(test, mode, &compile.output, &compile_annotations, sources)?;
            Ok(TestOutcome::Pass)
        }
        Suite::Run => {
            check_compile_warnings(
                test,
                &compile.output,
                &compile_annotations,
                &directive_warning_expectations,
                sources,
            )?;
            check_run(test, &compile)?;
            Ok(TestOutcome::Pass)
        }
        Suite::Debuginfo => {
            check_compile_warnings(
                test,
                &compile.output,
                &compile_annotations,
                &directive_warning_expectations,
                sources,
            )?;
            check_debuginfo(test, &compile)
        }
    }
}

fn collect_compile_annotations(
    test: &TestCase,
    mode: Mode,
) -> Result<(Vec<UiSpanExpectation>, HashMap<String, String>)> {
    let mut annotations = Vec::new();
    let mut sources = HashMap::new();
    sources.insert(
        test.path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        test.source.clone(),
    );
    annotations.extend(parse_ui_span_expectations(&test.path, mode)?);

    let test_dir = test.path.parent().context("test path has no parent")?;
    let test_stem = test.path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    for entry in std::fs::read_dir(test_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() && path != test.path {
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            if stem != test_stem {
                continue;
            }
            if let Some(ext) = path.extension().and_then(|s| s.to_str())
                && matches!(ext, "c" | "h" | "co2" | "rs")
            {
                sources.insert(
                    path.file_name().unwrap().to_string_lossy().into_owned(),
                    std::fs::read_to_string(&path)?,
                );
                annotations.extend(parse_ui_span_expectations(&path, mode)?);
            }
        }
    }

    Ok((annotations, sources))
}

fn run_nu_dir_test(root: &Path, test: &TestCase) -> Result<()> {
    let temp = TempDirBuilder::new()
        .prefix("co2-ct-dir-")
        .tempdir()
        .context("failed to create temp dir for Nushell test")?;
    let temp_path = temp.path().join(
        test.path
            .file_name()
            .context("directory test path has no final component")?,
    );
    copy_dir_all(&test.path, &temp_path)?;

    let main_nu = temp_path.join("main.nu");
    let run_status = directive_i32(test, "run-status")?.unwrap_or(0);

    let path_sep = if cfg!(windows) { ";" } else { ":" };
    let current_path = std::env::var("PATH").unwrap_or_default();
    let compiler_bin = root.join("target").join("debug");
    let merged_path = if current_path.is_empty() {
        compiler_bin.to_string_lossy().into_owned()
    } else {
        format!("{}{}{}", compiler_bin.display(), path_sep, current_path)
    };

    let output = Command::new("nu")
        .arg(&main_nu)
        .current_dir(&temp_path)
        .env("PATH", merged_path)
        .env("CO2_WORKSPACE_ROOT", root)
        .env("CO2_TEST_DIR", &temp_path)
        .env("CO2_BIN_DIR", &compiler_bin)
        .output()
        .with_context(|| format!("failed to execute Nushell test {}", main_nu.display()))?;

    let got_status = output.status.code().unwrap_or(-1);
    if got_status != run_status {
        bail!(
            "nushell test status mismatch: expected {run_status}, got {got_status}\n{}\n{}",
            format_named_output("stdout", &String::from_utf8_lossy(&output.stdout)),
            format_named_output("stderr", &String::from_utf8_lossy(&output.stderr)),
        );
    }

    Ok(())
}

fn check_run(test: &TestCase, compile: &CompileResult) -> Result<()> {
    if !compile.output.status.success() {
        bail!(
            "run test compilation failed\n{}\n{}",
            format_named_output("stdout", &String::from_utf8_lossy(&compile.output.stdout)),
            format_named_output("stderr", &String::from_utf8_lossy(&compile.output.stderr)),
        );
    }

    let run_args = directive_args(test, "run-args")?;
    let output = Command::new(&compile.exe_path)
        .args(run_args)
        .output()
        .with_context(|| format!("failed to execute {}", compile.exe_path.display()))?;

    let got_status = output.status.code().unwrap_or(-1);
    let expected_status = directive_i32(test, "run-status")?.unwrap_or(0);
    if got_status != expected_status {
        return Err(TestError {
            source: test.source.clone(),
            span: None,
            message: format!(
                "exit code mismatch: expected `{}`, got `{}`",
                expected_status, got_status
            ),
        }
        .into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if let Some(expected) = directive_text(test, "run-stdout") {
        let expected = unescape_text(&expected);
        if normalize(&expected) != normalize(&stdout) {
            return Err(TestError {
                source: test.source.clone(),
                span: None,
                message: format!("stdout mismatch:\n  expected: {expected}\n  actual:   {stdout}",),
            }
            .into());
        }
    }
    if let Some(expected) = directive_text(test, "run-stderr") {
        let expected = unescape_text(&expected);
        if normalize(&expected) != normalize(&stderr) {
            return Err(TestError {
                source: test.source.clone(),
                span: None,
                message: format!(
                    "stderr mismatch:\n  expected: {}\n  actual:   {}",
                    expected.lines().next().unwrap_or(""),
                    stderr.lines().next().unwrap_or("")
                ),
            }
            .into());
        }
    }

    for pat in test
        .directives
        .get("run-stdout-contains")
        .cloned()
        .unwrap_or_default()
    {
        if !stdout.contains(&pat) {
            return Err(TestError {
                source: test.source.clone(),
                span: None,
                message: format!(
                    "stdout missing pattern `{}` (got `{}`)",
                    pat,
                    stdout.lines().next().unwrap_or("")
                ),
            }
            .into());
        }
    }
    for pat in test
        .directives
        .get("run-stderr-contains")
        .cloned()
        .unwrap_or_default()
    {
        if !stderr.contains(&pat) {
            return Err(TestError {
                source: test.source.clone(),
                span: None,
                message: format!(
                    "stderr missing pattern `{}` (got `{}`)",
                    pat,
                    stderr.lines().next().unwrap_or("")
                ),
            }
            .into());
        }
    }

    Ok(())
}

fn check_debuginfo(test: &TestCase, compile: &CompileResult) -> Result<TestOutcome> {
    if !compile.output.status.success() {
        bail!(
            "debuginfo test compilation failed\n{}\n{}",
            format_named_output("stdout", &String::from_utf8_lossy(&compile.output.stdout)),
            format_named_output("stderr", &String::from_utf8_lossy(&compile.output.stderr)),
        );
    }

    let debugger = directive_text(test, "debugger").unwrap_or_else(|| "gdb".to_owned());
    if !tool_available(&debugger) {
        return Ok(TestOutcome::Skip(format!("{debugger} not available")));
    }

    let run_args = directive_args(test, "run-args")?;
    let debug_cmds = test
        .directives
        .get("debug-command")
        .cloned()
        .unwrap_or_else(|| vec!["break main".to_owned(), "run".to_owned()]);
    let checks = test
        .directives
        .get("debug-check")
        .cloned()
        .unwrap_or_default();

    let output = match debugger.as_str() {
        "gdb" => {
            let mut cmd = Command::new("gdb");
            cmd.arg("-q").arg("--batch");
            cmd.arg("-ex").arg("set pagination off");
            cmd.arg("-ex").arg("set confirm off");
            for script in debug_cmds {
                cmd.arg("-ex").arg(script);
            }
            cmd.arg("--args").arg(&compile.exe_path).args(&run_args);
            cmd.output().context("failed to execute gdb")?
        }
        "lldb" => {
            let mut cmd = Command::new("lldb");
            cmd.arg("-b");
            for script in debug_cmds {
                cmd.arg("-o").arg(script);
            }
            cmd.arg("--").arg(&compile.exe_path).args(&run_args);
            cmd.output().context("failed to execute lldb")?
        }
        _ => bail!("unsupported debugger `{debugger}`"),
    };

    let mut merged = String::new();
    merged.push_str(&String::from_utf8_lossy(&output.stdout));
    merged.push_str(&String::from_utf8_lossy(&output.stderr));

    let expected_status = directive_i32(test, "debug-status")?.unwrap_or(0);
    let got_status = output.status.code().unwrap_or(-1);
    if got_status != expected_status {
        if let Some(reason) = debuginfo_skip_reason(&merged) {
            return Ok(TestOutcome::Skip(format!(
                "debugger execution is restricted in this environment ({reason})"
            )));
        }
        bail!("debugger status mismatch: expected {expected_status}, got {got_status}\n{merged}");
    }

    for check in checks {
        if !merged.contains(&check) {
            if let Some(reason) = debuginfo_skip_reason(&merged) {
                return Ok(TestOutcome::Skip(format!(
                    "debugger output not usable in this environment ({reason})"
                )));
            }
            bail!("missing debug-check pattern `{check}`\n{merged}");
        }
    }

    Ok(TestOutcome::Pass)
}

fn debuginfo_skip_reason(output: &str) -> Option<&'static str> {
    let lowered = output.to_ascii_lowercase();
    let markers: [(&str, &str); 13] = [
        ("ptrace", "ptrace unavailable"),
        ("operation not permitted", "operation not permitted"),
        ("not permitted", "operation not permitted"),
        ("permission denied", "permission denied"),
        (
            "error disabling address space randomization",
            "cannot disable ASLR",
        ),
        ("could not attach", "attach failed"),
        ("can't attach", "attach failed"),
        ("cannot attach", "attach failed"),
        ("no such process", "target process unavailable"),
        ("process exited with code 127", "debugger launch failed"),
        ("no symbol", "missing debuginfo symbols"),
        (
            "no line number information",
            "missing line debug information",
        ),
        (
            "which has no line number information",
            "missing line debug information",
        ),
    ];

    for (needle, reason) in markers {
        if lowered.contains(needle) {
            return Some(reason);
        }
    }
    None
}

fn tool_available(name: &str) -> bool {
    Command::new(name)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
