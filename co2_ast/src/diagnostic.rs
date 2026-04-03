use std::sync::Mutex;

use ariadne::{Color, Label, Report, ReportKind, sources};
use chumsky::{error::Rich, span::SimpleSpan};

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
    errs.into_iter()
        .map(|e| e.map_token(|c| c.to_string()))
        .chain(
            take_errors()
                .into_iter()
                .map(|e| e.map_token(|tok| tok.to_string())),
        )
        .for_each(|e| {
            let range = safe_range(*e.span(), src.len());
            Report::build(ReportKind::Error, (filename.clone(), range.clone()))
                .with_config(ariadne::Config::new().with_index_type(ariadne::IndexType::Byte))
                .with_message(e.to_string())
                .with_label(
                    Label::new((filename.clone(), range))
                        .with_message(e.reason().to_string())
                        .with_color(Color::Red),
                )
                .with_labels(e.contexts().map(|(label, span)| {
                    Label::new((filename.clone(), safe_range(*span, src.len())))
                        .with_message(format!("while parsing this {label}"))
                        .with_color(Color::Yellow)
                }))
                .finish()
                .eprint(sources([(filename.clone(), src.to_owned())]))
                .unwrap()
        });
    std::process::exit(5);
}
