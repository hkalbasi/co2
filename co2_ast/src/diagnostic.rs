use std::sync::Mutex;

use ariadne::{sources, Color, Label, Report, ReportKind};
use chumsky::{error::Rich, span::SimpleSpan};

use crate::Token;

static ERRORS: Mutex<Vec<Rich<'static, Token, SimpleSpan>>> = Mutex::new(Vec::new());

pub fn take_errors() -> Vec<Rich<'static, Token, SimpleSpan>> {
    let mut guard = ERRORS.try_lock().unwrap();
    std::mem::take(&mut *guard)
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
            Report::build(ReportKind::Error, (filename.clone(), e.span().into_range()))
                .with_config(ariadne::Config::new().with_index_type(ariadne::IndexType::Byte))
                .with_message(e.to_string())
                .with_label(
                    Label::new((filename.clone(), e.span().into_range()))
                        .with_message(e.reason().to_string())
                        .with_color(Color::Red),
                )
                .with_labels(e.contexts().map(|(label, span)| {
                    Label::new((filename.clone(), span.into_range()))
                        .with_message(format!("while parsing this {label}"))
                        .with_color(Color::Yellow)
                }))
                .finish()
                .eprint(sources([(filename.clone(), src.to_owned())]))
                .unwrap()
        });
    std::process::exit(5);
}
