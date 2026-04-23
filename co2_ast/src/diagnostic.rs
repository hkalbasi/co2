use std::any::Any;
use std::sync::{
    Once,
    Mutex,
    Arc,
    atomic::{AtomicBool, Ordering},
};

use ariadne::{Color, Label, Report, ReportKind, sources};
use chumsky::{error::Rich, span::SimpleSpan};
use serde_json::json;

use crate::Token;

static ERRORS: Mutex<Vec<Rich<'static, Token, SimpleSpan>>> = Mutex::new(Vec::new());
static DIAGNOSTICS_EMITTED: AtomicBool = AtomicBool::new(false);
static INSTALL_HOOK: Once = Once::new();
static DIAGNOSTIC_MAPPER: Mutex<Option<DiagnosticMapper>> = Mutex::new(None);

#[derive(Clone)]
pub struct DiagnosticSpan {
    pub file_name: String,
    pub source: Arc<str>,
    pub start: usize,
    pub end: usize,
}

type DiagnosticMapper = Arc<dyn Fn(SimpleSpan) -> Option<DiagnosticSpan> + Send + Sync>;

#[derive(Debug)]
pub struct DiagnosticAbort;

pub fn take_errors() -> Vec<Rich<'static, Token, SimpleSpan>> {
    let mut guard = ERRORS.try_lock().unwrap();
    std::mem::take(&mut *guard)
}

pub fn safe_range(span: SimpleSpan, src_len: usize) -> std::ops::Range<usize> {
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

pub fn set_diagnostic_mapper(mapper: DiagnosticMapper) {
    *DIAGNOSTIC_MAPPER.try_lock().unwrap() = Some(mapper);
}

pub fn clear_diagnostic_mapper() {
    *DIAGNOSTIC_MAPPER.try_lock().unwrap() = None;
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
    filename: String,
    src: &'static str,
    errs: Vec<Rich<'_, char>>,
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

pub fn emit_mapped_errors_and_terminate(
    filename: String,
    src: &'static str,
    errs: Vec<Rich<'_, String, SimpleSpan>>,
) -> ! {
    DIAGNOSTICS_EMITTED.store(true, Ordering::SeqCst);
    if std::env::var_os("CO2_FORCE_JSON_DIAGNOSTICS").is_some() {
        for e in errs {
            emit_json_diagnostic(&filename, src, &e);
        }
    } else {
        for e in errs {
            emit_human_diagnostic(&filename, src, &e);
        }
    }
    panic_with_diagnostic_abort();
}

fn emit_human_diagnostic(filename: &str, src: &str, e: &Rich<'_, String, SimpleSpan>) {
    if let Some(mapped) = map_diagnostic_span(*e.span()) {
        Report::build(ReportKind::Error, (mapped.file_name.clone(), mapped.start..mapped.end))
            .with_config(ariadne::Config::new().with_index_type(ariadne::IndexType::Byte))
            .with_message(e.to_string())
            .with_label(
                Label::new((mapped.file_name.clone(), mapped.start..mapped.end))
                    .with_message(e.reason().to_string())
                    .with_color(Color::Red),
            )
            .finish()
            .eprint(sources([(mapped.file_name, mapped.source.to_string())]))
            .unwrap();
        return;
    }

    let range = safe_range(*e.span(), src.len());
    Report::build(ReportKind::Error, (filename.to_owned(), range.clone()))
        .with_config(ariadne::Config::new().with_index_type(ariadne::IndexType::Byte))
        .with_message(e.to_string())
        .with_label(
            Label::new((filename.to_owned(), range))
                .with_message(e.reason().to_string())
                .with_color(Color::Red),
        )
        .with_labels(e.contexts().map(|(label, span)| {
            Label::new((filename.to_owned(), safe_range(*span, src.len())))
                .with_message(format!("while parsing this {label}"))
                .with_color(Color::Yellow)
        }))
        .finish()
        .eprint(sources([(filename.to_owned(), src.to_owned())]))
        .unwrap();
}

fn emit_json_diagnostic(filename: &str, src: &str, e: &Rich<'_, String, SimpleSpan>) {
    if let Some(mapped) = map_diagnostic_span(*e.span()) {
        let range = mapped.start..mapped.end;
        let diagnostic = json!({
            "$message_type": "diagnostic",
            "message": e.to_string(),
            "code": null,
            "level": "error",
            "spans": [json_span_unadjusted(&mapped.file_name, &mapped.source, range, true, Some(e.reason().to_string()))],
            "children": [],
            "rendered": null,
        });
        eprintln!("{diagnostic}");
        return;
    }

    let range = safe_range(*e.span(), src.len());
    let mut spans = vec![json_span(filename, src, range.clone(), true, Some(e.reason().to_string()))];
    spans.extend(e.contexts().map(|(label, span)| {
        json_span(
            filename,
            src,
            safe_range(*span, src.len()),
            false,
            Some(format!("while parsing this {label}")),
        )
    }));

    let diagnostic = json!({
        "$message_type": "diagnostic",
        "message": e.to_string(),
        "code": null,
        "level": "error",
        "spans": spans,
        "children": [],
        "rendered": null,
    });
    eprintln!("{diagnostic}");
}

fn json_span(
    filename: &str,
    src: &str,
    range: std::ops::Range<usize>,
    is_primary: bool,
    label: Option<String>,
) -> serde_json::Value {
    let byte_offset = diagnostic_byte_offset();
    let adjusted_start = range.start.saturating_add_signed(byte_offset);
    let adjusted_end = range.end.saturating_add_signed(byte_offset);
    let (line_start, column_start) = byte_to_line_col(src, adjusted_start);
    let (line_end, column_end) = byte_to_line_col(src, adjusted_end);
    json!({
        "file_name": filename,
        "byte_start": adjusted_start,
        "byte_end": adjusted_end,
        "line_start": line_start,
        "line_end": line_end,
        "column_start": column_start,
        "column_end": column_end,
        "is_primary": is_primary,
        "text": [],
        "label": label,
        "suggested_replacement": null,
        "suggestion_applicability": null,
        "expansion": null,
    })
}

fn json_span_unadjusted(
    filename: &str,
    src: &str,
    range: std::ops::Range<usize>,
    is_primary: bool,
    label: Option<String>,
) -> serde_json::Value {
    let (line_start, column_start) = byte_to_line_col(src, range.start);
    let (line_end, column_end) = byte_to_line_col(src, range.end);
    let byte_start = ui_effective_byte_offset(src, range.start);
    let byte_end = ui_effective_byte_offset(src, range.end);
    json!({
        "file_name": filename,
        "byte_start": byte_start,
        "byte_end": byte_end,
        "line_start": line_start,
        "line_end": line_end,
        "column_start": column_start,
        "column_end": column_end,
        "is_primary": is_primary,
        "text": [],
        "label": label,
        "suggested_replacement": null,
        "suggestion_applicability": null,
        "expansion": null,
    })
}

fn ui_effective_byte_offset(src: &str, byte_idx: usize) -> usize {
    let line_starts = line_start_offsets(src);
    let line_idx = match line_starts.binary_search(&byte_idx) {
        Ok(i) => i,
        Err(i) => i.saturating_sub(1),
    };
    let line_start = line_starts[line_idx];
    let col = byte_idx.saturating_sub(line_start);

    let mut effective = 0usize;
    for (idx, line) in src.lines().enumerate() {
        if idx == line_idx {
            return effective + col.min(line.len());
        }
        effective += if is_ui_span_annotation(line) { 1 } else { line.len() + 1 };
    }
    effective
}

fn line_start_offsets(src: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (idx, b) in src.bytes().enumerate() {
        if b == b'\n' {
            starts.push(idx + 1);
        }
    }
    starts
}

fn is_ui_span_annotation(line: &str) -> bool {
    let Some(comment_start) = line.find("//") else {
        return false;
    };
    if !line[..comment_start].chars().all(char::is_whitespace) {
        return false;
    }
    let body = &line[comment_start + 2..];
    let Some(caret_offset) = body.find('^') else {
        return false;
    };
    body[..caret_offset].chars().all(char::is_whitespace)
}

fn diagnostic_byte_offset() -> isize {
    std::env::var("CO2_JSON_BYTE_OFFSET")
        .ok()
        .and_then(|raw| raw.parse::<isize>().ok())
        .unwrap_or(0)
}

fn map_diagnostic_span(span: SimpleSpan) -> Option<DiagnosticSpan> {
    let guard = DIAGNOSTIC_MAPPER.try_lock().unwrap();
    let mapper = guard.as_ref()?;
    mapper(span)
}

fn byte_to_line_col(src: &str, byte_idx: usize) -> (usize, usize) {
    let clamped = byte_idx.min(src.len());
    let prefix = &src[..clamped];
    let line = prefix.bytes().filter(|b| *b == b'\n').count() + 1;
    let col = prefix
        .rsplit_once('\n')
        .map(|(_, tail)| tail.chars().count() + 1)
        .unwrap_or_else(|| prefix.chars().count() + 1);
    (line, col)
}
