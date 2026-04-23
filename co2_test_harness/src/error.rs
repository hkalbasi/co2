use std::path::{Path, PathBuf};

use ariadne::{Color, Label, Report, ReportKind, Source};

#[derive(Debug, Clone)]
pub struct TestError {
    #[allow(dead_code)]
    pub source: String,
    pub span: Option<(usize, usize)>,
    pub message: String,
}

impl std::fmt::Display for TestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for TestError {}

#[derive(Debug, Clone)]
pub struct UiTestError {
    pub path: PathBuf,
    pub source: String,
    pub issues: Vec<UiTestIssue>,
}

#[derive(Debug, Clone)]
pub struct UiTestIssue {
    pub span: Option<crate::ui::UiSpanExpectation>,
    pub reason: String,
}

impl std::fmt::Display for UiTestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} issue(s)", self.issues.len())
    }
}

impl std::error::Error for UiTestError {}

pub fn render_ui_error(err: &UiTestError) {
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

pub fn render_test_error(path: &Path, err: &TestError) {
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
