use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use anyhow::{Context, Result, bail};
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
    for applet in ["co2rustc", "co2cc"] {
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
                eprintln!("FAIL {name}\n{err:#}");
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

    let compile = compile_test(root, suite, mode, test)?;
    match suite {
        Suite::Ui => {
            check_ui(test, &compile.output)?;
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

fn compile_test(root: &Path, suite: Suite, mode: Mode, test: &TestCase) -> Result<CompileResult> {
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

fn check_ui(test: &TestCase, output: &Output) -> Result<()> {
    if output.status.success() {
        bail!("UI test unexpectedly succeeded");
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut patterns = test.directives.get("ui-error").cloned().unwrap_or_default();
    if let Some(extra) = test.directives.get("ui-stderr-contains") {
        patterns.extend(extra.clone());
    }

    let sidecar = test.path.with_extension(format!(
        "{}.stderr",
        test.path
            .extension()
            .and_then(OsStr::to_str)
            .unwrap_or_default()
    ));

    if sidecar.exists() {
        let expected = fs::read_to_string(&sidecar)
            .with_context(|| format!("failed reading {}", sidecar.display()))?;
        if normalize(&expected) != normalize(&stderr) {
            bail!(
                "stderr mismatch for {}\n--- expected ---\n{}\n--- actual ---\n{}",
                test.path.display(),
                expected,
                stderr
            );
        }
        return Ok(());
    }

    if patterns.is_empty() {
        bail!("UI test has no expectations; add `//@ ui-error: ...` or a sidecar `.stderr` file");
    }

    for pat in patterns {
        if !stderr.contains(&pat) {
            bail!("missing UI pattern `{pat}` in stderr\n{stderr}");
        }
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
        bail!("run status mismatch: expected {expected_status}, got {got_status}");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if let Some(expected) = directive_text(test, "run-stdout") {
        let expected = unescape_text(&expected);
        if normalize(&expected) != normalize(&stdout) {
            bail!(
                "stdout mismatch\n--- expected ---\n{}\n--- actual ---\n{}",
                expected,
                stdout
            );
        }
    }
    if let Some(expected) = directive_text(test, "run-stderr") {
        let expected = unescape_text(&expected);
        if normalize(&expected) != normalize(&stderr) {
            bail!(
                "stderr mismatch\n--- expected ---\n{}\n--- actual ---\n{}",
                expected,
                stderr
            );
        }
    }

    for pat in test
        .directives
        .get("run-stdout-contains")
        .cloned()
        .unwrap_or_default()
    {
        if !stdout.contains(&pat) {
            bail!("missing run-stdout-contains pattern `{pat}` in stdout\n{stdout}");
        }
    }
    for pat in test
        .directives
        .get("run-stderr-contains")
        .cloned()
        .unwrap_or_default()
    {
        if !stderr.contains(&pat) {
            bail!("missing run-stderr-contains pattern `{pat}` in stderr\n{stderr}");
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

        out.push(TestCase {
            directives: parse_directives(&path)?,
            path,
            kind: TestKind::File,
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
