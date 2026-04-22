use chumsky::{Parser as _, input::Input as _};

use crate::{lexer::lexer, parser::translation_unit};

mod exp;
mod lexer;
mod parser;

pub(crate) use co2_ast::*;

pub(crate) use co2_ast::{Span, Spanned};

pub fn parse_translation_unit<R: TypeResolver>(
    filename: String,
    src: &'static str,
    resolver: R,
) -> Option<Spanned<TranslationUnit<R>>> {
    let (tokens, errs) = lexer().parse(src).into_output_errors();

    if let Some(tokens) = tokens {
        let tokens = tokens.leak();
        let (ast, parse_errs) = translation_unit(resolver)
            .map_with(|ast, e| (ast, e.span()))
            .parse(tokens.map((src.len()..src.len()).into(), |(t, s)| (t, s)))
            .into_output_errors();

        if parse_errs.is_empty() {
            if let Some(ast) = ast {
                return Some(ast.0);
            }
        } else {
            co2_ast::emit_mapped_errors_and_terminate(
                filename,
                src,
                parse_errs
                    .into_iter()
                    .map(|err| err.map_token(|tok| tok.to_string()))
                    .collect(),
            );
        }
    }

    print_errors_and_terminate(filename, src, errs);
}

pub fn parse_items(
    filename: String,
    src: &'static str,
) -> Option<Spanned<TranslationUnit<StatelessResolver>>> {
    parse_translation_unit(filename, src, StatelessResolver::new())
}

pub fn parse_compound_statement<R: TypeResolver>(
    tokens: &[Spanned<Token>],
    filename: String,
    src: &'static str,
    resolver: R,
) -> Spanned<CompoundStatement<R>> {
    let (ast, parse_errs) =
        parser::compound_statement(resolver.clone(), parser::statement(resolver))
            .parse(tokens.map((src.len()..src.len()).into(), |(t, s)| (t, s)))
            .into_output_errors();

    if parse_errs.is_empty() {
        if let Some(ast) = ast {
            return ast;
        }
    } else {
        co2_ast::emit_mapped_errors_and_terminate(
            filename,
            src,
            parse_errs
                .into_iter()
                .map(|err| err.map_token(|tok| tok.to_string()))
                .collect(),
        );
    }

    print_errors_and_terminate(filename, src, Vec::new());
}

pub fn parse_expression_tokens<R: TypeResolver>(
    tokens: &[Spanned<Token>],
    filename: String,
    src: &'static str,
    resolver: R,
) -> Spanned<Expression<R>> {
    let stmt_parser = parser::statement(resolver.clone());
    let expr_parser = parser::expression(parser::assignment_expression(resolver, stmt_parser));
    let (ast, parse_errs) = expr_parser
        .then_ignore(chumsky::primitive::end())
        .parse(tokens.map((src.len()..src.len()).into(), |(t, s)| (t, s)))
        .into_output_errors();

    if parse_errs.is_empty() {
        if let Some(ast) = ast {
            return ast;
        }
    } else {
        co2_ast::emit_mapped_errors_and_terminate(
            filename,
            src,
            parse_errs
                .into_iter()
                .map(|err| err.map_token(|tok| tok.to_string()))
                .collect(),
        );
    }

    print_errors_and_terminate(filename, src, Vec::new());
}
