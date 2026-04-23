use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use anyhow::{Context, Result, bail};
use ariadne::{Color, Label, Report, ReportKind, Source};
use clap::{Parser, ValueEnum};
use tempfile::{Builder as TempDirBuilder, TempDir};

#[derive(Parser, Debug)]
#[command(name = "co2_test_harness")]
struct Cli {
    #[arg(value_enum, default_value_t = SuiteArg::All)]
    suite: SuiteArg,
    #[arg(long)]
    filter: Option<String>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum SuiteArg {
    All,
    Ui,
    Run,
    Debuginfo,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Suite {
    Ui,
    Run,
    Debuginfo,
}

impl Suite {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ui => "ui",
            Self::Run => "run",
            Self::Debuginfo => "debuginfo",
        }
    }
}

fn suites_from_arg(arg: SuiteArg) -> Vec<Suite> {
    match arg {
        SuiteArg::All => vec![Suite::Ui, Suite::Run, Suite::Debuginfo],
        SuiteArg::Ui => vec![Suite::Ui],
        SuiteArg::Run => vec![Suite::Run],
        SuiteArg::Debuginfo => vec![Suite::Debuginfo],
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    C,
    Co2,
    Rust,
}

impl Mode {
    fn from_directive(s: &str) -> Result<Self> {
        match s {
            "c" => Ok(Self::C),
            "co2" => Ok(Self::Co2),
            "rust" => Ok(Self::Rust),
            _ => bail!("unknown mode: {s}"),
        }
    }
}

#[derive(Default)]
struct Stats {
    passed: usize,
    failed: usize,
    skipped: usize,
}

struct TestCase {
    path: PathBuf,
    kind: TestKind,
    directives: HashMap<String, Vec<String>>,
    #[allow(dead_code)]
    source: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TestKind {
    File,
    NuDir,
}

struct CompileResult {
    output: Output,
    exe_path: PathBuf,
    _temp: TempDir,
}

enum TestOutcome {
    Pass,
    Skip(String),
}

#[derive(Debug, Clone)]
struct UiSpanExpectation {
    byte_start: usize,
    byte_end: usize,
    message: Option<String>,
}

#[derive(Debug, Clone)]
struct UiDiagnostic {
    message: String,
    spans: Vec<UiDiagnosticSpan>,
}

#[derive(Debug, Clone)]
struct UiDiagnosticSpan {
    byte_start: usize,
    byte_end: usize,
    is_primary: bool,
}

#[derive(Debug, Clone)]
struct TestError {
    source: String,
    span: Option<(usize, usize)>,
    message: String,
}

impl std::fmt::Display for TestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for TestError {}

#[derive(Debug, Clone)]
struct UiTestError {
    path: PathBuf,
    source: String,
    issues: Vec<UiTestIssue>,
}

#[derive(Debug, Clone)]
struct UiTestIssue {
    span: Option<UiSpanExpectation>,
    reason: String,
}

impl std::fmt::Display for UiTestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} issue(s)", self.issues.len())
    }
}

impl std::error::Error for UiTestError {}

fn render_ui_error(err: &UiTestError) {
    let path = &err.path;
    let name = path.display().to_string();
    let source = &err.source;

for issue in &err.issues {
        if let Some(span) = &issue.span {
            if span.byte_start < source.len() && span.byte_end <= source.len() {
                let mut r = Report::build(
                    ReportKind::Error,
                    (&*name, span.byte_start..span.byte_end),
                );
                r.add_label(
                    Label::new((&*name, span.byte_start..span.byte_end))
                        .with_color(Color::Red)
                        .with_message(&issue.reason),
                );
                r.finish()
                    .eprint((&*name, Source::from(source)))
.unwrap_or_else(|_| eprintln!("Error: {}\n  at {}", issue.reason, name));
            } else {
                eprintln!("Error: {}\n  at {}", issue.reason, name);
            }
        } else {
            eprintln!("Error: {}\n  at {}", issue.reason, name);
        }
    }
}

fn render_test_error(path: &Path, err: &TestError) {
    let source = &err.source;
    if let Some((start, end)) = err.span {
        let mut r = Report::build(ReportKind::Error, (path.display().to_string(), start..end));
        r.add_label(
            Label::new((path.display().to_string(), start..end))
                .with_color(Color::Red)
                .with_message(&err.message),
        );
        r.finish()
            .eprint((path.display().to_string(), Source::from(source)))
            .unwrap_or_else(|e| eprintln!("{e:#}"));
    } else {
        let mut r = Report::build(ReportKind::Error, (path.display().to_string(), 0..0));
        r.set_note(&err.message);
        r.finish()
            .eprint((path.display().to_string(), Source::from(source)))
            .unwrap_or_else(|e| eprintln!("{e:#}"));
    }
}

fn main() {
    if let Err(e) = run_main() {
        eprintln!("co2_test_harness: {e:#}");
        std::process::exit(1);
    }
}

fn run_main() -> Result<()> {
    let cli = Cli::parse();
    let root = workspace_root()?;
    build_compilers(&root)?;

    let mut stats = Stats::default();
    for suite in suites_from_arg(cli.suite) {
        run_suite(&root, suite, cli.filter.as_deref(), &mut stats)?;
    }

    eprintln!(
        "summary: passed={}, failed={}, skipped={}",
        stats.passed, stats.failed, stats.skipped
    );

    if stats.failed > 0 {
        std::process::exit(1);
    }
    Ok(())
}

fn workspace_root() -> Result<PathBuf> {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .context("failed to resolve workspace root")
}

fn build_compilers(root: &Path) -> Result<()> {
    let status = Command::new("cargo")
        .current_dir(root)
        .args(["build", "-q", "--locked", "-p", "co2-multicall"])
        .status()
        .context("failed to run cargo build for co2-multicall")?;
    if !status.success() {
        bail!("building co2-multicall failed with status {status}");
    }
    ensure_compiler_links(&root.join("target").join("debug"))?;
    Ok(())
}

fn ensure_compiler_links(bin_dir: &Path) -> Result<()> {
    let multicall = bin_dir.join(exe_name("co2-multicall"));
    for applet in ["co2rustc", "co2cc", "co2cargo"] {
        ensure_link(&multicall, &bin_dir.join(exe_name(applet)))?;
    }
    Ok(())
}

fn exe_name(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

fn ensure_link(source: &Path, target: &Path) -> Result<()> {
    if let Ok(existing) = fs::read_link(target) {
        if existing == source || existing == Path::new(source.file_name().unwrap_or_default()) {
            return Ok(());
        }
    }

    if target.exists() || fs::symlink_metadata(target).is_ok() {
        let metadata = fs::symlink_metadata(target)
            .with_context(|| format!("failed to stat existing {}", target.display()))?;
        if metadata.file_type().is_dir() {
            bail!("refusing to replace directory {}", target.display());
        }
        fs::remove_file(target)
            .with_context(|| format!("failed to remove existing {}", target.display()))?;
    }

    create_symlink_or_copy(source, target)
}

#[cfg(unix)]
fn create_symlink_or_copy(source: &Path, target: &Path) -> Result<()> {
    std::os::unix::fs::symlink(source, target).or_else(|_| {
        fs::copy(source, target)
            .with_context(|| format!("failed to copy {} to {}", source.display(), target.display()))
            .map(|_| ())
    })
}

#[cfg(windows)]
fn create_symlink_or_copy(source: &Path, target: &Path) -> Result<()> {
    std::os::windows::fs::symlink_file(source, target).or_else(|_| {
        fs::copy(source, target)
            .with_context(|| format!("failed to copy {} to {}", source.display(), target.display()))
            .map(|_| ())
    })
}

fn run_suite(root: &Path, suite: Suite, filter: Option<&str>, stats: &mut Stats) -> Result<()> {
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

    let ui_spans = if suite == Suite::Ui {
        parse_ui_span_expectations(&test.path, mode)?
    } else {
        Vec::new()
    };
        let compile = compile_test(root, suite, mode, test, !ui_spans.is_empty())?;
    match suite {
        Suite::Ui => {
            check_ui(test, mode, &compile.output, &ui_spans)?;
            Ok(TestOutcome::Pass)
        }
        Suite::Run => {
            check_run(test, &compile)?;
            Ok(TestOutcome::Pass)
        }
        Suite::Debuginfo => check_debuginfo(test, &compile),
    }
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
            "nushell test status mismatch: expected {run_status}, got {got_status}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(())
}

fn compile_test(
    root: &Path,
    suite: Suite,
    mode: Mode,
    test: &TestCase,
    json_diagnostics: bool,
) -> Result<CompileResult> {
    let temp = TempDirBuilder::new()
        .prefix("co2-ct-")
        .tempdir()
        .context("failed to create temp dir for test artifacts")?;
    let temp_path = temp.path();

    let name = test
        .path
        .file_stem()
        .and_then(OsStr::to_str)
        .context("invalid test filename")?;
    let exe_path = temp_path.join(format!("{name}.bin"));

    let mut compile_flags = directive_args(test, "compile-flags")?;
    if suite == Suite::Debuginfo
        && !compile_flags.windows(2).any(|w| w == ["-C", "debuginfo=2"])
        && !compile_flags.iter().any(|s| s.contains("debuginfo="))
    {
        compile_flags.extend(["-C".to_owned(), "debuginfo=2".to_owned()]);
    }

    let output = match mode {
        Mode::C => {
            let c_src = temp_path.join(test.path.file_name().context("missing C test filename")?);
            fs::copy(&test.path, &c_src).context("failed to copy C test source")?;

            let mut cmd = Command::new(root.join("target").join("debug").join("co2cc"));
            cmd.arg(&c_src).arg("-o").arg(&exe_path).args(compile_flags);
            if json_diagnostics {
                cmd.env("CO2_FORCE_JSON_DIAGNOSTICS", "1");
            }
            cmd.output().context("failed to execute co2cc")?
        }
        Mode::Co2 => {
            let co2_src = temp_path.join(format!("{name}.co2"));
            fs::copy(&test.path, &co2_src).context("failed to copy co2 test source")?;

            let shim = temp_path.join(format!("{name}.rs"));
            fs::write(
                &shim,
                "#![feature(register_tool)]\n#![feature(custom_inner_attributes)]\n#![register_tool(co2)]\n#![co2::language]\n",
            )
            .context("failed to write co2 shim rust file")?;

            let mut cmd = Command::new(root.join("target").join("debug").join("co2rustc"));
            cmd.arg(&shim)
                .arg("--crate-type=bin")
                .arg("--edition=2024")
                .arg("-o")
                .arg(&exe_path)
                .args(compile_flags);
            if json_diagnostics {
                cmd.env("CO2_FORCE_JSON_DIAGNOSTICS", "1");
            }
            cmd.output().context("failed to execute co2rustc")?
        }
        Mode::Rust => {
            let rust_src = temp_path.join(
                test.path
                    .file_name()
                    .context("missing Rust test filename")?,
            );
            fs::copy(&test.path, &rust_src).context("failed to copy Rust test source")?;

            let mut cmd = Command::new(root.join("target").join("debug").join("co2rustc"));
            cmd.arg("--edition=2024")
                .arg(&rust_src)
                .arg("-o")
                .arg(&exe_path)
                .args(compile_flags);
            if json_diagnostics {
                cmd.env("CO2_FORCE_JSON_DIAGNOSTICS", "1");
            }
            cmd.output()
                .context("failed to execute co2rustc for Rust test")?
        }
    };

    Ok(CompileResult {
        output,
        exe_path,
        _temp: temp,
    })
}

fn check_ui(
    test: &TestCase,
    mode: Mode,
    output: &Output,
    span_expectations: &[UiSpanExpectation],
) -> Result<()> {
    if output.status.success() {
        bail!("UI test unexpectedly succeeded");
    }
    let expected_status = match mode {
        Mode::Rust => 1,
        Mode::C | Mode::Co2 => 5,
    };
    let got_status = output.status.code().unwrap_or(-1);
    if got_status != expected_status {
        bail!(
            "compile-fail status mismatch: expected {expected_status}, got {got_status}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    if test.directives.contains_key("ui-error") || test.directives.contains_key("ui-stderr-contains")
    {
        bail!(
            "legacy UI directives are no longer supported in {}; use `//@ compile-fail` with inline `//^^^^ error: ...` annotations",
            test.path.display()
        );
    }
    if !test.directives.contains_key("compile-fail") {
        bail!(
            "UI test is missing `//@ compile-fail`: {}",
            test.path.display()
        );
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    if span_expectations.is_empty() {
        bail!(
            "UI test has no inline span expectations; add a `//^^^^ error: ...` annotation near the failing source: {}",
            test.path.display()
        );
    }

    if !span_expectations.iter().any(|expected| expected.message.is_some()) {
        bail!(
            "UI test must include diagnostic text in at least one inline span annotation: {}",
            test.path.display()
        );
    }

    let diagnostics = parse_ui_diagnostics(&stderr)?;
    let mut issues = Vec::new();

    for expected in span_expectations {
        let message_matches = diagnostics.iter().any(|diagnostic| {
            if let Some(message) = &expected.message
                && !diagnostic.message.contains(message)
            {
                return false;
            }

            diagnostic.spans.iter().any(|span| {
                span.is_primary
                    && span.byte_start == expected.byte_start
                    && span.byte_end == expected.byte_end
            })
        });

        if !message_matches {
            let reason = if let Some(msg) = &expected.message {
                let found = diagnostics.iter().any(|d| d.message.contains(msg));
                if found {
                    format!("Missing diagnostic span for: {}", msg)
                } else {
                    format!("Missing diagnostic: {}", msg)
                }
            } else {
                "missing UI span annotation".to_string()
            };
            issues.push(UiTestIssue {
                span: Some(expected.clone()),
                reason,
            });
        }
    }

    for diagnostic in &diagnostics {
        let matched = span_expectations.iter().any(|expected| {
            if let Some(message) = &expected.message
                && !diagnostic.message.contains(message)
            {
                return false;
            }

            diagnostic.spans.iter().any(|span| {
                span.is_primary
                    && span.byte_start == expected.byte_start
                    && span.byte_end == expected.byte_end
            })
        });

        if !matched {
            if let Some(primary_span) = diagnostic.spans.iter().find(|s| s.is_primary) {
                issues.push(UiTestIssue {
                    span: Some(UiSpanExpectation {
                        message: Some(diagnostic.message.clone()),
                        byte_start: primary_span.byte_start,
                        byte_end: primary_span.byte_end,
                    }),
                    reason: format!("Unexpected diagnostic: {}", diagnostic.message),
                });
            } else {
                issues.push(UiTestIssue {
                    span: None,
                    reason: format!("Unexpected diagnostic: {}", diagnostic.message),
                });
            }
        }
    }

    if !issues.is_empty() {
        return Err(UiTestError {
            path: test.path.clone(),
            source: test.source.clone(),
            issues,
        }
        .into());
    }

    Ok(())
}

fn check_run(test: &TestCase, compile: &CompileResult) -> Result<()> {
    if !compile.output.status.success() {
        bail!(
            "run test compilation failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&compile.output.stdout),
            String::from_utf8_lossy(&compile.output.stderr)
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
                message: format!(
                    "stdout mismatch:\n  expected: {}\n  actual:   {}",
                    expected.lines().next().unwrap_or(""),
                    stdout.lines().next().unwrap_or("")
                ),
                
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
            "debuginfo test compilation failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&compile.output.stdout),
            String::from_utf8_lossy(&compile.output.stderr)
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

fn collect_tests(dir: &Path, filter: Option<&str>) -> Result<Vec<TestCase>> {
    let mut out = Vec::new();
    if !dir.exists() {
        return Ok(out);
    }
    collect_tests_inner(dir, filter, &mut out)?;
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

fn collect_tests_inner(dir: &Path, filter: Option<&str>, out: &mut Vec<TestCase>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("failed to read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            let main_nu = path.join("main.nu");
            if main_nu.exists() {
                if let Some(f) = filter
                    && !path.to_string_lossy().contains(f)
                {
                    continue;
                }
                out.push(TestCase {
                    directives: parse_directives(&main_nu)?,
                    path,
                    kind: TestKind::NuDir,
                    source: String::new(),
                });
                continue;
            }
            collect_tests_inner(&path, filter, out)?;
            continue;
        }

        let Some(ext) = path.extension().and_then(OsStr::to_str) else {
            continue;
        };
        if ext != "c" && ext != "co2" && ext != "rs" {
            continue;
        }
        if let Some(f) = filter
            && !path.to_string_lossy().contains(f)
        {
            continue;
        }

        let source = fs::read_to_string(&path)
            .with_context(|| format!("failed to read test source {}", path.display()))?;
        out.push(TestCase {
            directives: parse_directives(&path)?,
            path,
            kind: TestKind::File,
            source,
        });
    }
    Ok(())
}

fn parse_directives(path: &Path) -> Result<HashMap<String, Vec<String>>> {
    let src = fs::read_to_string(path)
        .with_context(|| format!("failed to read test source {}", path.display()))?;
    let mut map: HashMap<String, Vec<String>> = HashMap::new();

    for line in src.lines() {
        let line = line.trim_start();
        let body = if let Some(body) = line.strip_prefix("//@") {
            body.trim()
        } else if let Some(body) = line.strip_prefix("#@") {
            body.trim()
        } else {
            continue;
        };
        let (key, value) = if let Some((k, v)) = body.split_once(':') {
            (k.trim().to_owned(), v.trim().to_owned())
        } else {
            (body.to_owned(), String::new())
        };
        map.entry(key).or_default().push(value);
    }

    Ok(map)
}

fn parse_ui_span_expectations(path: &Path, _mode: Mode) -> Result<Vec<UiSpanExpectation>> {
    let src = fs::read_to_string(path)
        .with_context(|| format!("failed to read test source {}", path.display()))?;
    let line_starts = line_start_offsets(&src);
    let mut out = Vec::new();

    for (idx, line) in src.lines().enumerate() {
        let Some((column_start, column_end, message)) = parse_ui_span_annotation(line) else {
            continue;
        };
        let line_no = idx + 1;
        if line_no == 1 {
            bail!(
                "span annotation on the first line has no source line to point at: {}",
                path.display()
            );
        }
        let source_line_idx = line_no - 2;
        let line_start = line_starts[source_line_idx];
        out.push(UiSpanExpectation {
            byte_start: line_start + (column_start - 1),
            byte_end: line_start + (column_end - 1),
            message,
        });
    }

    Ok(out)
}

fn parse_ui_span_annotation(line: &str) -> Option<(usize, usize, Option<String>)> {
    let comment_start = line.find("//")?;
    if !line[..comment_start].chars().all(char::is_whitespace) {
        return None;
    }

    let body = &line[comment_start + 2..];
    let caret_offset = body.find('^')?;
    if !body[..caret_offset].chars().all(char::is_whitespace) {
        return None;
    }

    let caret_count = body[caret_offset..]
        .chars()
        .take_while(|ch| *ch == '^')
        .count();
    if caret_count == 0 {
        return None;
    }

    let column_start = line[..comment_start + 2 + caret_offset].chars().count() + 1;
    let column_end = column_start + caret_count;
    let trailing = body[caret_offset + caret_count..].trim();
    let message = (!trailing.is_empty()).then(|| {
        trailing
            .strip_prefix("error:")
            .or_else(|| trailing.strip_prefix("warning:"))
            .or_else(|| trailing.strip_prefix("help:"))
            .map(str::trim)
            .unwrap_or(trailing)
            .to_owned()
    });
    Some((column_start, column_end, message))
}

fn parse_ui_diagnostics(stderr: &str) -> Result<Vec<UiDiagnostic>> {
    parse_json_diagnostics(stderr)
}

fn parse_json_diagnostics(stderr: &str) -> Result<Vec<UiDiagnostic>> {
    let mut diagnostics = Vec::new();

    for line in stderr.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || !trimmed.starts_with('{') {
            continue;
        }

        let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        let Some(spans) = value.get("spans").and_then(serde_json::Value::as_array) else {
            continue;
        };
        let Some(message) = value.get("message").and_then(serde_json::Value::as_str) else {
            continue;
        };

        let spans = spans
            .iter()
            .filter_map(|span| {
                Some(UiDiagnosticSpan {
                    byte_start: span.get("byte_start")?.as_u64()? as usize,
                    byte_end: span.get("byte_end")?.as_u64()? as usize,
                    is_primary: span.get("is_primary")?.as_bool()?,
                })
            })
            .collect::<Vec<_>>();

        diagnostics.push(UiDiagnostic {
            message: message.to_owned(),
            spans,
        });
    }

    if diagnostics.is_empty() {
        bail!("failed to find JSON diagnostics in stderr");
    }

    Ok(diagnostics)
}

fn line_start_offsets(src: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (idx, ch) in src.char_indices() {
        if ch == '\n' {
            starts.push(idx + 1);
        }
    }
    starts
}

fn directive_text(test: &TestCase, key: &str) -> Option<String> {
    test.directives.get(key).and_then(|v| v.first().cloned())
}

fn directive_i32(test: &TestCase, key: &str) -> Result<Option<i32>> {
    let Some(raw) = directive_text(test, key) else {
        return Ok(None);
    };
    let parsed = raw
        .parse::<i32>()
        .with_context(|| format!("directive `{key}` must be an integer, got `{raw}`"))?;
    Ok(Some(parsed))
}

fn directive_args(test: &TestCase, key: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for raw in test.directives.get(key).cloned().unwrap_or_default() {
        let split = shlex::split(&raw)
            .with_context(|| format!("failed to parse `{key}` args from `{raw}`"))?;
        out.extend(split);
    }
    Ok(out)
}

fn unescape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

fn normalize(s: &str) -> String {
    s.replace("\r\n", "\n")
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
}

fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst).with_context(|| format!("failed to create {}", dst.display()))?;
    for entry in fs::read_dir(src).with_context(|| format!("failed to read {}", src.display()))? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &to)?;
        } else {
            fs::copy(entry.path(), &to).with_context(|| {
                format!(
                    "failed to copy {} -> {}",
                    entry.path().display(),
                    to.display()
                )
            })?;
        }
    }
    Ok(())
}
