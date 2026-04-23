use std::fs;
use std::path::Path;
use std::process::Output;

use anyhow::{Context, Result, bail};

use crate::test_case::{Mode, TestCase};
use crate::error::{UiTestError, UiTestIssue};
use crate::util::{line_start_offsets};

#[derive(Debug, Clone)]
pub struct UiSpanExpectation {
    pub byte_start: usize,
    pub byte_end: usize,
    pub message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct UiDiagnostic {
    pub message: String,
    pub spans: Vec<UiDiagnosticSpan>,
}

#[derive(Debug, Clone)]
pub struct UiDiagnosticSpan {
    pub byte_start: usize,
    pub byte_end: usize,
    pub is_primary: bool,
}

pub fn check_ui(
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

pub fn parse_ui_span_expectations(path: &Path, _mode: Mode) -> Result<Vec<UiSpanExpectation>> {
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

pub fn parse_ui_diagnostics(stderr: &str) -> Result<Vec<UiDiagnostic>> {
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
