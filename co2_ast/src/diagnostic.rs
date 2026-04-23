use std::any::Any;
use std::sync::{
    Once,
    Mutex,
    Arc,
    atomic::{AtomicBool, Ordering},
};

use ariadne::{Color, Label, Report, ReportKind, sources};
use chumsky::error::Rich;
use serde_json::json;

use crate::{Token, Span, FileId};

static ERRORS: Mutex<Vec<Rich<'static, Token, Span>>> = Mutex::new(Vec::new());
static DIAGNOSTICS_EMITTED: AtomicBool = AtomicBool::new(false);
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

pub fn safe_range(span: Span, src_len: usize) -> std::ops::Range<usize> {
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

pub fn emit_mapped_errors_and_terminate(
    filename: String,
    src: &'static str,
    errs: Vec<Rich<'_, String, Span>>,
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

fn emit_human_diagnostic(filename: &str, src: &str, e: &Rich<'_, String, Span>) {
    if let Some(mapped) = get_diagnostic_info(*e.span()) {
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

fn emit_json_diagnostic(filename: &str, src: &str, e: &Rich<'_, String, Span>) {
    if let Some(mapped) = get_diagnostic_info(*e.span()) {
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
    let (line_start, column_start) = byte_to_line_col(src, range.start);
    let (line_end, column_end) = byte_to_line_col(src, range.end);
    json!({
        "file_name": filename,
        "byte_start": range.start,
        "byte_end": range.end,
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
    json_span(filename, src, range, is_primary, label)
}

fn get_diagnostic_info(span: Span) -> Option<DiagnosticSpan> {
    let guard = SOURCE_MAP.try_lock().unwrap();
    let sm = guard.as_ref()?;
    let (file_name, source) = sm.get_file_info(span.context)?;
    Some(DiagnosticSpan {
        file_name,
        source,
        start: span.start,
        end: span.end,
    })
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
