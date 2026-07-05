use chumsky::Parser as _;
use chumsky::input::{BorrowInput, ExactSizeInput, Input as ChumskyInput, SliceInput, ValueInput};

mod exp;
mod parser;

pub(crate) use co2_ast::{
    CompoundStatement, Expression, Token, TranslationUnit, TypeResolver, print_errors_and_terminate,
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
    let input = TokenSpanInput { tokens, end_span };
    let (ast, parse_errs) = parser::translation_unit(resolver)
        .map_with(|ast, e| (ast, e.span()))
        .parse(input)
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
    let input = TokenSpanInput { tokens, end_span };
    let (ast, parse_errs) =
        parser::compound_statement(resolver.clone(), parser::statement(resolver))
            .parse(input)
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
    let input = TokenSpanInput { tokens, end_span };
    let (ast, parse_errs) = expr_parser
        .then_ignore(chumsky::primitive::end())
        .parse(input)
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

// ── Custom chumsky input preserving per-token span contexts ─────────
//
// chumsky's MappedInput::span() always uses eoi.context() as the span
// context, discarding each token's own FileId. This causes parse errors
// in included headers to point to the main file instead of the header.
// TokenSpanInput fixes this by returning the token's original span directly.

struct TokenSpanInput<'src> {
    tokens: &'src [Spanned<Token>],
    end_span: Span,
}

type TokenSpanCache<'src> = (&'src [Spanned<Token>], Span);

impl<'src> ChumskyInput<'src> for TokenSpanInput<'src> {
    type Cursor = usize;
    type Span = Span;

    type Token = Token;
    type MaybeToken = &'src Token;

    type Cache = TokenSpanCache<'src>;

    fn begin(self) -> (Self::Cursor, Self::Cache) {
        (0, (self.tokens, self.end_span))
    }

    fn cursor_location(cursor: &Self::Cursor) -> usize {
        *cursor
    }

    unsafe fn next_maybe(
        (cache, _): &mut Self::Cache,
        cursor: &mut Self::Cursor,
    ) -> Option<Self::MaybeToken> {
        let (tok, _) = cache.get(*cursor)?;
        *cursor += 1;
        Some(tok)
    }

    unsafe fn span(
        (cache, end_span): &mut Self::Cache,
        range: core::ops::Range<&Self::Cursor>,
    ) -> Self::Span {
        let start = *range.start;
        let end = *range.end;
        if start < end && end > 0 {
            let first_span = &cache[start.min(cache.len().saturating_sub(1))].1;
            let last_span = &cache[(end - 1).min(cache.len().saturating_sub(1))].1;
            let ctx = first_span.data().context;
            let start_off = first_span.data().start;
            let end_off = last_span.data().end;
            Span::from_parts(ctx, start_off..end_off)
        } else if start < cache.len() {
            cache[start].1.clone()
        } else {
            end_span.clone()
        }
    }
}

impl<'src> ExactSizeInput<'src> for TokenSpanInput<'src> {
    unsafe fn span_from(
        (cache, end_span): &mut Self::Cache,
        range: core::ops::RangeFrom<&Self::Cursor>,
    ) -> Self::Span {
        let idx = *range.start;
        if idx < cache.len() {
            cache[idx].1.clone()
        } else {
            end_span.clone()
        }
    }
}

impl<'src> ValueInput<'src> for TokenSpanInput<'src> {
    unsafe fn next((cache, _): &mut Self::Cache, cursor: &mut Self::Cursor) -> Option<Self::Token> {
        let (tok, _) = cache.get(*cursor)?;
        *cursor += 1;
        Some(tok.clone())
    }
}

impl<'src> BorrowInput<'src> for TokenSpanInput<'src> {
    unsafe fn next_ref(
        (cache, _): &mut Self::Cache,
        cursor: &mut Self::Cursor,
    ) -> Option<&'src Self::Token> {
        let (tok, _) = cache.get(*cursor)?;
        *cursor += 1;
        Some(tok)
    }
}

impl<'src> SliceInput<'src> for TokenSpanInput<'src> {
    type Slice = &'src [Spanned<Token>];

    fn full_slice((cache, _): &mut Self::Cache) -> Self::Slice {
        *cache
    }

    unsafe fn slice(
        (cache, _): &mut Self::Cache,
        range: core::ops::Range<&Self::Cursor>,
    ) -> Self::Slice {
        &cache[*range.start..*range.end]
    }

    unsafe fn slice_from(
        (cache, _): &mut Self::Cache,
        from: core::ops::RangeFrom<&Self::Cursor>,
    ) -> Self::Slice {
        &cache[*from.start..]
    }
}
