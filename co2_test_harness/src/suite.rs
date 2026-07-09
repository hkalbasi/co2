use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use anyhow::{Context, Result, bail};
use tempfile::Builder as TempDirBuilder;

use crate::compiler::{
    CompileResult, MiriRun, binary_path, compile_test, find_bin_dir_from_path, run_miri_test,
};
use crate::debuginfo;
use crate::error::{TestError, UiTestError, render_test_error, render_ui_error};
use crate::test_case::{
    Mode, TestCase, TestKind, TestOutcome, collect_examples, collect_tests, directive_args,
    directive_i32, directive_text,
};
use crate::ui::{
    UiAnnotationLevel, UiSpanExpectation, check_compile_warnings, check_ui, format_named_output,
    parse_ui_span_expectations,
};
use crate::util::{copy_dir_all, line_start_offsets, normalize, unescape_text};

#[derive(Default)]
pub struct Stats {
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub failed_names: Vec<String>,
}

pub fn run_tests(
    root: &Path,
    bin_dir: Option<&Path>,
    filter: Option<&str>,
    coverage_dir: Option<&Path>,
    dump_mir: bool,
    update_snapshots: bool,
    verbose: bool,
    stats: &mut Stats,
) -> Result<()> {
    let mut tests = collect_tests(root, filter)?;
    let examples = collect_examples(root, filter)?;
    tests.extend(examples);
    eprintln!("running {} tests", tests.len());

    for test in tests {
        let name = test.path.strip_prefix(root).unwrap_or(&test.path).display();
        match run_test(
            root,
            bin_dir,
            &test,
            coverage_dir,
            dump_mir,
            update_snapshots,
            verbose,
        ) {
            Ok(TestOutcome::Pass) => {
                stats.passed += 1;
                if verbose {
                    eprintln!("ok {name}");
                } else {
                    eprint!(".");
                }
            }
            Ok(TestOutcome::Skip(reason)) => {
                stats.skipped += 1;
                eprintln!("\nskip {name} ({reason})");
            }
            Err(err) => {
                stats.failed += 1;
                let name = test
                    .path
                    .strip_prefix(root)
                    .unwrap_or(&test.path)
                    .display()
                    .to_string();
                stats.failed_names.push(name.clone());
                if let Some(e) = err.downcast_ref::<TestError>() {
                    eprintln!("\nFAIL {name}");
                    render_test_error(&test.path, e);
                } else if let Some(e) = err.downcast_ref::<UiTestError>() {
                    eprintln!("\nFAIL {name}");
                    render_ui_error(e);
                } else {
                    eprintln!("\nFAIL {name}\n{err:#}");
                }
            }
        }
    }
    Ok(())
}

fn run_test(
    root: &Path,
    bin_dir: Option<&Path>,
    test: &TestCase,
    coverage_dir: Option<&Path>,
    dump_mir: bool,
    update_snapshots: bool,
    verbose: bool,
) -> Result<TestOutcome> {
    if let Some(reason) = directive_text(test, "skip") {
        return Ok(TestOutcome::Skip(reason));
    }

    if test.kind == TestKind::NuDir {
        run_nu_dir_test(
            root,
            bin_dir,
            test,
            coverage_dir,
            dump_mir,
            update_snapshots,
            verbose,
        )?;
        return Ok(TestOutcome::Pass);
    }

    if test.kind == TestKind::Example {
        return run_example_test(root, bin_dir, test, update_snapshots);
    }

    let mode = Mode::from_directive(
        test.directives
            .get("mode")
            .and_then(|v| v.first())
            .context("missing `//@ mode: c|co2|rust|format` directive")?,
    )?;

    if mode == Mode::Format {
        return run_format_test(root, bin_dir, test, update_snapshots);
    }

    let (compile_annotations, sources) = collect_compile_annotations(test, mode)?;
    let directive_warning_expectations = test
        .directives
        .get("compile-warning")
        .cloned()
        .unwrap_or_default();
    let compile = compile_test(root, bin_dir, mode, test, true, coverage_dir, dump_mir)?;
    if test.directives.contains_key("compile-fail") {
        check_ui(test, mode, &compile.output, &compile_annotations, sources)?;
        return Ok(TestOutcome::Pass);
    }

    check_compile_warnings(
        test,
        &compile.output,
        &compile_annotations,
        &directive_warning_expectations,
        sources.clone(),
    )?;

    if !compile.output.status.success() {
        bail!(
            "compilation failed\n{}\n{}",
            format_named_output("stdout", &String::from_utf8_lossy(&compile.output.stdout)),
            format_named_output("stderr", &String::from_utf8_lossy(&compile.output.stderr)),
        );
    }

    let is_debuginfo =
        test.directives.contains_key("gdb-command") || test.directives.contains_key("gdb-check");

    if is_debuginfo {
        debuginfo::run_debuginfo_test(test, &compile)?;
        return Ok(TestOutcome::Pass);
    }

    if !test.directives.contains_key("miri-error") {
        check_run(test, &compile, coverage_dir)?;
    }
    if test.directives.contains_key("run-miri") || test.directives.contains_key("miri-error") {
        match run_miri_test(root, bin_dir, mode, test)? {
            MiriRun::Ran(output) if test.directives.contains_key("miri-error") => {
                check_miri_error(test, &output, &compile_annotations, sources)?;
            }
            MiriRun::Ran(output) => check_run_output(test, &output, "miri")?,
            MiriRun::Unavailable(reason) => return Ok(TestOutcome::Skip(reason)),
        }
    }
    Ok(TestOutcome::Pass)
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

fn run_nu_dir_test(
    root: &Path,
    bin_dir: Option<&Path>,
    test: &TestCase,
    coverage_dir: Option<&Path>,
    dump_mir: bool,
    update_snapshots: bool,
    verbose: bool,
) -> Result<()> {
    let temp = TempDirBuilder::new()
        .prefix("co2-ct-dir-")
        .tempdir()
        .context("failed to create temp dir for Nushell test")?;
    let temp_path = temp.path().join(
        test.path
            .file_name()
            .context("directory test path has no final component")?,
    );
    if verbose {
        println!("Temp dir path = {:?}", temp_path);
        std::mem::forget(temp);
    }
    copy_dir_all(&test.path, &temp_path)?;

    let snapshot_utils_src = root.join("tests").join("snapshot-utils.nu");
    if snapshot_utils_src.exists() {
        std::fs::copy(&snapshot_utils_src, temp_path.join("snapshot-utils.nu"))
            .context("failed to copy snapshot-utils.nu to test temp dir")?;
    }

    let main_nu = temp_path.join("main.nu");
    let run_status = directive_i32(test, "run-status")?.unwrap_or(0);

    let mut cmd = Command::new("nu");
    cmd.arg(&main_nu)
        .current_dir(&temp_path)
        .env("NO_COLOR", "1")
        .env("CARGO_TERM_COLOR", "never")
        .env("CO2_WORKSPACE_ROOT", root)
        .env("CO2_TEST_DIR", &temp_path)
        .env("CO2_TEST_SOURCE_DIR", &test.path);

    if let Some(dir) = bin_dir {
        let path_sep = if cfg!(windows) { ";" } else { ":" };
        let current_path = std::env::var("PATH").unwrap_or_default();
        let merged_path = if current_path.is_empty() {
            dir.to_string_lossy().into_owned()
        } else {
            format!("{}{}{}", dir.display(), path_sep, current_path)
        };
        cmd.env("PATH", merged_path);
        cmd.env("CO2_BIN_DIR", dir);
    } else if let Some(dir) = find_bin_dir_from_path("co2cc") {
        cmd.env("CO2_BIN_DIR", dir);
    }

    if update_snapshots {
        cmd.env("CO2_UPDATE_SNAPSHOTS", "1");
    }

    if dump_mir {
        cmd.env("CO2_DUMP_MIR", "1");
        cmd.env("RUSTFLAGS", "-Zdump-mir=all");
    }

    if let Some(dir) = coverage_dir {
        cmd.env(
            "LLVM_PROFILE_FILE",
            dir.join(format!(
                "compiler-nu-{}-%p-%m.profraw",
                test.path.file_stem().unwrap().to_str().unwrap()
            )),
        );
    }

    let output = cmd
        .output()
        .with_context(|| format!("failed to execute Nushell test {}", main_nu.display()))?;

    if dump_mir {
        copy_dump_dirs(&temp_path, root);
    }

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

fn copy_dump_dirs(src_root: &Path, dst_root: &Path) {
    for dir in ["co2_mir_dump", "mir_dump"] {
        let src = src_root.join(dir);
        if src.exists() {
            let dst = dst_root.join(dir);
            let _ = fs::create_dir_all(&dst);
            if let Ok(entries) = fs::read_dir(&src) {
                for entry in entries.flatten() {
                    let ty = entry.file_type().ok();
                    let name = entry.file_name();
                    let dst_path = dst.join(&name);
                    if ty.is_some_and(|t| t.is_dir()) {
                        let _ = copy_dir_all(&entry.path(), &dst_path);
                    } else {
                        let _ = fs::copy(&entry.path(), &dst_path);
                    }
                }
            }
        }
    }
}

fn run_example_test(
    _root: &Path,
    bin_dir: Option<&Path>,
    test: &TestCase,
    update_snapshots: bool,
) -> Result<TestOutcome> {
    let example_dir = test
        .path
        .parent()
        .and_then(|p| p.parent())
        .context("example path has no parent")?;

    let mut cmd = std::process::Command::new(binary_path(bin_dir, "co2cargo"));
    cmd.current_dir(example_dir).args(["run", "-q"]);

    if let Some(dir) = bin_dir {
        let path_sep = if cfg!(windows) { ";" } else { ":" };
        let current_path = std::env::var("PATH").unwrap_or_default();
        let merged_path = if current_path.is_empty() {
            dir.to_string_lossy().into_owned()
        } else {
            format!("{}{}{}", dir.display(), path_sep, current_path)
        };
        cmd.env("PATH", merged_path);
        cmd.env("CO2_BIN_DIR", dir);
    } else if let Some(dir) = find_bin_dir_from_path("co2cc") {
        cmd.env("CO2_BIN_DIR", dir);
    }

    cmd.env("CO2_TEST_SUITE", "1");

    let run_args = directive_args(test, "run-args")?;
    if !run_args.is_empty() {
        cmd.arg("--").args(run_args);
    }

    let output = cmd.output().with_context(|| {
        format!(
            "failed to run co2cargo example in {}",
            example_dir.display()
        )
    })?;

    let got_status = output.status.code().unwrap_or(-1);
    let expected_status = directive_i32(test, "run-status")?.unwrap_or(0);
    if got_status != expected_status {
        return Err(TestError {
            source: test.source.clone(),
            span: None,
            message: format!(
                "example exit code mismatch: expected `{expected_status}`, got `{got_status}`\n{}",
                format_named_output("stdout", &String::from_utf8_lossy(&output.stdout)),
            ),
        }
        .into());
    }

    let snapshot_path = example_dir.join("expected_output.txt");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();

    if update_snapshots {
        std::fs::write(&snapshot_path, &stdout)
            .with_context(|| format!("failed to write snapshot to {}", snapshot_path.display()))?;
        eprintln!("  updated snapshot: {}", snapshot_path.display());
        return Ok(TestOutcome::Pass);
    }

    if snapshot_path.exists() {
        let expected = std::fs::read_to_string(&snapshot_path)
            .with_context(|| format!("failed to read {}", snapshot_path.display()))?;
        if normalize(&expected) != normalize(&stdout) {
            return Err(TestError {
                source: test.source.clone(),
                span: None,
                message: format!(
                    "example stdout mismatch\n  expected ({}):\n{}\n  actual:\n{}",
                    snapshot_path.display(),
                    expected,
                    stdout,
                ),
            }
            .into());
        }
    }

    Ok(TestOutcome::Pass)
}

fn run_format_test(
    _root: &Path,
    bin_dir: Option<&Path>,
    test: &TestCase,
    update_snapshots: bool,
) -> Result<TestOutcome> {
    let test_dir = test.path.parent().context("test path has no parent")?;
    let stem = test.path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let ugly_path = test_dir.join(format!("{stem}.ugly.co2"));
    if !ugly_path.exists() {
        bail!(
            "format test requires corresponding `{}.ugly.co2` file (expected at {})",
            stem,
            ugly_path.display()
        );
    }

    let output = std::process::Command::new(binary_path(bin_dir, "co2fmt"))
        .arg(&ugly_path)
        .output()
        .with_context(|| format!("failed to execute co2fmt on {}", ugly_path.display()))?;

    if !output.status.success() {
        bail!(
            "co2fmt failed on {}\n{}",
            ugly_path.display(),
            String::from_utf8_lossy(&output.stderr),
        );
    }

    let actual =
        String::from_utf8(output.stdout).with_context(|| "co2fmt output is not valid UTF-8")?;

    if update_snapshots {
        write_snapshot(test, &actual)?;
        return Ok(TestOutcome::Pass);
    }

    let expected: String = test
        .source
        .lines()
        .filter(|l| !l.trim_start().starts_with("//@"))
        .skip_while(|l| l.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let expected = expected.trim().to_owned() + "\n";
    let actual = actual.trim().to_owned() + "\n";
    if actual != *expected {
        bail!(
            "format mismatch\n--- expected ({}):\n||\n{}||\n--- actual ({:?} bytes):\n||\n{}||\n",
            test.path.display(),
            expected,
            actual.len(),
            actual,
        );
    }
    Ok(TestOutcome::Pass)
}

fn write_snapshot(test: &TestCase, actual: &str) -> Result<()> {
    let path = &test.path;
    let source_lines: Vec<&str> = test.source.lines().collect();
    let mut directive_lines = Vec::new();
    let mut found_directives = false;

    for line in &source_lines {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//@") || trimmed.starts_with("#@") {
            directive_lines.push(*line);
            found_directives = true;
        } else if found_directives && trimmed.is_empty() {
            // blank line after directives
            directive_lines.push(*line);
        } else if found_directives {
            break;
        } else {
            break;
        }
    }

    let mut out = String::new();
    for line in &directive_lines {
        out.push_str(line);
        out.push('\n');
    }
    out.push_str(actual);

    std::fs::write(path, &out)
        .with_context(|| format!("failed to write snapshot to {}", path.display()))?;
    eprintln!("  updated snapshot: {}", path.display());
    Ok(())
}

fn check_run(test: &TestCase, compile: &CompileResult, _coverage_dir: Option<&Path>) -> Result<()> {
    let run_args = directive_args(test, "run-args")?;
    let output = Command::new(&compile.exe_path)
        .args(run_args)
        .output()
        .with_context(|| format!("failed to execute {}", compile.exe_path.display()))?;

    check_run_output(test, &output, "run")
}

fn check_run_output(test: &TestCase, output: &Output, runner: &str) -> Result<()> {
    let got_status = output.status.code().unwrap_or(-1);
    let expected_status = directive_i32(test, "run-status")?.unwrap_or(0);
    if got_status != expected_status {
        return Err(TestError {
            source: test.source.clone(),
            span: None,
            message: format!(
                "{runner} exit code mismatch: expected `{expected_status}`, got `{got_status}`\n{}\n{}",
                format_named_output("stdout", &String::from_utf8_lossy(&output.stdout)),
                format_named_output("stderr", &String::from_utf8_lossy(&output.stderr)),
            ),
        }
        .into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if let Some(expected) = output_expectation(test, "run-stdout")?
        && normalize(&expected) != normalize(&stdout)
    {
        return Err(TestError {
            source: test.source.clone(),
            span: None,
            message: format!(
                "{runner} stdout mismatch:\n  expected: {expected}\n  actual:   {stdout}"
            ),
        }
        .into());
    }
    if let Some(expected) = output_expectation(test, "run-stderr")?
        && normalize(&expected) != normalize(&stderr)
    {
        return Err(TestError {
            source: test.source.clone(),
            span: None,
            message: format!(
                "{runner} stderr mismatch:\n  expected: {}\n  actual:   {}",
                expected.lines().next().unwrap_or(""),
                stderr.lines().next().unwrap_or("")
            ),
        }
        .into());
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
                    "{runner} stdout missing pattern `{}` (got `{}`)",
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
                    "{runner} stderr missing pattern `{}` (got `{}`)",
                    pat,
                    stderr.lines().next().unwrap_or("")
                ),
            }
            .into());
        }
    }

    Ok(())
}

fn check_miri_error(
    test: &TestCase,
    output: &Output,
    span_expectations: &[UiSpanExpectation],
    sources: HashMap<String, String>,
) -> Result<()> {
    if output.status.success() {
        bail!("miri-error test unexpectedly succeeded");
    }

    let error_expectations = span_expectations
        .iter()
        .filter(|expected| expected.level == Some(UiAnnotationLevel::Error))
        .collect::<Vec<_>>();
    if error_expectations.is_empty() {
        bail!(
            "miri-error test has no inline error expectations; add a `//^^^^ error: ...` annotation near the UB source: {}",
            test.path.display()
        );
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let diagnostics = parse_miri_rendered_diagnostics(&stderr);
    let mut issues = Vec::new();
    let mut matched = vec![false; diagnostics.len()];

    for expected in error_expectations {
        let mut found = false;
        for (i, diagnostic) in diagnostics.iter().enumerate() {
            if !matched[i] && miri_diagnostic_matches_expected(test, expected, diagnostic) {
                matched[i] = true;
                found = true;
                break;
            }
        }
        if !found {
            issues.push(crate::error::UiTestIssue {
                span: Some(expected.clone()),
                reason: expected.message.as_ref().map_or_else(
                    || "Missing miri error".to_owned(),
                    |message| format!("Missing miri error: {message}"),
                ),
            });
        }
    }

    for (i, diagnostic) in diagnostics.into_iter().enumerate() {
        if !matched[i] {
            let file_name = test.path.file_name().unwrap().to_string_lossy().to_string();
            let (byte_start, byte_end) = if let Some(source) = sources.get(&file_name) {
                let offsets = crate::util::line_start_offsets(source);
                if diagnostic.line > 0 && diagnostic.line <= offsets.len() {
                    let line_start = offsets[diagnostic.line - 1];
                    (
                        line_start + diagnostic.column_start.saturating_sub(1),
                        line_start + diagnostic.column_end.saturating_sub(1),
                    )
                } else {
                    (0, 0)
                }
            } else {
                (0, 0)
            };

            issues.push(crate::error::UiTestIssue {
                span: Some(UiSpanExpectation {
                    file_name,
                    byte_start,
                    byte_end,
                    level: Some(UiAnnotationLevel::Error),
                    message: Some(diagnostic.message.clone()),
                }),
                reason: format!("Unexpected miri error: {}", diagnostic.message),
            });
        }
    }

    if !issues.is_empty() {
        return Err(UiTestError {
            path: test.path.clone(),
            sources,
            issues,
        }
        .into());
    }

    Ok(())
}

#[derive(Debug)]
struct MiriRenderedDiagnostic {
    message: String,
    line: usize,
    column_start: usize,
    column_end: usize,
}

fn parse_miri_rendered_diagnostics(stderr: &str) -> Vec<MiriRenderedDiagnostic> {
    let lines = stderr.lines().collect::<Vec<_>>();
    let mut diagnostics = Vec::new();
    let mut idx = 0;

    while idx < lines.len() {
        let Some(message) = lines[idx].strip_prefix("error: ") else {
            idx += 1;
            continue;
        };

        let mut location = None;
        let mut caret = None;
        let mut cursor = idx + 1;
        while cursor < lines.len() {
            let line = lines[cursor];
            if cursor != idx + 1 && line.starts_with("error: ") {
                break;
            }
            if let Some(line_no) = parse_miri_location(line) {
                location = Some(line_no);
            } else if let Some((column_start, column_end)) = parse_miri_caret(line) {
                caret = Some((column_start, column_end));
            }
            cursor += 1;
        }

        if let (Some(line_no), Some((caret_start, caret_end))) = (location, caret) {
            diagnostics.push(MiriRenderedDiagnostic {
                message: message.to_owned(),
                line: line_no,
                column_start: caret_start,
                column_end: caret_end,
            });
        }
        idx = cursor;
    }

    diagnostics
}

fn parse_miri_location(line: &str) -> Option<usize> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix("--> ")?;
    let mut parts = rest.rsplitn(3, ':');
    let _column: usize = parts.next()?.parse().ok()?;
    let line_no = parts.next()?.parse().ok()?;
    Some(line_no)
}

fn parse_miri_caret(line: &str) -> Option<(usize, usize)> {
    let after_pipe = line.split_once('|')?.1;
    let caret_index = after_pipe.find('^')?;
    let caret_count = after_pipe[caret_index..]
        .chars()
        .take_while(|ch| *ch == '^')
        .count();
    Some((caret_index, caret_index + caret_count))
}

fn miri_diagnostic_matches_expected(
    test: &TestCase,
    expected: &UiSpanExpectation,
    diagnostic: &MiriRenderedDiagnostic,
) -> bool {
    if let Some(message) = &expected.message
        && diagnostic.message != *message
    {
        return false;
    }

    let Some((line, column_start, column_end)) = source_span_line_columns(test, expected) else {
        return false;
    };
    diagnostic.line == line
        && diagnostic.column_start == column_start
        && diagnostic.column_end == column_end
}

fn source_span_line_columns(
    test: &TestCase,
    expected: &UiSpanExpectation,
) -> Option<(usize, usize, usize)> {
    let line_starts = line_start_offsets(&test.source);
    let line_idx = line_starts
        .iter()
        .enumerate()
        .rev()
        .find(|(_, start)| **start <= expected.byte_start)?
        .0;
    let line_start = line_starts[line_idx];
    Some((
        line_idx + 1,
        expected.byte_start - line_start + 1,
        expected.byte_end - line_start + 1,
    ))
}

fn output_expectation(test: &TestCase, key: &str) -> Result<Option<String>> {
    let Some(expected) = directive_text(test, key) else {
        return Ok(None);
    };

    if let Some(path) = expected.strip_prefix("FILE:") {
        let expected_path = test
            .path
            .parent()
            .context("test path has no parent")?
            .join(path.trim());
        return fs::read_to_string(&expected_path)
            .with_context(|| format!("failed to read {}", expected_path.display()))
            .map(Some);
    }

    Ok(Some(unescape_text(&expected)))
}
