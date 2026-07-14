use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::Output;

use anyhow::{Context, Result, bail};

use crate::error::{UiTestError, UiTestIssue};
use crate::test_case::{Mode, TestCase};
use crate::util::line_start_offsets;

#[derive(Debug, Clone)]
pub struct UiSpanExpectation {
    pub file_name: String,
    pub byte_start: usize,
    pub byte_end: usize,
    pub level: Option<UiAnnotationLevel>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiAnnotationLevel {
    Error,
    Warning,
    Help,
}

impl UiAnnotationLevel {
    fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Help => "help",
        }
    }
}

#[derive(Debug, Clone)]
pub struct UiDiagnostic {
    pub level: String,
    pub message: String,
    pub spans: Vec<UiDiagnosticSpan>,
    pub rendered: Option<String>,
}

#[derive(Debug, Clone)]
pub struct UiDiagnosticSpan {
    pub file_name: String,
    pub byte_start: usize,
    pub byte_end: usize,
    pub is_primary: bool,
}

pub fn check_ui(
    test: &TestCase,
    mode: Mode,
    output: &Output,
    span_expectations: &[UiSpanExpectation],
    sources: HashMap<String, String>,
) -> Result<()> {
    if output.status.success() {
        bail!("UI test unexpectedly succeeded");
    }
    let expected_status = match mode {
        Mode::Rust => 1,
        Mode::C | Mode::Co2 => 5,
        Mode::Format => unreachable!("format tests do not support compile-fail"),
    };
    let got_status = output.status.code().unwrap_or(-1);
    if got_status != expected_status {
        bail!(
            "compile-fail status mismatch: expected {expected_status}, got {got_status}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    if test.directives.contains_key("ui-error")
        || test.directives.contains_key("ui-stderr-contains")
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

    if !span_expectations
        .iter()
        .any(|expected| expected.message.is_some())
    {
        bail!(
            "UI test must include diagnostic text in at least one inline span annotation: {}",
            test.path.display()
        );
    }

    let diagnostics = parse_ui_diagnostics(&stderr)?;
    let mut issues = Vec::new();

    for expected in span_expectations {
        let message_matches = diagnostics
            .iter()
            .any(|diagnostic| diagnostic_matches_expected(expected, diagnostic));

        if !message_matches {
            issues.push(missing_expected_issue(expected, &diagnostics));
        }
    }

    for diagnostic in &diagnostics {
        if is_summary_diagnostic(diagnostic) {
            continue;
        }

        let matched = span_expectations
            .iter()
            .any(|expected| diagnostic_matches_expected(expected, diagnostic));

        if !matched {
            issues.push(unexpected_diagnostic_issue(diagnostic));
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

pub fn check_compile_warnings(
    test: &TestCase,
    output: &Output,
    span_expectations: &[UiSpanExpectation],
    directive_expectations: &[String],
    sources: HashMap<String, String>,
) -> Result<()> {
    if !output.status.success() {
        return Ok(());
    }

    let diagnostics = parse_compile_diagnostics(&String::from_utf8_lossy(&output.stderr));
    let warning_expectations = span_expectations
        .iter()
        .filter(|expected| expected.level == Some(UiAnnotationLevel::Warning))
        .collect::<Vec<_>>();
    let warnings = diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.level == UiAnnotationLevel::Warning.as_str()
                && !is_summary_diagnostic(diagnostic)
        })
        .collect::<Vec<_>>();

    if warning_expectations.is_empty() && warnings.is_empty() {
        return Ok(());
    }

    let mut issues = Vec::new();

    for expected in &warning_expectations {
        if !warnings
            .iter()
            .any(|diagnostic| diagnostic_matches_expected(expected, diagnostic))
        {
            issues.push(missing_expected_issue(expected, &diagnostics));
        }
    }

    for expected in directive_expectations {
        if !warnings
            .iter()
            .any(|diagnostic| diagnostic.message == *expected)
        {
            issues.push(UiTestIssue {
                span: None,
                reason: format!("Missing warning: {expected}"),
            });
        }
    }

    for diagnostic in warnings {
        if !warning_expectations
            .iter()
            .any(|expected| diagnostic_matches_expected(expected, diagnostic))
            && !directive_expectations.contains(&diagnostic.message)
        {
            issues.push(unexpected_diagnostic_issue(diagnostic));
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

pub fn parse_ui_span_expectations(path: &Path, _mode: Mode) -> Result<Vec<UiSpanExpectation>> {
    let src = fs::read_to_string(path)
        .with_context(|| format!("failed to read test source {}", path.display()))?;
    let line_starts = line_start_offsets(&src);
    let mut out = Vec::new();

    let file_name = path.file_name().unwrap().to_string_lossy().into_owned();

    for (idx, line) in src.lines().enumerate() {
        let Some((column_start, column_end, level, message)) = parse_ui_span_annotation(line)
        else {
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
            file_name: file_name.clone(),
            byte_start: line_start + (column_start - 1),
            byte_end: line_start + (column_end - 1),
            level,
            message,
        });
    }

    Ok(out)
}

fn parse_ui_span_annotation(
    line: &str,
) -> Option<(usize, usize, Option<UiAnnotationLevel>, Option<String>)> {
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
    let (level, message) = parse_annotation_message(trailing);
    Some((column_start, column_end, level, message))
}

pub fn parse_ui_diagnostics(stderr: &str) -> Result<Vec<UiDiagnostic>> {
    let diagnostics = parse_json_diagnostics(stderr);
    if diagnostics.is_empty() {
        bail!("failed to find JSON diagnostics in stderr");
    }
    Ok(diagnostics)
}

pub fn parse_compile_diagnostics(stderr: &str) -> Vec<UiDiagnostic> {
    parse_json_diagnostics(stderr)
}

pub fn prettify_diagnostic_output(text: &str) -> String {
    let mut out = String::new();
    let mut changed = false;

    for line in text.lines() {
        let Some((prefix, diagnostic)) = parse_diagnostic_line(line) else {
            out.push_str(line);
            out.push('\n');
            continue;
        };
        changed = true;

        if !prefix.trim_end().is_empty() {
            out.push_str(prefix.trim_end());
            out.push('\n');
        }

        if let Some(rendered) = diagnostic.rendered.as_deref() {
            out.push_str(rendered.trim_end_matches('\n'));
            out.push('\n');
        } else {
            use std::fmt::Write;
            writeln!(out, "{}: {}", diagnostic.level, diagnostic.message).unwrap();
        }
    }

    if !changed {
        return text.to_owned();
    }

    out.trim_end_matches('\n').to_owned()
}

pub fn format_named_output(name: &str, text: &str) -> String {
    let pretty = prettify_diagnostic_output(text);
    format!("{name}:\n{pretty}")
}

fn parse_json_diagnostics(stderr: &str) -> Vec<UiDiagnostic> {
    let mut diagnostics = Vec::new();

    for line in stderr.lines() {
        let Some((_, diagnostic)) = parse_diagnostic_line(line) else {
            continue;
        };
        diagnostics.push(diagnostic);
    }

    diagnostics
}

fn parse_diagnostic_line(line: &str) -> Option<(&str, UiDiagnostic)> {
    let json_start = line.find('{')?;
    let prefix = &line[..json_start];
    let trimmed = line[json_start..].trim();
    if trimmed.is_empty() || !trimmed.starts_with('{') {
        return None;
    }

    let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
        return None;
    };
    if value
        .get("$message_type")
        .and_then(serde_json::Value::as_str)
        != Some("diagnostic")
    {
        return None;
    }
    let level = value.get("level")?.as_str()?;
    let message = value.get("message")?.as_str()?;

    let spans = value
        .get("spans")
        .and_then(serde_json::Value::as_array)
        .map(|spans| {
            spans
                .iter()
                .filter_map(|span| {
                    Some(UiDiagnosticSpan {
                        file_name: span.get("file_name")?.as_str()?.to_owned(),
                        byte_start: span.get("byte_start")?.as_u64()? as usize,
                        byte_end: span.get("byte_end")?.as_u64()? as usize,
                        is_primary: span.get("is_primary")?.as_bool()?,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Some((
        prefix,
        UiDiagnostic {
            level: level.to_owned(),
            message: message.to_owned(),
            spans,
            rendered: value
                .get("rendered")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
        },
    ))
}

fn parse_annotation_message(trailing: &str) -> (Option<UiAnnotationLevel>, Option<String>) {
    if trailing.is_empty() {
        return (None, None);
    }

    for (prefix, level) in [
        ("error:", UiAnnotationLevel::Error),
        ("warning:", UiAnnotationLevel::Warning),
        ("help:", UiAnnotationLevel::Help),
    ] {
        if let Some(message) = trailing.strip_prefix(prefix) {
            return (Some(level), Some(message.trim().to_owned()));
        }
    }

    (None, Some(trailing.to_owned()))
}

fn diagnostic_matches_expected(expected: &UiSpanExpectation, diagnostic: &UiDiagnostic) -> bool {
    if let Some(level) = expected.level
        && diagnostic.level != level.as_str()
    {
        return false;
    }

    if let Some(message) = &expected.message
        && diagnostic.message != *message
    {
        return false;
    }

    diagnostic.spans.iter().any(|span| {
        span.is_primary
            && span.file_name.ends_with(&expected.file_name)
            && span.byte_start == expected.byte_start
            && span.byte_end == expected.byte_end
    })
}

fn missing_expected_issue(
    expected: &UiSpanExpectation,
    diagnostics: &[UiDiagnostic],
) -> UiTestIssue {
    let diagnostic_kind = expected.level.map_or("diagnostic", |level| level.as_str());
    let reason = if let Some(msg) = &expected.message {
        let found = diagnostics.iter().any(|diagnostic| {
            diagnostic.message == *msg
                && expected
                    .level
                    .is_none_or(|level| diagnostic.level == level.as_str())
        });
        if found {
            format!(
                "Missing {diagnostic_kind} span in {} for: {}",
                expected.file_name, msg
            )
        } else {
            format!("Missing {diagnostic_kind}: {msg}")
        }
    } else {
        format!(
            "missing {diagnostic_kind} span annotation in {}",
            expected.file_name
        )
    };

    UiTestIssue {
        span: Some(expected.clone()),
        reason,
    }
}

fn unexpected_diagnostic_issue(diagnostic: &UiDiagnostic) -> UiTestIssue {
    if let Some(primary_span) = diagnostic.spans.iter().find(|span| span.is_primary) {
        UiTestIssue {
            span: Some(UiSpanExpectation {
                file_name: primary_span.file_name.clone(),
                message: Some(diagnostic.message.clone()),
                byte_start: primary_span.byte_start,
                byte_end: primary_span.byte_end,
                level: None,
            }),
            reason: format!("Unexpected {}: {}", diagnostic.level, diagnostic.message),
        }
    } else {
        UiTestIssue {
            span: None,
            reason: format!("Unexpected {}: {}", diagnostic.level, diagnostic.message),
        }
    }
}

fn is_summary_diagnostic(diagnostic: &UiDiagnostic) -> bool {
    diagnostic.level == "failure-note"
        || diagnostic.message.starts_with("aborting due to ")
        || diagnostic.message.ends_with(" warning emitted")
        || diagnostic.message.ends_with(" warnings emitted")
}
