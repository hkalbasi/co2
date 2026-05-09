use chumsky::{
    Parser as _,
    error::Rich,
    input::Input as _,
    span::{SimpleSpan, Span as _},
};

use crate::{lexer::lexer, parser::translation_unit};

mod exp;
mod lexer;
mod parser;

pub(crate) use co2_ast::*;

pub(crate) use co2_ast::{Span, Spanned};

fn map_lexer_span(
    span: SimpleSpan<usize>,
    pp: Option<&co2_preprocessor::PreprocessedSource>,
) -> Span {
    if let Some(pp) = pp {
        pp.real_span(span.start, span.end)
    } else {
        Span::new(FileId::INVALID, span.start..span.end)
    }
}

fn map_lexer_error<'src>(
    err: Rich<'src, char, SimpleSpan<usize>>,
    pp: Option<&co2_preprocessor::PreprocessedSource>,
) -> Rich<'src, char, Span> {
    Rich::custom(map_lexer_span(*err.span(), pp), err.to_string())
}

fn map_lexer_tokens(
    tokens: Vec<(Token, SimpleSpan<usize>)>,
    pp: Option<&co2_preprocessor::PreprocessedSource>,
) -> Vec<Spanned<Token>> {
    tokens
        .into_iter()
        .map(|(token, span)| (token, map_lexer_span(span, pp)))
        .collect()
}

fn eoi_span_for_tokens(tokens: &[Spanned<Token>], fallback: Span) -> Span {
    tokens.last().map_or(fallback, |(_, span)| {
        Span::new(span.context, span.end..span.end)
    })
}

pub fn parse_translation_unit<R: TypeResolver>(
    filename: String,
    src: &'static str,
    pp: Option<&co2_preprocessor::PreprocessedSource>,
    resolver: R,
) -> Option<Spanned<TranslationUnit<R>>> {
    parse_translation_unit_internal(filename, src, pp, resolver)
}

fn parse_translation_unit_internal<R: TypeResolver>(
    filename: String,
    src: &'static str,
    pp: Option<&co2_preprocessor::PreprocessedSource>,
    resolver: R,
) -> Option<Spanned<TranslationUnit<R>>> {
    let (tokens, errs) = lexer().parse(src).into_output_errors();

    if let Some(tokens) = tokens {
        let tokens = map_lexer_tokens(tokens, pp);
        let fallback_end_span = if let Some(pp) = pp {
            map_lexer_span(SimpleSpan::new((), src.len()..src.len()), Some(pp))
        } else {
            Span::new(FileId::INVALID, src.len()..src.len())
        };
        let end_span = eoi_span_for_tokens(&tokens, fallback_end_span);
        let tokens = tokens.leak();
        let (ast, parse_errs) = translation_unit(resolver)
            .map_with(|ast, e| (ast, e.span()))
            .parse(tokens.map(end_span, |(t, s)| (t, s)))
            .into_output_errors();

        if parse_errs.is_empty() {
            if let Some(ast) = ast {
                return Some(ast.0);
            }
        } else {
            co2_ast::emit_errors_and_terminate(
                parse_errs
                    .into_iter()
                    .map(|err| err.map_token(|tok| tok.to_string()))
                    .collect(),
            );
        }
    }

    print_errors_and_terminate(
        filename,
        src,
        errs.into_iter()
            .map(|err| map_lexer_error(err, pp))
            .collect(),
    );
}

pub fn parse_items(
    filename: String,
    src: &'static str,
    pp: Option<&co2_preprocessor::PreprocessedSource>,
) -> Option<Spanned<TranslationUnit<StatelessResolver>>> {
    parse_translation_unit(filename, src, pp, StatelessResolver::new())
}

/// Parse a translation unit from an already-tokenised slice.
/// Used for inline modules whose tokens were captured during parent-file parsing.
pub fn parse_translation_unit_from_tokens<R: TypeResolver>(
    tokens: &[Spanned<Token>],
    filename: String,
    src: &'static str,
    end_span: Span,
    resolver: R,
) -> Spanned<TranslationUnit<R>> {
    let end_span = eoi_span_for_tokens(tokens, end_span);
    let (ast, parse_errs) = parser::translation_unit(resolver)
        .map_with(|ast, e| (ast, e.span()))
        .parse(tokens.map(end_span, |(t, s)| (t, s)))
        .into_output_errors();

    if parse_errs.is_empty() {
        if let Some(ast) = ast {
            return ast.0;
        }
    } else {
        co2_ast::emit_errors_and_terminate(
            parse_errs
                .into_iter()
                .map(|err| err.map_token(|tok| tok.to_string()))
                .collect(),
        );
    }

    print_errors_and_terminate(filename, src, Vec::new());
}

pub fn parse_compound_statement<R: TypeResolver>(
    tokens: &[Spanned<Token>],
    filename: String,
    src: &'static str,
    end_span: Span,
    resolver: R,
) -> Spanned<CompoundStatement<R>> {
    let end_span = eoi_span_for_tokens(tokens, end_span);
    let (ast, parse_errs) =
        parser::compound_statement(resolver.clone(), parser::statement(resolver))
            .parse(tokens.map(end_span, |(t, s)| (t, s)))
            .into_output_errors();

    if parse_errs.is_empty() {
        if let Some(ast) = ast {
            return ast;
        }
    } else {
        co2_ast::emit_errors_and_terminate(
            parse_errs
                .into_iter()
                .map(|err| err.map_token(|tok| tok.to_string()))
                .collect(),
        );
    }

    print_errors_and_terminate(filename, src, Vec::new());
}

#[test]
fn include_body_lazy_span_uses_header_context() {
    use std::path::Path;

    let pp = co2_preprocessor::preprocess(
        Path::new("../tests/compiletest/ui/include_error.c"),
        &["-I".to_owned(), "../tests/compiletest/ui".to_owned()],
    );
    let src: &'static str = Box::leak(pp.normalized.to_string().into_boxed_str());
    let (tokens, lex_errs) = lexer().parse(src).into_output_errors();
    assert!(lex_errs.is_empty());
    let tokens = map_lexer_tokens(tokens.unwrap(), Some(&pp));
    let tokens = tokens.leak();
    let end_span = pp.real_span(src.len(), src.len());
    let (ast, parse_errs) = translation_unit(StatelessResolver::new())
        .map_with(|ast, e| (ast, e.span()))
        .parse(tokens.map(end_span, |(t, s)| (t, s)))
        .into_output_errors();
    assert!(parse_errs.is_empty());

    let tu = ast.unwrap().0.0;
    let body = match &tu.items[0].0 {
        Declaration::FunctionDefinition { body, .. } => &body.0.tokens,
        other => panic!("unexpected first item: {other:?}"),
    };

    let body_file = pp.files().get(&body.1.context).unwrap();
    assert_eq!(body_file.path.file_name().unwrap(), "include_error.h");
    assert_eq!(
        &body_file.source[body.1.start..body.1.end],
        "{\n    return missing;\n    //     ^^^^^^^ error: Unresolved name\n}",
    );
}

pub fn parse_expression_tokens<R: TypeResolver>(
    tokens: &[Spanned<Token>],
    end_span: Span,
    resolver: R,
) -> Spanned<Expression<R>> {
    let end_span = eoi_span_for_tokens(tokens, end_span);
    let stmt_parser = parser::statement(resolver.clone());
    let expr_parser = parser::expression(parser::assignment_expression(resolver, stmt_parser));
    let (ast, parse_errs) = expr_parser
        .then_ignore(chumsky::primitive::end())
        .parse(tokens.map(end_span, |(t, s)| (t, s)))
        .into_output_errors();

    if parse_errs.is_empty() {
        if let Some(ast) = ast {
            return ast;
        }
        panic!("No ast emitted but there was no error.");
    } else {
        co2_ast::emit_errors_and_terminate(
            parse_errs
                .into_iter()
                .map(|err| err.map_token(|tok| tok.to_string()))
                .collect(),
        );
    }
}
