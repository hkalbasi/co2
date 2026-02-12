use chumsky::span::SimpleSpan;

use ariadne::{Color, Label, Report, ReportKind, sources};
use chumsky::{Parser as _, input::Input as _};

use crate::{
    diagnostic::{RaiseErrorPanicPayload, report_error, take_errors},
    lexer::lexer,
    parser::translation_unit,
};

mod diagnostic;
mod exp;
pub mod hir;
mod lexer;
mod parser;
mod rust_ast;
pub mod type_ir;

pub use lexer::Token;
pub use parser::{
    CompoundStatement, Constant, Declaration, DeclarationSpecifier, Expression, InitDeclarator,
    LazyCompoundStatement, RustPath, RustPathSegment, Statement, StatementOrDeclaration,
};
pub use rust_ast::{Field, FnSig, Item, RustType, State};

// Type definitions
pub type Span = SimpleSpan<usize>;
pub type Spanned<T> = (T, Span);

pub fn parse_items(filename: String, src: &'static str) -> Option<State> {
    let (tokens, errs) = lexer().parse(src).into_output_errors();

    if let Some(tokens) = tokens {
        let tokens = tokens.leak();
        let (ast, parse_errs) = translation_unit()
            .map_with(|ast, e| (ast, e.span()))
            .parse(tokens.map((src.len()..src.len()).into(), |(t, s)| (t, s)))
            .into_output_errors();

        if parse_errs.is_empty() {
            if let Some(ast) = ast {
                let panic_payload = std::panic::catch_unwind(|| {
                    let mut state = State::default();
                    state.collect_translation_unit(ast.0.0);
                    state
                });
                match panic_payload {
                    Err(panic_payload) => {
                        if panic_payload
                            .downcast_ref::<RaiseErrorPanicPayload>()
                            .is_none()
                        {
                            std::panic::resume_unwind(panic_payload);
                        }
                    }
                    Ok(state) => return Some(state),
                }
            }
        } else {
            for err in parse_errs {
                report_error(err);
            }
        }
    }

    print_errors_and_terminate(filename, src, errs);
}

pub fn parse_compound_statement(
    tokens: &[Spanned<Token>],
    filename: String,
    src: &'static str,
) -> Option<Spanned<CompoundStatement>> {
    let (ast, parse_errs) = parser::compound_statement()
        .parse(tokens.map((src.len()..src.len()).into(), |(t, s)| (t, s)))
        .into_output_errors();

    if parse_errs.is_empty() {
        if let Some(ast) = ast {
            return Some(ast);
        }
    } else {
        for err in parse_errs {
            let e = err.map_token(|tok| tok.to_string());
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
                .print(sources([(filename.clone(), src.to_owned())]))
                .unwrap();
        }
        std::process::exit(5);
    }

    print_errors_and_terminate(filename, src, Vec::new());
}

pub fn print_errors_and_terminate(
    filename: String,
    src: &'static str,
    errs: Vec<chumsky::prelude::Rich<'_, char>>,
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
                .print(sources([(filename.clone(), src.to_owned())]))
                .unwrap()
        });
    std::process::exit(5);
}
