use std::any::Any;
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex, Once,
    atomic::{AtomicBool, Ordering},
};

use ariadne::{Color, Label, Report, ReportKind, sources};
use chumsky::error::Rich;
use serde_json::json;

use crate::{FileId, Span, Token};

static ERRORS: Mutex<Vec<Rich<'static, Token, Span>>> = Mutex::new(Vec::new());
static DIAGNOSTICS_EMITTED: AtomicBool = AtomicBool::new(false);
static FORCE_JSON_DIAGNOSTICS: AtomicBool = AtomicBool::new(false);
static INSTALL_HOOK: Once = Once::new();
static SOURCE_MAP: Mutex<Option<Arc<dyn SourceMap>>> = Mutex::new(None);

pub trait SourceMap: Send + Sync {
    fn get_file_info(&self, id: FileId) -> Option<(String, Arc<str>)>;
}

#[derive(Clone)]
pub struct DiagnosticSpan {
    pub file_name: String,
    pub source: Arc<str>,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug)]
pub struct DiagnosticAbort;

pub fn take_errors() -> Vec<Rich<'static, Token, Span>> {
    let mut guard = ERRORS.try_lock().unwrap();
    std::mem::take(&mut *guard)
}

pub fn byte_to_line_col(src: &str, byte_pos: usize) -> (usize, usize) {
    let byte_pos = byte_pos.min(src.len());
    let mut line = 1;
    let mut col = 1;
    for (i, c) in src.char_indices() {
        if i >= byte_pos {
            break;
        }
        if c == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

pub fn safe_range(span: Span, src_len: usize) -> std::ops::Range<usize> {
    let span = span.data();
    let mut start = span.start.min(src_len);
    let mut end = span.end.min(src_len);
    if end < start {
        std::mem::swap(&mut start, &mut end);
    }
    start..end
}

pub fn reset_diagnostic_state() {
    install_diagnostic_panic_hook();
    DIAGNOSTICS_EMITTED.store(false, Ordering::SeqCst);
}

pub fn set_source_map(source_map: Arc<dyn SourceMap>) {
    *SOURCE_MAP.try_lock().unwrap() = Some(source_map);
}

pub fn set_force_json_diagnostics(force: bool) {
    FORCE_JSON_DIAGNOSTICS.store(force, Ordering::SeqCst);
}

pub fn diagnostics_were_emitted() -> bool {
    DIAGNOSTICS_EMITTED.load(Ordering::SeqCst)
}

pub fn panic_with_diagnostic_abort() -> ! {
    install_diagnostic_panic_hook();
    std::panic::panic_any(DiagnosticAbort);
}

pub fn is_diagnostic_abort(payload: &(dyn Any + Send)) -> bool {
    payload.is::<DiagnosticAbort>()
}

fn install_diagnostic_panic_hook() {
    INSTALL_HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if info.payload().is::<DiagnosticAbort>() {
                return;
            }
            previous(info);
        }));
    });
}

pub fn print_errors_and_terminate(
    filename: &str,
    src: &'static str,
    errs: Vec<Rich<'_, char, Span>>,
) -> ! {
    let errs = errs
        .into_iter()
        .map(|e| e.map_token(|c| c.to_string()))
        .chain(
            take_errors()
                .into_iter()
                .map(|e| e.map_token(|tok| tok.to_string())),
        )
        .collect();
    emit_mapped_errors_and_terminate(filename, src, errs);
}

pub fn emit_errors_and_terminate(errs: Vec<Rich<'_, String, Span>>) -> ! {
    emit_mapped_diagnostics("<unknown>", "", errs, DiagnosticLevel::Error, true);
    unreachable!("fatal diagnostics should abort");
}

pub fn emit_errors(errs: Vec<Rich<'_, String, Span>>) {
    emit_mapped_diagnostics("<unknown>", "", errs, DiagnosticLevel::Error, false);
}

pub fn emit_warnings(warnings: Vec<Rich<'_, String, Span>>) {
    emit_mapped_diagnostics("<unknown>", "", warnings, DiagnosticLevel::Warning, false);
}

fn emit_mapped_errors_and_terminate(
    filename: &str,
    src: &'static str,
    errs: Vec<Rich<'_, String, Span>>,
) -> ! {
    emit_mapped_diagnostics(filename, src, errs, DiagnosticLevel::Error, true);
    unreachable!("fatal diagnostics should abort");
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DiagnosticLevel {
    Error,
    Warning,
}

impl DiagnosticLevel {
    fn report_kind(self) -> ReportKind<'static> {
        match self {
            DiagnosticLevel::Error => ReportKind::Error,
            DiagnosticLevel::Warning => ReportKind::Warning,
        }
    }

    fn label_color(self) -> Color {
        match self {
            DiagnosticLevel::Error => Color::Red,
            DiagnosticLevel::Warning => Color::Yellow,
        }
    }

    fn json_level(self) -> &'static str {
        match self {
            DiagnosticLevel::Error => "error",
            DiagnosticLevel::Warning => "warning",
        }
    }
}

fn emit_mapped_diagnostics(
    filename: &str,
    src: &'static str,
    diagnostics: Vec<Rich<'_, String, Span>>,
    level: DiagnosticLevel,
    terminate: bool,
) {
    DIAGNOSTICS_EMITTED.store(true, Ordering::SeqCst);
    if FORCE_JSON_DIAGNOSTICS.load(Ordering::SeqCst)
        || std::env::var_os("CO2_FORCE_JSON_DIAGNOSTICS").is_some()
    {
        for e in diagnostics {
            emit_json_diagnostic(filename, src, &e, level);
        }
    } else {
        for e in diagnostics {
            emit_human_diagnostic(filename, src, &e, level);
        }
    }
    if terminate {
        panic_with_diagnostic_abort();
    } else if level == DiagnosticLevel::Warning {
        DIAGNOSTICS_EMITTED.store(false, Ordering::SeqCst);
    }
}

static DIAGNOSTIC_BASE_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

pub fn set_diagnostic_base_path(path: Option<PathBuf>) {
    let mut guard = DIAGNOSTIC_BASE_PATH.lock().unwrap();
    *guard = path;
}

fn relativize_path(path: &str) -> String {
    let path = std::path::Path::new(path);
    let guard = DIAGNOSTIC_BASE_PATH.lock().unwrap();
    if let Some(base) = guard.as_ref()
        && let Ok(relative) = path.strip_prefix(base)
    {
        return relative.display().to_string();
    }
    path.display().to_string()
}

fn emit_human_diagnostic(
    filename: &str,
    src: &str,
    e: &Rich<'_, String, Span>,
    level: DiagnosticLevel,
) {
    if let Some(mapped) = get_diagnostic_info(*e.span()) {
        let display_name = relativize_path(&mapped.file_name);
        Report::build(
            level.report_kind(),
            (display_name.clone(), mapped.start..mapped.end),
        )
        .with_config(ariadne::Config::new().with_index_type(ariadne::IndexType::Byte))
        .with_message(e.to_string())
        .with_label(
            Label::new((display_name.clone(), mapped.start..mapped.end))
                .with_message(e.reason().to_string())
                .with_color(level.label_color()),
        )
        .finish()
        .eprint(sources([(display_name, mapped.source.to_string())]))
        .unwrap();
        return;
    }

    let range = safe_range(*e.span(), src.len());
    let display_name = relativize_path(filename);
    Report::build(level.report_kind(), (display_name.clone(), range.clone()))
        .with_config(ariadne::Config::new().with_index_type(ariadne::IndexType::Byte))
        .with_message(e.to_string())
        .with_label(
            Label::new((display_name.clone(), range))
                .with_message(e.reason().to_string())
                .with_color(level.label_color()),
        )
        .with_labels(e.contexts().map(|(label, span)| {
            Label::new((display_name.clone(), safe_range(*span, src.len())))
                .with_message(format!("while parsing this {label}"))
                .with_color(Color::Yellow)
        }))
        .finish()
        .eprint(sources([(display_name, src.to_owned())]))
        .unwrap();
}

fn emit_json_diagnostic(
    filename: &str,
    src: &str,
    e: &Rich<'_, String, Span>,
    level: DiagnosticLevel,
) {
    if let Some(mapped) = get_diagnostic_info(*e.span()) {
        let range = mapped.start..mapped.end;
        let display_name = relativize_path(&mapped.file_name);
        let (ls, cs) = byte_to_line_col(&mapped.source, mapped.start);
        let (le, ce) = byte_to_line_col(&mapped.source, mapped.end);
        let mut rendered = Vec::new();
        Report::build(level.report_kind(), (display_name.clone(), range.clone()))
            .with_config(ariadne::Config::new().with_index_type(ariadne::IndexType::Byte))
            .with_message(e.to_string())
            .with_label(
                Label::new((display_name.clone(), range.clone()))
                    .with_message(e.reason().to_string())
                    .with_color(level.label_color()),
            )
            .finish()
            .write(
                sources([(display_name.clone(), mapped.source.to_string())]),
                &mut rendered,
            )
            .unwrap();
        let label = e.reason().to_string();
        let diagnostic = json!({
            "$message_type": "diagnostic",
            "message": e.to_string(),
            "code": null,
            "level": level.json_level(),
            "spans": [json_span(&display_name, range, true, Some(&label), ls, cs, le, ce)],
            "children": [],
            "rendered": String::from_utf8(rendered).unwrap(),
        });
        eprintln!("{diagnostic}");
        return;
    }

    let range = safe_range(*e.span(), src.len());
    let display_name = relativize_path(filename);
    let (ls, cs) = byte_to_line_col(src, range.start);
    let (le, ce) = byte_to_line_col(src, range.end);
    let primary_label = e.reason().to_string();
    let mut spans = vec![json_span(
        &display_name,
        range.clone(),
        true,
        Some(&primary_label),
        ls,
        cs,
        le,
        ce,
    )];
    spans.extend(e.contexts().map(|(label, span)| {
        let context_label = format!("while parsing this {label}");
        let span_range = safe_range(*span, src.len());
        let (sl, sc) = byte_to_line_col(src, span_range.start);
        let (el, ec) = byte_to_line_col(src, span_range.end);
        json_span(
            &display_name,
            span_range,
            false,
            Some(&context_label),
            sl,
            sc,
            el,
            ec,
        )
    }));
    let mut rendered = Vec::new();
    Report::build(level.report_kind(), (display_name.clone(), range.clone()))
        .with_config(ariadne::Config::new().with_index_type(ariadne::IndexType::Byte))
        .with_message(e.to_string())
        .with_label(
            Label::new((display_name.clone(), range.clone()))
                .with_message(e.reason().to_string())
                .with_color(level.label_color()),
        )
        .with_labels(e.contexts().map(|(label, span)| {
            Label::new((display_name.clone(), safe_range(*span, src.len())))
                .with_message(format!("while parsing this {label}"))
                .with_color(Color::Yellow)
        }))
        .finish()
        .write(
            sources([(display_name.clone(), src.to_owned())]),
            &mut rendered,
        )
        .unwrap();

    let diagnostic = json!({
        "$message_type": "diagnostic",
        "message": e.to_string(),
        "code": null,
        "level": level.json_level(),
        "spans": spans,
        "children": [],
        "rendered": String::from_utf8(rendered).unwrap(),
    });
    eprintln!("{diagnostic}");
}

fn json_span(
    filename: &str,
    range: std::ops::Range<usize>,
    is_primary: bool,
    label: Option<&str>,
    line_start: usize,
    col_start: usize,
    line_end: usize,
    col_end: usize,
) -> serde_json::Value {
    json!({
        "file_name": filename,
        "byte_start": range.start,
        "byte_end": range.end,
        "line_start": line_start,
        "line_end": line_end,
        "column_start": col_start,
        "column_end": col_end,
        "is_primary": is_primary,
        "text": [],
        "label": label,
        "suggested_replacement": null,
        "suggestion_applicability": null,
        "expansion": null,
    })
}

fn get_diagnostic_info(span: Span) -> Option<DiagnosticSpan> {
    let guard = SOURCE_MAP.try_lock().unwrap();
    let sm = guard.as_ref()?;
    let span = span.data();
    let (file_name, source) = sm.get_file_info(span.context)?;
    Some(DiagnosticSpan {
        file_name,
        source,
        start: span.start,
        end: span.end,
    })
}
