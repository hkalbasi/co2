use chumsky::{Parser as _, input::Input as _};

mod exp;
mod parser;

pub(crate) use co2_ast::{
    CompoundStatement, Expression, Token, TranslationUnit, TypeResolver,
    print_errors_and_terminate,
};

pub(crate) use co2_ast::{Span, Spanned};

fn eoi_span_for_tokens(tokens: &[Spanned<Token>], fallback: Span) -> Span {
    tokens.last().map_or(fallback, |(_, span)| {
        Span::from_parts(span.data().context, span.data().end..span.data().end)
    })
}

pub fn parse_translation_unit<R: TypeResolver>(
    filename: &str,
    preprocessed: &co2_preprocessor::PreprocessedSource,
    resolver: R,
) -> Option<Spanned<TranslationUnit<R>>> {
    let src: &'static str = Box::leak(preprocessed.raw_src.to_string().into_boxed_str());
    let end_span = preprocessed.tokens.last().map_or(
        Span::from_parts(preprocessed.main_file_idx, src.len()..src.len()),
        |(_, span)| Span::from_parts(span.data().context, span.data().end..span.data().end),
    );
    Some(parse_translation_unit_from_tokens(
        &preprocessed.tokens,
        filename,
        src,
        end_span,
        resolver,
    ))
}

/// Parse a translation unit from an already-tokenised slice.
/// Used for inline modules whose tokens were captured during parent-file parsing.
pub fn parse_translation_unit_from_tokens<R: TypeResolver>(
    tokens: &[Spanned<Token>],
    filename: &str,
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
        // Emit errors but don't terminate — inline modules need to continue
        // so that errors from all modules are reported together.
        co2_ast::emit_errors(
            parse_errs
                .into_iter()
                .map(|err| err.map_token(|tok| tok.to_string()))
                .collect(),
        );
        if let Some(ast) = ast {
            return ast.0;
        }
    }

    print_errors_and_terminate(filename, src, Vec::new());
}

pub fn parse_compound_statement<R: TypeResolver>(
    tokens: &[Spanned<Token>],
    filename: &str,
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
