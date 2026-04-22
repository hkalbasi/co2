use std::sync::Mutex;

use ariadne::{Color, Label, Report, ReportKind, sources};
use chumsky::{error::Rich, span::SimpleSpan};
use serde_json::json;

use crate::Token;

static ERRORS: Mutex<Vec<Rich<'static, Token, SimpleSpan>>> = Mutex::new(Vec::new());

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
    if std::env::var_os("CO2_FORCE_JSON_DIAGNOSTICS").is_some() {
        for e in errs {
            emit_json_diagnostic(&filename, src, &e);
        }
    } else {
        for e in errs {
            emit_human_diagnostic(&filename, src, &e);
        }
    }
    std::process::exit(5);
}

fn emit_human_diagnostic(filename: &str, src: &str, e: &Rich<'_, String, SimpleSpan>) {
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

fn diagnostic_byte_offset() -> isize {
    std::env::var("CO2_JSON_BYTE_OFFSET")
        .ok()
        .and_then(|raw| raw.parse::<isize>().ok())
        .unwrap_or(0)
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
