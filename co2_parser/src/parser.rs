//! Hand-written recursive-descent C parser.
//!
//! Design (high-performance style, like clang/tcc):
//! - Borrowed token slice + integer cursor. Zero token clones; AST owns its data.
//! - First-token dispatch everywhere; speculation only where C is truly ambiguous
//!   (cast-vs-paren, declaration-vs-expression-statement, typename-vs-expr in
//!   `typeof`/`sizeof`), via cheap pos+resolver checkpoints. No error allocation
//!   on the happy path.
//! - Errors are cold: `Fail` carries (span, message); only the outermost entry
//!   turns it into a diagnostic. TU parsing uses panic-mode recovery (emit +
//!   skip to `;`/`}`) so all modules' errors are still reported together.

use co2_ast::TypeResolver;
use co2_ast::{
    CompoundStatement, Declaration, DeclarationSpecifier, Declarator, EnumSpecifier, Enumerator,
    Expression, FileId, ForInit, FunctionDefinitionSignature, FunctionSpecifier, InitDeclarator,
    LazyCompoundStatement, LazyRustConstExpr, LazySubscription, ModItem, ParameterList,
    RustAttribute, RustAttributeStyle, RustFunctionParam, RustFunctionSignature, RustPath,
    RustPathSegment, RustStructField, RustTy, Span, Spanned, SpecifierQualifier, StatelessResolver,
    Statement, StatementOrDeclaration, StorageClassSpecifier, StringLiteral, StringLiteralPrefix,
    StructDeclarator, StructOrUnionField, StructOrUnionKind, StructOrUnionSpecifier, Token,
    TranslationUnit, TypeName, TypeQualifier, TypeQueryResult, TypeSpecifier, UseItem, Visibility,
};

// ── Errors (cold path only) ────────────────────────────────────────────

/// A parse failure. Carries no backtrace and allocates only the message string.
pub(crate) struct Fail {
    pub span: Span,
    pub msg: String,
}

pub(crate) type PR<T> = Result<T, Fail>;

// ── Cursor ─────────────────────────────────────────────────────────────

pub(crate) struct P<'a, R> {
    pub toks: &'a [Spanned<Token>],
    pub pos: usize,
    pub end_span: Span,
    pub resolver: R,
}

impl<'a, R: TypeResolver> P<'a, R> {
    pub fn new(toks: &'a [Spanned<Token>], end_span: Span, resolver: R) -> Self {
        Self {
            toks,
            pos: 0,
            end_span,
            resolver,
        }
    }

    #[inline]
    pub fn peek(&self, off: usize) -> Option<&'a Token> {
        self.toks.get(self.pos + off).map(|(t, _)| t)
    }

    #[inline]
    pub fn peek_span(&self, off: usize) -> Span {
        self.toks
            .get(self.pos + off)
            .map_or(self.end_span, |(_, s)| *s)
    }

    #[inline]
    pub fn cur_span(&self) -> Span {
        self.peek_span(0)
    }

    #[inline]
    pub fn at(&self, t: &Token) -> bool {
        self.peek(0) == Some(t)
    }

    /// Consume the current token if it equals `t`.
    #[inline]
    pub fn eat(&mut self, t: &Token) -> Option<Span> {
        if self.at(t) {
            let s = self.peek_span(0);
            self.pos += 1;
            Some(s)
        } else {
            None
        }
    }

    pub fn expect(&mut self, t: &Token, what: &str) -> PR<Span> {
        if self.at(t) {
            let s = self.peek_span(0);
            self.pos += 1;
            Ok(s)
        } else {
            Err(self.fail_here(format!("expected {what}, found {}", self.describe())))
        }
    }

    pub fn describe(&self) -> String {
        match self.peek(0) {
            Some(t) => t.to_string(),
            None => "end of input".to_string(),
        }
    }

    pub fn fail_here(&self, msg: String) -> Fail {
        Fail {
            span: self.cur_span(),
            msg,
        }
    }

    pub fn fail_at(&self, span: Span, msg: String) -> Fail {
        Fail { span, msg }
    }

    /// Span covering tokens [start, pos). Zero-width at cursor if empty.
    pub fn span_since(&self, start: usize) -> Span {
        if start < self.pos && !self.toks.is_empty() {
            let s = self.toks[start.min(self.toks.len() - 1)].1;
            let e = self.toks[(self.pos - 1).min(self.toks.len() - 1)].1;
            join_spans(s, e)
        } else {
            self.cur_span()
        }
    }

    pub fn checkpoint(&self) -> (usize, R) {
        (self.pos, self.resolver.clone())
    }

    pub fn restore(&mut self, cp: (usize, R)) {
        self.pos = cp.0;
        self.resolver = cp.1;
    }

    fn take_ident(&mut self, want: Option<&str>) -> Option<(String, Span)> {
        match self.peek(0) {
            Some(Token::Ident(s)) if want.is_none_or(|w| s == w) => {
                let span = self.peek_span(0);
                let s = s.clone();
                self.pos += 1;
                Some((s, span))
            }
            _ => None,
        }
    }

    /// Consume `Ident(name)` exactly.
    pub fn eat_ident(&mut self, name: &str) -> Option<(String, Span)> {
        self.take_ident(Some(name))
    }

    /// Consume any identifier.
    pub fn any_ident(&mut self) -> PR<(String, Span)> {
        self.take_ident(None)
            .ok_or_else(|| self.fail_here(format!("expected identifier, found {}", self.describe())))
    }

    // Capture tokens inside a balanced `open ... close` pair, assuming the
    // cursor is ON `open`. Consumes through the matching `close`, handles
    // nesting. Returns (inner tokens, whole span incl. delimiters).
    pub fn capture_balanced(
        &mut self,
        open: &Token,
        close: &Token,
    ) -> PR<(Vec<Spanned<Token>>, Span)> {
        let start = self.pos;
        let whole = self.capture_balanced_full(open, close)?;
        let span = self.span_since(start);
        let inner = if whole.len() >= 2 {
            whole[1..whole.len() - 1].to_vec()
        } else {
            Vec::new()
        };
        Ok((inner, span))
    }

    /// Capture the full slice (including delimiters) of a balanced pair.
    pub fn capture_balanced_full(
        &mut self,
        open: &Token,
        close: &Token,
    ) -> PR<Vec<Spanned<Token>>> {
        let start = self.pos;
        self.expect(open, &open.to_string())?;
        let mut depth = 1u32;
        while depth > 0 {
            match self.peek(0) {
                None => {
                    return Err(self.fail_here(format!(
                        "expected {}, found end of input",
                        close.to_string()
                    )));
                }
                Some(t) => {
                    if t == open {
                        depth += 1;
                    } else if t == close {
                        depth -= 1;
                    }
                    self.pos += 1;
                }
            }
        }
        Ok(self.toks[start..self.pos].to_vec())
    }

    /// Capture tokens up to (excluding) `close`. No nesting. Leaves `close`.
    pub fn capture_until(&mut self, close: &Token) -> Vec<Spanned<Token>> {
        let start = self.pos;
        while let Some(t) = self.peek(0) {
            if t == close {
                break;
            }
            self.pos += 1;
        }
        self.toks[start..self.pos].to_vec()
    }
}

/// `( item (, item)* [,] )` with optional trailing comma.
pub(crate) fn parse_comma_list<'a, R: TypeResolver, T>(
    p: &mut P<'a, R>,
    open: &Token,
    close: &Token,
    allow_trailing: bool,
    mut item: impl FnMut(&mut P<'a, R>) -> PR<T>,
) -> PR<Vec<T>> {
    p.expect(open, &open.to_string())?;
    let mut out = Vec::new();
    if !p.at(close) {
        loop {
            out.push(item(p)?);
            if p.eat(&Token::Comma).is_none() {
                break;
            }
            if allow_trailing && p.at(close) {
                break;
            }
        }
    }
    p.expect(close, &close.to_string())?;
    Ok(out)
}

// ── Shared pure helpers (unchanged semantics) ──────────────────────────

pub(crate) fn join_spans(start: Span, end: Span) -> Span {
    let start_data = start.data();
    let end_data = end.data();
    if start_data.context == end_data.context {
        Span::from_parts(start_data.context, start_data.start..end_data.end)
    } else {
        start
    }
}

pub(crate) fn merge_string_literals(parts: Vec<StringLiteral>, span: Span) -> StringLiteral {
    let mut target: Option<StringLiteralPrefix> = None;
    for part in &parts {
        let prefix = part.prefix();
        if prefix != StringLiteralPrefix::None {
            match target {
                None => target = Some(prefix),
                Some(t) if t != prefix => {
                    co2_ast::emit_errors(vec![co2_ast::Rich::custom(
                        span,
                        "unsupported concatenation of string literals with different encoding prefixes",
                    )]);
                }
                Some(_) => {}
            }
        }
    }
    let prefix = target.unwrap_or(StringLiteralPrefix::None);
    if prefix.is_wide() {
        let mut code_units = Vec::new();
        for part in parts {
            match part {
                StringLiteral::Utf16(units)
                | StringLiteral::Utf32(units)
                | StringLiteral::Wide(units) => {
                    code_units.extend(units);
                }
                StringLiteral::None(bytes)
                | StringLiteral::Str(bytes)
                | StringLiteral::Utf8(bytes) => {
                    for ch in String::from_utf8_lossy(&bytes).chars() {
                        let cp = ch as u32;
                        if prefix == StringLiteralPrefix::Utf16 && cp > 0xFFFF {
                            let base = cp - 0x10000;
                            code_units.push(0xD800 | (base >> 10));
                            code_units.push(0xDC00 | (base & 0x3FF));
                        } else {
                            code_units.push(cp);
                        }
                    }
                }
            }
        }
        match prefix {
            StringLiteralPrefix::Utf16 => StringLiteral::Utf16(code_units),
            StringLiteralPrefix::Utf32 => StringLiteral::Utf32(code_units),
            _ => StringLiteral::Wide(code_units),
        }
    } else {
        let mut bytes = Vec::new();
        for part in parts {
            match part {
                StringLiteral::None(b) | StringLiteral::Str(b) | StringLiteral::Utf8(b) => {
                    bytes.extend_from_slice(&b);
                }
                StringLiteral::Utf16(_) | StringLiteral::Utf32(_) | StringLiteral::Wide(_) => {
                    unreachable!("wide part with narrow target prefix")
                }
            }
        }
        match prefix {
            StringLiteralPrefix::Str => StringLiteral::Str(bytes),
            StringLiteralPrefix::Utf8 => StringLiteral::Utf8(bytes),
            _ => StringLiteral::None(bytes),
        }
    }
}

pub(crate) fn slice_span<T>(slice: &[(T, Span)], fallback: Span) -> Span {
    slice
        .first()
        .zip(slice.last())
        .map_or(fallback, |(first, last)| join_spans(first.1, last.1))
}

/// C type-specifier and qualifier keywords that are not valid in Rust type
/// positions. Returns the keyword text if `token` is one of them.
fn c_type_keyword_token_str(token: &Token) -> Option<&'static str> {
    Some(match token {
        Token::Bool => "_Bool",
        Token::Char => "char",
        Token::Const => "const",
        Token::Double => "double",
        Token::Float => "float",
        Token::Int => "int",
        Token::Long => "long",
        Token::Restrict => "restrict",
        Token::Short => "short",
        Token::Signed => "signed",
        Token::Unsigned => "unsigned",
        Token::Void => "void",
        Token::Volatile => "volatile",
        Token::Atomic => "_Atomic",
        _ => return None,
    })
}

/// Compute the Rust type suggestion for a sequence of C type keywords.
/// Returns `(rust_primitive, ffi_path_suffix)`.
fn c_type_keywords_suggestion(words: &[&str]) -> Option<(&'static str, &'static str)> {
    let types = words
        .iter()
        .copied()
        .filter(|word| !matches!(*word, "const" | "volatile" | "restrict" | "_Atomic"))
        .collect::<Vec<_>>();
    match types.as_slice() {
        ["void"] => Some(("()", "c_void")),
        ["_Bool"] => Some(("bool", "c_bool")),
        ["char"] => Some(("u8", "c_char")),
        ["signed", "char"] => Some(("i8", "c_schar")),
        ["unsigned", "char"] => Some(("u8", "c_uchar")),
        ["short"] | ["short", "int"] | ["signed", "short"] | ["signed", "short", "int"] => {
            Some(("i16", "c_short"))
        }
        ["unsigned", "short"] | ["unsigned", "short", "int"] => Some(("u16", "c_ushort")),
        ["int"] | ["signed"] | ["signed", "int"] => Some(("i32", "c_int")),
        ["unsigned"] | ["unsigned", "int"] => Some(("u32", "c_uint")),
        ["long"] | ["long", "int"] | ["signed", "long"] | ["signed", "long", "int"] => {
            Some(("i64", "c_long"))
        }
        ["unsigned", "long"] | ["unsigned", "long", "int"] => Some(("u64", "c_ulong")),
        ["long", "long"]
        | ["long", "long", "int"]
        | ["signed", "long", "long"]
        | ["signed", "long", "long", "int"] => Some(("i64", "c_longlong")),
        ["unsigned", "long", "long"] | ["unsigned", "long", "long", "int"] => {
            Some(("u64", "c_ulonglong"))
        }
        ["float"] => Some(("f32", "c_float")),
        ["double"] => Some(("f64", "c_double")),
        ["long", "double"] => Some(("f64", "c_longdouble")),
        _ => None,
    }
}

fn keyword_token_str(token: &Token) -> Option<&'static str> {
    Some(match token {
        Token::Auto => "auto",
        Token::Bool => "_Bool",
        Token::Break => "break",
        Token::Case => "case",
        Token::Char => "char",
        Token::Const => "const",
        Token::Constexpr => "constexpr",
        Token::Continue => "continue",
        Token::Default => "default",
        Token::Do => "do",
        Token::Double => "double",
        Token::Else => "else",
        Token::Enum => "enum",
        Token::Extern => "extern",
        Token::Float => "float",
        Token::For => "for",
        Token::Goto => "goto",
        Token::If => "if",
        Token::Inline => "inline",
        Token::Int => "int",
        Token::Long => "long",
        Token::Register => "register",
        Token::Restrict => "restrict",
        Token::Return => "return",
        Token::Short => "short",
        Token::Signed => "signed",
        Token::Sizeof => "sizeof",
        Token::Typeof => "typeof",
        Token::Alignof => "_Alignof",
        Token::Offsetof => "offsetof",
        Token::Static => "static",
        Token::Atomic => "_Atomic",
        Token::Struct => "struct",
        Token::Switch => "switch",
        Token::Typedef => "typedef",
        Token::Union => "union",
        Token::Unsigned => "unsigned",
        Token::Void => "void",
        Token::Volatile => "volatile",
        Token::While => "while",
        Token::Generic => "_Generic",
        Token::StaticAssert => "static_assert",
        Token::VaStart => "va_start",
        Token::VaArg => "va_arg",
        Token::VaCopy => "va_copy",
        Token::VaEnd => "va_end",
        Token::BuiltinInf => "__builtin_inf",
        Token::BuiltinNan => "__builtin_nan",
        Token::BuiltinConstantP => "__builtin_constant_p",
        Token::BuiltinTypesCompatibleP => "__builtin_types_compatible_p",
        _ => return None,
    })
}

fn parse_rust_attr(
    tokens: Vec<Spanned<Token>>,
    span: Span,
) -> Result<RustAttribute, (Span, String)> {
    let mut idx = 0;
    let mut path = Vec::new();
    while let Some((token, token_span)) = tokens.get(idx) {
        let segment = match token {
            Token::Ident(s) => s.clone(),
            other => match keyword_token_str(other) {
                Some(s) => s.to_string(),
                None => break,
            },
        };
        path.push((segment, *token_span));
        idx += 1;
        if !matches!(tokens.get(idx), Some((Token::ColonColon, _))) {
            break;
        }
        idx += 1;
    }
    if path.is_empty() {
        return Err((
            span,
            "attribute path must start with an identifier".to_string(),
        ));
    }
    Ok(RustAttribute {
        path,
        args: tokens[idx..].to_vec(),
        style: RustAttributeStyle::Outer,
    })
}

pub(crate) fn rust_path_span<R: TypeResolver>(path: &RustPath<R>, fallback: Span) -> Span {
    slice_span(&path.segments, fallback)
}

pub(crate) fn parse_hex_float_constant(text: &str) -> Option<f64> {
    let (significand, exponent) = text.split_once(['p', 'P'])?;
    let exponent = exponent.parse::<i32>().ok()?;
    let significand = significand
        .strip_prefix("0x")
        .or_else(|| significand.strip_prefix("0X"))?;
    let (int_part, frac_part) = significand.split_once('.').unwrap_or((significand, ""));
    if int_part.is_empty() && frac_part.is_empty() {
        return None;
    }

    let mut value = 0.0f64;
    for ch in int_part.chars() {
        let digit = ch.to_digit(16)?;
        value = value * 16.0 + f64::from(digit);
    }
    let mut scale = 1.0f64 / 16.0;
    for ch in frac_part.chars() {
        let digit = ch.to_digit(16)?;
        value += f64::from(digit) * scale;
        scale /= 16.0;
    }
    Some(value * 2.0f64.powi(exponent))
}

// ── Attributes ─────────────────────────────────────────────────────────

impl<'a, R: TypeResolver> P<'a, R> {
    /// Parse `#[...]` / `#![...]` / doc comments (zero or more).
    pub fn parse_attr_list(&mut self) -> PR<Vec<Spanned<RustAttribute>>> {
        let mut attrs = Vec::new();
        loop {
            if self.at(&Token::Hash) {
                self.pos += 1; // `#`
                let mut style = RustAttributeStyle::Outer;
                if self.at(&Token::Bang) {
                    self.pos += 1;
                    style = RustAttributeStyle::Inner;
                }
                if !self.at(&Token::LBracket) {
                    return Err(self.fail_here(format!("expected [, found {}", self.describe())));
                }
                let (inner, span) = self.capture_balanced(&Token::LBracket, &Token::RBracket)?;
                match parse_rust_attr(inner, span) {
                    Ok(mut attr) => {
                        attr.style = style;
                        attrs.push((attr, span));
                    }
                    Err((span, msg)) => return Err(self.fail_at(span, msg)),
                }
            } else if let Some((Token::DocComment { inner, text }, span)) = self.peek_doc() {
                let span = span;
                let inner = *inner;
                let text = text.clone();
                let attr = RustAttribute {
                    path: vec![("doc".to_owned(), span)],
                    args: vec![(
                        Token::StringLit(StringLiteral::None(text.into_bytes())),
                        span,
                    )],
                    style: if inner {
                        RustAttributeStyle::Inner
                    } else {
                        RustAttributeStyle::Outer
                    },
                };
                self.pos += 1;
                attrs.push((attr, span));
            } else {
                break;
            }
        }
        Ok(attrs)
    }

    fn peek_doc(&self) -> Option<(&Token, Span)> {
        match self.toks.get(self.pos) {
            Some((t @ Token::DocComment { .. }, s)) => Some((t, *s)),
            _ => None,
        }
    }
}

// ── Rust paths & types ───────────────────────────────────────────────

impl<'a, R: TypeResolver> P<'a, R> {
    pub(crate) fn parse_identifier(&mut self) -> PR<Spanned<String>> {
        self.any_ident()
    }

    /// Parse a `::`-separated path. With `bare_generics`, `ident<...>` segments
    /// are accepted; otherwise generics are only allowed as standalone
    /// `::<...>` (turbofish) segments.
    pub(crate) fn parse_rust_path(
        &mut self,
        bare_generics: bool,
    ) -> PR<Spanned<RustPath<StatelessResolver>>> {
        let start = self.pos;
        if self.at(&Token::ColonColon) {
            self.pos += 1;
        }
        let mut segments: Vec<Spanned<RustPathSegment<StatelessResolver>>> = Vec::new();
        // First segment: ident, or a leading `<...>` generics segment
        // (e.g. `::<T>` type specifiers).
        if self.at(&Token::Lt) {
            let (seg, span) = self.parse_generics_segment()?;
            segments.push((seg, span));
        } else {
            let (name, span) = self.any_ident()?;
            segments.push((RustPathSegment::Ident(name), span));
            if bare_generics && self.at(&Token::Lt) {
                if let Some((seg, span)) = self.try_parse_generics_segment() {
                    segments.push((seg, span));
                }
            }
        }
        while self.at(&Token::ColonColon) {
            self.pos += 1;
            if self.at(&Token::Lt) {
                let (seg, span) = self.parse_generics_segment()?;
                segments.push((seg, span));
            } else {
                let (name, span) = self.any_ident()?;
                segments.push((RustPathSegment::Ident(name), span));
                if bare_generics && self.at(&Token::Lt) {
                    if let Some((seg, span)) = self.try_parse_generics_segment() {
                        segments.push((seg, span));
                    }
                }
            }
        }
        if segments.is_empty() {
            return Err(self.fail_here("expected path".to_string()));
        }
        let span = self.span_since(start);
        Ok((RustPath { segments }, span))
    }

    fn parse_generics_segment(&mut self) -> PR<(RustPathSegment<StatelessResolver>, Span)> {
        let start = self.pos;
        let args = parse_comma_list(
            self,
            &Token::Lt,
            &Token::Gt,
            true,
            parse_rust_ty_stateless_inner,
        )?;
        let span = self.span_since(start);
        Ok((RustPathSegment::Generics(args), span))
    }

    fn try_parse_generics_segment(&mut self) -> Option<(RustPathSegment<StatelessResolver>, Span)> {
        let cp = self.checkpoint();
        match self.parse_generics_segment() {
            Ok(v) => Some(v),
            Err(_) => {
                self.restore(cp);
                None
            }
        }
    }

    fn parse_pub_token(&mut self) -> PR<(Visibility, Option<Span>)> {
        if self.eat_ident("pub").is_none() {
            return Ok((Visibility::Private, None));
        }
        if self.eat(&Token::LParen).is_some() {
            if self.eat_ident("crate").is_some() {
                self.expect(&Token::RParen, ")")?;
                return Ok((Visibility::Crate, None));
            }
            if self.eat_ident("self").is_some() {
                self.expect(&Token::RParen, ")")?;
                return Ok((Visibility::Restricted, None));
            }
            // `pub(<inner>)`: invalid specifier, capture inner span.
            let inner_start = self.pos;
            while !self.at(&Token::RParen) {
                if self.peek(0).is_none() {
                    return Err(self.fail_here("expected ), found end of input".to_string()));
                }
                self.pos += 1;
            }
            let inner = slice_span(&self.toks[inner_start..self.pos], self.cur_span());
            self.expect(&Token::RParen, ")")?;
            return Ok((Visibility::Public, Some(inner)));
        }
        Ok((Visibility::Public, None))
    }

    /// Resolver-classified Rust type for `R` positions.
    pub fn parse_rust_ty(&mut self) -> PR<Spanned<RustTy<R>>> {
        parse_rust_ty_resolving(self)
    }
}

// Concrete Rust-type parsing lives in free functions so the stateless and
// resolving variants stay independent.
fn parse_ptr_mut<'a, R: TypeResolver>(p: &mut P<'a, R>) -> PR<bool> {
    if p.at(&Token::Const) {
        p.pos += 1;
        Ok(false)
    } else if p.eat_ident("mut").is_some() {
        Ok(true)
    } else {
        Err(p.fail_here(format!("expected const or mut, found {}", p.describe())))
    }
}

fn parse_opt_lifetime<'a, R: TypeResolver>(p: &mut P<'a, R>) -> Option<(String, Span)> {
    match p.peek(0) {
        Some(Token::Lifetime(name)) => {
            let span = p.peek_span(0);
            let name = name.clone();
            p.pos += 1;
            Some((name, span))
        }
        _ => None,
    }
}

fn parse_array_len<'a, R: TypeResolver>(
    p: &mut P<'a, R>,
) -> Option<(LazyRustConstExpr, Span)> {
    let semi_span = p.eat(&Token::Semicolon)?;
    let tokens = p.capture_until(&Token::RBracket);
    // Span covers from `;` (old `map_with` on the `;...` match).
    let span = if tokens.is_empty() {
        semi_span
    } else {
        join_spans(semi_span, slice_span(&tokens, p.cur_span()))
    };
    Some((LazyRustConstExpr { tokens }, span))
}

pub(crate) fn parse_rust_ty_resolving<'a, R: TypeResolver>(
    p: &mut P<'a, R>,
) -> PR<Spanned<RustTy<R>>> {
    let start = p.pos;
    match p.peek(0) {
        Some(Token::Star) => {
            p.pos += 1;
            let mutable = parse_ptr_mut(p)?;
            let inner = parse_rust_ty_resolving(p)?;
            let span = p.span_since(start);
            Ok((
                RustTy::Ptr {
                    mutable,
                    inner: Box::new(inner),
                },
                span,
            ))
        }
        Some(Token::Amp) => {
            p.pos += 1;
            let lifetime = parse_opt_lifetime(p);
            let mutable = p.eat_ident("mut").is_some();
            let inner = parse_rust_ty_resolving(p)?;
            let span = p.span_since(start);
            Ok((
                RustTy::Ref {
                    lifetime,
                    mutable,
                    inner: Box::new(inner),
                },
                span,
            ))
        }
        Some(Token::Bang) => {
            p.pos += 1;
            Ok((RustTy::Never, p.span_since(start)))
        }
        Some(Token::LParen) => {
            let elems = parse_comma_list(
                p,
                &Token::LParen,
                &Token::RParen,
                true,
                parse_rust_ty_resolving,
            )?;
            Ok((RustTy::Tuple(elems), p.span_since(start)))
        }
        Some(Token::LBracket) => {
            p.pos += 1;
            let inner = parse_rust_ty_resolving(p)?;
            let len = parse_array_len(p);
            p.expect(&Token::RBracket, "]")?;
            let span = p.span_since(start);
            match len {
                Some(len) => Ok((
                    RustTy::Array {
                        inner: Box::new(inner),
                        len,
                    },
                    span,
                )),
                None => Ok((RustTy::Slice(Box::new(inner)), span)),
            }
        }
        Some(Token::Bool) | Some(Token::Char) => {
            let (name, span) = match p.peek(0) {
                Some(Token::Bool) => ("bool", p.peek_span(0)),
                _ => ("char", p.peek_span(0)),
            };
            p.pos += 1;
            let path = RustPath {
                segments: vec![(RustPathSegment::Ident(name.to_string()), span)],
            };
            match p.resolver.classify_path(&path) {
                Ok((TypeQueryResult::Type | TypeQueryResult::Unsure, resolved)) => {
                    Ok((RustTy::Path((resolved, span)), span))
                }
                Ok((TypeQueryResult::Expr, _)) => {
                    Err(p.fail_at(span, format!("{name} is not a type")))
                }
                Err((msg, err_span)) => Err(p.fail_at(err_span, msg)),
            }
        }
        Some(t) if c_type_keyword_token_str(t).is_some() => {
            let mut words = Vec::new();
            while let Some(w) = p.peek(0).and_then(c_type_keyword_token_str) {
                words.push(w);
                p.pos += 1;
            }
            let span = p.span_since(start);
            let c_type = words.join(" ");
            let msg = match c_type_keywords_suggestion(&words) {
                Some((primitive, ffi)) => format!(
                    "{c_type} is invalid as Rust type. Use either `{primitive}` or `std::ffi::{ffi}`"
                ),
                None => format!("{c_type} is invalid as Rust type"),
            };
            co2_ast::emit_errors(vec![co2_ast::Rich::custom(span, msg)]);
            Ok((RustTy::Tuple(vec![]), span))
        }
        _ => {
            let (path, _) = p.parse_rust_path(true)?;
            let span = p.span_since(start);
            let path_span = rust_path_span(&path, span);
            match p.resolver.classify_path(&path) {
                Ok((TypeQueryResult::Unsure | TypeQueryResult::Type, resolved)) => {
                    Ok((RustTy::Path((resolved, path_span)), span))
                }
                Ok((TypeQueryResult::Expr, _)) => {
                    Err(p.fail_at(path_span, "expected type, found expression".to_string()))
                }
                Err((msg, err_span)) => Err(p.fail_at(err_span, msg)),
            }
        }
    }
}

pub(crate) fn parse_rust_ty_stateless_inner<'a, R: TypeResolver>(
    p: &mut P<'a, R>,
) -> PR<Spanned<RustTy<StatelessResolver>>> {
    let start = p.pos;
    match p.peek(0) {
        Some(Token::Ident(s)) if s == "_" => {
            p.pos += 1;
            Ok((RustTy::Wild, p.span_since(start)))
        }
        Some(Token::Star) => {
            p.pos += 1;
            let mutable = parse_ptr_mut(p)?;
            let inner = parse_rust_ty_stateless_inner(p)?;
            let span = p.span_since(start);
            Ok((
                RustTy::Ptr {
                    mutable,
                    inner: Box::new(inner),
                },
                span,
            ))
        }
        Some(Token::Amp) => {
            p.pos += 1;
            let lifetime = parse_opt_lifetime(p);
            let mutable = p.eat_ident("mut").is_some();
            let inner = parse_rust_ty_stateless_inner(p)?;
            let span = p.span_since(start);
            Ok((
                RustTy::Ref {
                    lifetime,
                    mutable,
                    inner: Box::new(inner),
                },
                span,
            ))
        }
        Some(Token::Bang) => {
            p.pos += 1;
            Ok((RustTy::Never, p.span_since(start)))
        }
        Some(Token::LParen) => {
            let elems = parse_comma_list(
                p,
                &Token::LParen,
                &Token::RParen,
                true,
                parse_rust_ty_stateless_inner,
            )?;
            Ok((RustTy::Tuple(elems), p.span_since(start)))
        }
        Some(Token::LBracket) => {
            p.pos += 1;
            let inner = parse_rust_ty_stateless_inner(p)?;
            let len = parse_array_len(p);
            p.expect(&Token::RBracket, "]")?;
            let span = p.span_since(start);
            match len {
                Some(len) => Ok((
                    RustTy::Array {
                        inner: Box::new(inner),
                        len,
                    },
                    span,
                )),
                None => Ok((RustTy::Slice(Box::new(inner)), span)),
            }
        }
        Some(Token::Lifetime(_)) => {
            let (name, span) = match p.peek(0) {
                Some(Token::Lifetime(name)) => (name.clone(), p.peek_span(0)),
                _ => unreachable!(),
            };
            p.pos += 1;
            Ok((RustTy::Lifetime((name, span)), span))
        }
        Some(Token::Bool) | Some(Token::Char) => {
            let (name, span) = match p.peek(0) {
                Some(Token::Bool) => ("bool", p.peek_span(0)),
                _ => ("char", p.peek_span(0)),
            };
            p.pos += 1;
            let path = RustPath {
                segments: vec![(RustPathSegment::Ident(name.to_string()), span)],
            };
            match StatelessResolver::new().classify_path(&path) {
                Ok((_, resolved)) => Ok((RustTy::Path((resolved, span)), span)),
                Err((msg, err_span)) => Err(p.fail_at(err_span, msg)),
            }
        }
        Some(t) if c_type_keyword_token_str(t).is_some() => {
            let mut words = Vec::new();
            while let Some(w) = p.peek(0).and_then(c_type_keyword_token_str) {
                words.push(w);
                p.pos += 1;
            }
            let span = p.span_since(start);
            let c_type = words.join(" ");
            let msg = match c_type_keywords_suggestion(&words) {
                Some((primitive, ffi)) => format!(
                    "{c_type} is invalid as Rust type. Use either `{primitive}` or `std::ffi::{ffi}`"
                ),
                None => format!("{c_type} is invalid as Rust type"),
            };
            co2_ast::emit_errors(vec![co2_ast::Rich::custom(span, msg)]);
            Ok((RustTy::Tuple(vec![]), span))
        }
        _ => {
            let (path, _) = p.parse_rust_path(true)?;
            let span = p.span_since(start);
            Ok((RustTy::Path((path, span)), span))
        }
    }
}

// ── C types & declarators ────────────────────────────────────────────

fn is_type_qualifier(tok: Option<&Token>) -> bool {
    matches!(
        tok,
        Some(Token::Const | Token::Restrict | Token::Volatile | Token::Atomic)
    )
}

impl<'a, R: TypeResolver> P<'a, R> {
    fn parse_type_qualifier(&mut self) -> PR<Spanned<TypeQualifier>> {
        let start = self.pos;
        let q = match self.peek(0) {
            Some(Token::Const) => TypeQualifier::Const,
            Some(Token::Restrict) => TypeQualifier::Restrict,
            Some(Token::Volatile) => TypeQualifier::Volatile,
            Some(Token::Atomic) => TypeQualifier::Atomic,
            _ => {
                return Err(self.fail_here(format!(
                    "expected type qualifier, found {}",
                    self.describe()
                )));
            }
        };
        self.pos += 1;
        Ok((q, self.span_since(start)))
    }

    fn parse_storage_class(&mut self) -> PR<Spanned<StorageClassSpecifier>> {
        let start = self.pos;
        let s = match self.peek(0) {
            Some(Token::Typedef) => StorageClassSpecifier::Typedef,
            Some(Token::Extern) => StorageClassSpecifier::Extern,
            Some(Token::Static) => StorageClassSpecifier::Static,
            Some(Token::Constexpr) => StorageClassSpecifier::Constexpr,
            Some(Token::Atomic) => StorageClassSpecifier::Atomic,
            Some(Token::ThreadLocal) => StorageClassSpecifier::ThreadLocal,
            Some(Token::Auto) => StorageClassSpecifier::Auto,
            Some(Token::Register) => StorageClassSpecifier::Register,
            _ => {
                return Err(self.fail_here(format!(
                    "expected storage specifier, found {}",
                    self.describe()
                )));
            }
        };
        self.pos += 1;
        Ok((s, self.span_since(start)))
    }

    pub fn parse_type_specifier(&mut self) -> PR<Spanned<TypeSpecifier<R>>> {
        let start = self.pos;
        let spec = match self.peek(0) {
            Some(Token::Int) => TypeSpecifier::Int,
            Some(Token::Bool) => TypeSpecifier::Bool,
            Some(Token::Void) => TypeSpecifier::Void,
            Some(Token::Char) => TypeSpecifier::Char,
            Some(Token::Short) => TypeSpecifier::Short,
            Some(Token::Long) => TypeSpecifier::Long,
            Some(Token::Float) => TypeSpecifier::Float,
            Some(Token::Double) => TypeSpecifier::Double,
            Some(Token::Signed) => TypeSpecifier::Signed,
            Some(Token::Unsigned) => TypeSpecifier::Unsigned,
            _ => {
                return match self.peek(0) {
                    Some(Token::Alignas) => {
                        self.pos += 1;
                        self.expect(&Token::LParen, "(")?;
                        self.capture_until(&Token::RParen);
                        self.expect(&Token::RParen, ")")?;
                        Ok((TypeSpecifier::Alignas, self.span_since(start)))
                    }
                    Some(Token::Struct) | Some(Token::Union) => self.parse_struct_or_union(),
                    Some(Token::Enum) => self.parse_enum_specifier(),
                    Some(Token::Typeof) => self.parse_typeof_specifier(),
                    Some(Token::Ident(_)) | Some(Token::ColonColon) | Some(Token::Lt) => {
                        self.parse_typedef_name()
                    }
                    _ => Err(self.fail_here(format!(
                "expected type specifier, found {}",
                self.describe()
            ))),
                };
            }
        };
        self.pos += 1;
        Ok((spec, self.span_since(start)))
    }

    /// `typedef-name` path with the turbofish-miss check.
    fn parse_typedef_name(&mut self) -> PR<Spanned<TypeSpecifier<R>>> {
        let start = self.pos;
        // See exp::parse_ufcs_path: report classify failures at the start token.
        let err_span = self.cur_span();
        let (path, _) = self.parse_rust_path(false)?;
        let path_span = rust_path_span(&path, self.span_since(start));
        let has_lt = self.at(&Token::Lt);
        let lt_span = self.cur_span();
        match self.resolver.classify_path(&path) {
            Ok((TypeQueryResult::Type, _)) if has_lt => {
                let ctx = path_span.data().context;
                let s = path_span.data().start;
                let e = lt_span.data().end;
                let span = Span::from_parts(ctx, s..e);
                co2_ast::emit_errors_and_terminate(vec![
                    co2_ast::Rich::custom(
                        span,
                        format!("generic arguments require turbofish syntax: `{path}::<...>`"),
                    )
                    .map_token(|tok: Token| tok.to_string()),
                ]);
            }
            Ok((TypeQueryResult::Unsure | TypeQueryResult::Type, resolved)) => Ok((
                TypeSpecifier::TypedefName((resolved, path_span)),
                self.span_since(start),
            )),
            Ok((TypeQueryResult::Expr, _)) => {
                Err(self.fail_at(err_span, "expected type name, found expression".to_string()))
            }
            Err((msg, _)) => Err(self.fail_at(err_span, msg)),
        }
    }

    fn parse_struct_or_union(&mut self) -> PR<Spanned<TypeSpecifier<R>>> {
        let start = self.pos;
        let kind = match self.peek(0) {
            Some(Token::Struct) => StructOrUnionKind::Struct,
            _ => StructOrUnionKind::Union,
        };
        self.pos += 1;
        // Inner span excludes the `struct`/`union` keyword (matches old).
        let inner_start = self.pos;
        let specifier: Spanned<StructOrUnionSpecifier<R>> = if self.at(&Token::LBrace) {
            let fields = self.parse_struct_fields()?;
            let span = self.span_since(inner_start);
            (StructOrUnionSpecifier::Anonymous { fields }, span)
        } else {
            let ident = self.parse_identifier()?;
            if self.at(&Token::LBrace) {
                let fields = self.parse_struct_fields()?;
                let span = self.span_since(inner_start);
                (StructOrUnionSpecifier::Defined { ident, fields }, span)
            } else {
                let span = self.span_since(inner_start);
                (StructOrUnionSpecifier::Declared { ident }, span)
            }
        };
        let span = specifier.1;
        let registered = self
            .resolver
            .register_struct_or_union_specifier(kind, specifier);
        Ok((
            TypeSpecifier::StructOrUnion {
                kind,
                specifier: (registered, span),
            },
            self.span_since(start),
        ))
    }

    /// `{` fields `}`. Caller guarantees cursor is on `{`.
    fn parse_struct_fields(&mut self) -> PR<Vec<Spanned<StructOrUnionField<R>>>> {
        self.expect(&Token::LBrace, "{")?;
        let mut fields = Vec::new();
        loop {
            if self.eat(&Token::RBrace).is_some() {
                break;
            }
            if self.peek(0).is_none() {
                return Err(
                    self.fail_here("expected } or struct member, found end of input".to_string())
                );
            }
            if self.at(&Token::Semicolon) {
                let span = self.peek_span(0);
                self.pos += 1;
                fields.push((
                    StructOrUnionField {
                        specifiers: vec![],
                        declarators: vec![],
                    },
                    span,
                ));
                continue;
            }
            fields.push(self.parse_struct_field()?);
        }
        // Same filter as before: drop bare `;`; drop all-abstract declarator
        // lists unless they carry an anonymous struct/union member.
        Ok(fields
            .into_iter()
            .filter(|(field, _)| {
                if field.specifiers.is_empty() && field.declarators.is_empty() {
                    return false;
                }
                if !field.declarators.is_empty()
                    && field.declarators.iter().all(|(d, _)| {
                        matches!(d.declarator.0, Declarator::Abstract) && d.bits.is_none()
                    })
                {
                    let has_anon_struct_union = field.specifiers.iter().any(|(s, _)| {
                        matches!(
                            s,
                            SpecifierQualifier::TypeSpecifier((
                                TypeSpecifier::StructOrUnion { .. },
                                _,
                            ))
                        )
                    });
                    return has_anon_struct_union;
                }
                true
            })
            .collect())
    }

    fn parse_struct_field(&mut self) -> PR<Spanned<StructOrUnionField<R>>> {
        let start = self.pos;
        let mut specs = vec![self.parse_spec_qualifier()?];
        loop {
            let cp = self.checkpoint();
            match self.parse_struct_declarator_list() {
                Ok(declarators) => {
                    let span = self.span_since(start);
                    return Ok((
                        StructOrUnionField {
                            specifiers: specs,
                            declarators,
                        },
                        span,
                    ));
                }
                Err(_) => {
                    self.restore(cp);
                }
            }
            specs.push(self.parse_spec_qualifier()?);
        }
    }

    fn parse_struct_declarator_list(&mut self) -> PR<Vec<Spanned<StructDeclarator<R>>>> {
        let mut out = Vec::new();
        loop {
            let start = self.pos;
            let declarator = self.parse_declarator()?;
            let bits = if self.eat(&Token::Colon).is_some() {
                Some(crate::exp::parse_assignment(self)?)
            } else {
                None
            };
            let span = self.span_since(start);
            out.push((StructDeclarator { declarator, bits }, span));
            if self.eat(&Token::Comma).is_none() {
                break;
            }
        }
        self.expect(&Token::Semicolon, ";")?;
        Ok(out)
    }

    fn parse_spec_qualifier(&mut self) -> PR<Spanned<SpecifierQualifier<R>>> {
        let start = self.pos;
        if is_type_qualifier(self.peek(0)) {
            let q = self.parse_type_qualifier()?;
            return Ok((SpecifierQualifier::TypeQualifier(q), self.span_since(start)));
        }
        let s = self.parse_type_specifier()?;
        Ok((SpecifierQualifier::TypeSpecifier(s), self.span_since(start)))
    }

    fn parse_enum_specifier(&mut self) -> PR<Spanned<TypeSpecifier<R>>> {
        let start = self.pos;
        self.expect(&Token::Enum, "enum")?;
        // Inner span excludes the `enum` keyword (matches old).
        let inner_start = self.pos;
        let ident = match self.peek(0) {
            Some(Token::Ident(_)) => Some(self.parse_identifier()?),
            _ => None,
        };
        let underlying_type = {
            // C23 `enum E : int` underlying type. Backtrack if no type
            // follows (e.g. `enum Color: 2` in `_Generic`).
            let cp = self.checkpoint();
            let mut ut = None;
            if self.eat(&Token::Colon).is_some() {
                let mut list = Vec::new();
                loop {
                    let sstart = self.pos;
                    if is_type_qualifier(self.peek(0)) {
                        let q = self.parse_type_qualifier()?;
                        list.push((
                            SpecifierQualifier::TypeQualifier(q),
                            self.span_since(sstart),
                        ));
                    } else if self.peek_is_type_spec_start() {
                        let s = self.parse_type_specifier()?;
                        list.push((
                            SpecifierQualifier::TypeSpecifier(s),
                            self.span_since(sstart),
                        ));
                    } else {
                        break;
                    }
                }
                if list.is_empty() {
                    self.restore(cp);
                } else {
                    ut = Some(TypeName {
                        specifier_qualifier_list: list,
                        abstract_declarator: None,
                    });
                }
            }
            ut
        };
        let spec: Spanned<EnumSpecifier<R>> = if self.at(&Token::LBrace) {
            self.pos += 1;
            let mut enumerators = Vec::new();
            if !self.at(&Token::RBrace) {
                loop {
                    let estart = self.pos;
                    let ident = self.parse_identifier()?;
                    let value = if self.eat(&Token::Assign).is_some() {
                        Some(crate::exp::parse_assignment(self)?)
                    } else {
                        None
                    };
                    let espan = self.span_since(estart);
                    let reg = self
                        .resolver
                        .register_enumerator((Enumerator { ident, value }, espan));
                    enumerators.push((reg, espan));
                    if self.eat(&Token::Comma).is_some() {
                        if self.at(&Token::RBrace) {
                            break;
                        }
                        continue;
                    }
                    break;
                }
            }
            self.expect(&Token::RBrace, "}")?;
            let span = self.span_since(inner_start);
            match ident {
                Some(ident) => (
                    EnumSpecifier::Defined {
                        ident,
                        underlying_type,
                        enumerators,
                    },
                    span,
                ),
                None => (
                    EnumSpecifier::Anonymous {
                        underlying_type,
                        enumerators,
                    },
                    span,
                ),
            }
        } else {
            let ident = ident.ok_or_else(|| {
                self.fail_here(format!(
                    "expected identifier or {{, found {}",
                    self.describe()
                ))
            })?;
            let span = self.span_since(inner_start);
            (
                EnumSpecifier::Declared {
                    ident,
                    underlying_type,
                },
                span,
            )
        };
        let span = spec.1;
        let reg = self.resolver.register_enum_specifier(spec);
        Ok((TypeSpecifier::Enum((reg, span)), self.span_since(start)))
    }

    fn peek_is_type_spec_start(&self) -> bool {
        matches!(
            self.peek(0),
            Some(
                Token::Int
                    | Token::Bool
                    | Token::Void
                    | Token::Char
                    | Token::Short
                    | Token::Long
                    | Token::Float
                    | Token::Double
                    | Token::Signed
                    | Token::Unsigned
                    | Token::Alignas
                    | Token::Struct
                    | Token::Union
                    | Token::Enum
                    | Token::Typeof
                    | Token::Ident(_)
                    | Token::ColonColon
                    | Token::Lt,
            )
        )
    }

    fn parse_typeof_specifier(&mut self) -> PR<Spanned<TypeSpecifier<R>>> {
        let start = self.pos;
        self.expect(&Token::Typeof, "typeof")?;
        self.expect(&Token::LParen, "(")?;
        // Try type-name first; fall back to expression.
        let cp = self.checkpoint();
        match self.parse_type_name() {
            Ok(ty) => {
                self.expect(&Token::RParen, ")")?;
                Ok((
                    TypeSpecifier::TypeofType(Box::new(ty)),
                    self.span_since(start),
                ))
            }
            Err(_) => {
                self.restore(cp);
                let expr = crate::exp::parse_expression(self)?;
                self.expect(&Token::RParen, ")")?;
                Ok((
                    TypeSpecifier::TypeofExpr(Box::new(expr)),
                    self.span_since(start),
                ))
            }
        }
    }

    pub fn parse_type_name(&mut self) -> PR<TypeName<R>> {
        let mut list = Vec::new();
        loop {
            let sstart = self.pos;
            if is_type_qualifier(self.peek(0)) {
                let q = self.parse_type_qualifier()?;
                list.push((
                    SpecifierQualifier::TypeQualifier(q),
                    self.span_since(sstart),
                ));
            } else if self.peek_is_type_spec_start() {
                // Avoid consuming a lone identifier that is not a type: probe.
                let cp = self.checkpoint();
                match self.parse_type_specifier() {
                    Ok(s) => {
                        list.push((
                            SpecifierQualifier::TypeSpecifier(s),
                            self.span_since(sstart),
                        ));
                    }
                    Err(e) => {
                        self.restore(cp);
                        if list.is_empty() {
                            return Err(e);
                        }
                        break;
                    }
                }
            } else {
                break;
            }
        }
        if list.is_empty() {
            return Err(self.fail_here(format!("expected type name, found {}", self.describe())));
        }
        let decl = self.try_parse_declarator_opt()?;
        Ok(TypeName {
            specifier_qualifier_list: list,
            abstract_declarator: decl.and_then(|d| {
                if matches!(d.0, Declarator::Abstract) {
                    None
                } else {
                    Some(d)
                }
            }),
        })
    }

    /// Declarator if one is present, else Abstract without consuming... but an
    /// empty abstract must only apply where valid; here we always return Some
    /// and let the caller drop Abstract. Never fails.
    fn try_parse_declarator_opt(&mut self) -> PR<Option<Spanned<Declarator<R>>>> {
        // Only parse if the next tokens can start a declarator (including
        // abstract array tails like `[3]` in `(int[3]){...}`).
        match self.peek(0) {
            Some(Token::Star | Token::LParen | Token::LBracket | Token::Ident(_)) => {
                Ok(Some(self.parse_declarator()?))
            }
            _ => Ok(None),
        }
    }

    fn parse_decl_specifier(&mut self) -> PR<Spanned<DeclarationSpecifier<R>>> {
        match self.parse_decl_specifier_inner() {
            Ok(v) => Ok(v),
            Err(e) => {
                let found = match self.peek(0) {
                    Some(t) => format!("'{t}'"),
                    None => "end of input".to_string(),
                };
                Err(Fail {
                    span: e.span,
                    msg: format!(
                        "found {found} expected Type specifier, Type qualifier, Storage specifier, Function specifier, or something else"
                    ),
                })
            }
        }
    }

    fn parse_decl_specifier_inner(&mut self) -> PR<Spanned<DeclarationSpecifier<R>>> {
        let start = self.pos;
        match self.peek(0) {
            // NOTE: `_Atomic` always parses as a qualifier here, matching the
            // old grammar where the qualifier alternative preceded storage.
            Some(
                Token::Typedef
                | Token::Extern
                | Token::Static
                | Token::Constexpr
                | Token::ThreadLocal
                | Token::Auto
                | Token::Register,
            ) => {
                let s = self.parse_storage_class()?;
                Ok((
                    DeclarationSpecifier::StorageSpecifier(s),
                    self.span_since(start),
                ))
            }
            Some(Token::Inline) => {
                let kw_span = self.peek_span(0);
                self.pos += 1;
                Ok((
                    DeclarationSpecifier::FunctionSpecifier((FunctionSpecifier::Inline, kw_span)),
                    self.span_since(start),
                ))
            }
            Some(Token::Hash) | Some(Token::DocComment { .. }) => {
                let attrs = self.parse_attr_list()?;
                let span = self.span_since(start);
                Ok((DeclarationSpecifier::GNUAttribute(attrs), span))
            }
            _ if is_type_qualifier(self.peek(0)) => {
                let q = self.parse_type_qualifier()?;
                Ok((
                    DeclarationSpecifier::TypeQualifier(q),
                    self.span_since(start),
                ))
            }
            _ => {
                let s = self.parse_type_specifier()?;
                Ok((
                    DeclarationSpecifier::TypeSpecifier(s),
                    self.span_since(start),
                ))
            }
        }
    }

    // ── Declarators ──

    pub fn parse_declarator(&mut self) -> PR<Spanned<Declarator<R>>> {
        let start = self.pos;
        let mut pointers: Vec<(Vec<Spanned<TypeQualifier>>, Span)> = Vec::new();
        while self.at(&Token::Star) {
            let star = self.peek_span(0);
            self.pos += 1;
            let mut quals = Vec::new();
            while is_type_qualifier(self.peek(0)) {
                quals.push(self.parse_type_qualifier()?);
            }
            pointers.push((quals, star));
        }
        let mut base = self.parse_direct_declarator()?;
        for (quals, star_span) in pointers.into_iter().rev() {
            let span = join_spans(star_span, base.1);
            base = (
                Declarator::PointerDeclarator {
                    declarator: Box::new(base),
                    qualifiers: quals,
                },
                span,
            );
            let _ = start;
        }
        Ok(base)
    }

    fn parse_direct_declarator(&mut self) -> PR<Spanned<Declarator<R>>> {
        let mut base: Spanned<Declarator<R>> = if self.at(&Token::LParen) {
            // Could be a grouped declarator. (Parameter tails are handled in
            // the loop below; a group always starts with `(` too, so try it.)
            // To avoid misparsing `f(int)` when called at declaration start...
            // note this fn is only called after pointers, where the next token
            // decides: ident -> named, `(` -> group. `f(int)`: base `f` is an
            // ident, never reaches here. Only true groups/abstracts arrive.
            let cp = self.checkpoint();
            self.pos += 1;
            match self.parse_declarator() {
                Ok(inner) => {
                    self.expect(&Token::RParen, ")")?;
                    inner
                }
                Err(_) => {
                    self.restore(cp);
                    // Abstract empty (e.g. `void` param, `(*)` cast).
                    (Declarator::Abstract, self.cur_span())
                }
            }
        } else if let Some(Token::Ident(_)) = self.peek(0) {
            let (name, span) = self.parse_identifier()?;
            let ident = self.resolver.register_ident(name);
            (Declarator::Identifier((ident, span)), span)
        } else {
            (Declarator::Abstract, self.cur_span())
        };
        loop {
            if self.at(&Token::LParen) {
                let tstart = self.pos;
                let (params, tail_span) = self.parse_parameter_type_list()?;
                let base_span = base.1;
                let placeholder = matches!(base.0, Declarator::Abstract);
                base = (
                    Declarator::FunctionDeclarator {
                        declarator: Box::new(base),
                        param_list: params,
                    },
                    if placeholder {
                        tail_span
                    } else {
                        join_spans(base_span, tail_span)
                    },
                );
                let _ = tstart;
            } else if self.at(&Token::LBracket) {
                let tstart = self.pos;
                let full = self.capture_balanced_full(&Token::LBracket, &Token::RBracket)?;
                let tail_span = self.span_since(tstart);
                let sub_span = slice_span(&full, tail_span);
                let sub = self
                    .resolver
                    .register_subscription((LazySubscription { tokens: full }, sub_span));
                let base_span = base.1;
                let placeholder = matches!(base.0, Declarator::Abstract);
                base = (
                    Declarator::ArrayDeclarator {
                        declarator: Box::new(base),
                        subscription: (sub, sub_span),
                    },
                    if placeholder {
                        tail_span
                    } else {
                        join_spans(base_span, tail_span)
                    },
                );
            } else {
                break;
            }
        }
        Ok(base)
    }

    fn parse_parameter_type_list(&mut self) -> PR<(ParameterList<R>, Span)> {
        let start = self.pos;
        self.expect(&Token::LParen, "(")?;
        let mut parameters = Vec::new();
        let mut ellipsis = false;
        if !self.at(&Token::RParen) {
            loop {
                parameters.push(self.parse_parameter_single()?);
                if self.eat(&Token::Comma).is_none() {
                    break;
                }
                // `, ...` terminates the list (old grammar required a comma
                // before `...`; bare `(...)` was rejected too).
                // A trailing `,)` is rejected like gcc: loop back and
                // require another parameter.
                if self.eat(&Token::Ellipsis).is_some() {
                    ellipsis = true;
                    break;
                }
            }
        }
        self.expect(&Token::RParen, ")")?;
        let span = self.span_since(start);
        Ok((
            ParameterList {
                parameters,
                ellipsis,
                empty_is_variadic: true,
            },
            span,
        ))
    }

    fn parse_parameter_single(
        &mut self,
    ) -> PR<(
        Vec<Spanned<DeclarationSpecifier<R>>>,
        Spanned<Declarator<R>>,
    )> {
        let mut specs = vec![self.parse_decl_specifier()?];
        loop {
            let has_type_spec = specs
                .iter()
                .any(|(s, _)| matches!(s, DeclarationSpecifier::TypeSpecifier(_)));
            let mut next_is_typedef_name = false;
            if !has_type_spec {
                if let Some(Token::Ident(s)) = self.peek(0) {
                    let path = RustPath::<StatelessResolver>::from_ident((
                        s.clone(),
                        Span::from_parts(FileId::INVALID, 0..0),
                    ));
                    if let Ok((TypeQueryResult::Type | TypeQueryResult::Unsure, _)) =
                        self.resolver.classify_path(&path)
                    {
                        next_is_typedef_name = true;
                    }
                }
            }
            if !next_is_typedef_name {
                let cp = self.checkpoint();
                match self.parse_declarator() {
                    Ok(decl) => {
                        if self.at(&Token::RParen) || self.at(&Token::Comma) {
                            return Ok((specs, decl));
                        }
                        self.restore(cp);
                    }
                    Err(_) => {
                        self.restore(cp);
                    }
                }
            }
            // If the declarator attempt failed or the next token cannot end a
            // parameter, another specifier must follow — otherwise error out
            // instead of looping forever.
            if self.at(&Token::RParen) || self.at(&Token::Comma) {
                // Declarator attempt above failed but we are at a boundary:
                // bare spec (e.g. `void`, `int` in prototype). Synthesize an
                // abstract declarator.
                let span = self.cur_span();
                return Ok((specs, (Declarator::Abstract, span)));
            }
            specs.push(self.parse_decl_specifier()?);
        }
    }
}

fn declarator_has_name<R: TypeResolver>(decl: &Declarator<R>) -> bool {
    match decl {
        Declarator::Identifier(_) => true,
        Declarator::Abstract => false,
        Declarator::FunctionDeclarator { declarator, .. }
        | Declarator::PointerDeclarator { declarator, .. }
        | Declarator::ArrayDeclarator { declarator, .. } => declarator_has_name(&declarator.0),
    }
}

// In C, a function cannot return a function (only a pointer to one). A valid
// function-definition declarator therefore never has a FunctionDeclarator
// immediately wrapping another FunctionDeclarator.
fn function_decl_direct_inner_is_not_function<R: TypeResolver>(decl: &Declarator<R>) -> bool {
    match decl {
        Declarator::FunctionDeclarator { declarator, .. } => {
            !matches!(&declarator.0, Declarator::FunctionDeclarator { .. })
        }
        _ => true,
    }
}

// ── Declarations ─────────────────────────────────────────────────────

impl<'a, R: TypeResolver> P<'a, R> {
    fn parse_static_assert(&mut self) -> PR<Declaration<R>> {
        self.expect(&Token::StaticAssert, "static_assert")?;
        self.expect(&Token::LParen, "(")?;
        let expr = crate::exp::parse_assignment(self)?;
        let mut message: Option<(String, Span)> = None;
        if self.eat(&Token::Comma).is_some() {
            let mut parts = Vec::new();
            loop {
                match self.peek(0) {
                    Some(Token::StringLit(s)) => {
                        parts.push(s.clone());
                        self.pos += 1;
                    }
                    _ => break,
                }
            }
            if parts.is_empty() {
                return Err(self.fail_here(format!(
                    "expected string literal, found {}",
                    self.describe()
                )));
            }
            let span = self.cur_span();
            let literal = merge_string_literals(parts, span);
            let text = String::from_utf8_lossy(literal.to_bytes().as_ref()).into_owned();
            message = Some((text, span));
        }
        self.expect(&Token::RParen, ")")?;
        self.expect(&Token::Semicolon, ";")?;
        Ok(Declaration::StaticAssert {
            expr,
            message: message
                .unwrap_or_else(|| (String::new(), Span::from_parts(FileId::INVALID, 0..0))),
        })
    }

    /// One-or-more `declarator (= init)?` items. Registers each name before
    /// parsing its initializer (C11 6.2.1p7). Empty vec if no declarator.
    fn parse_init_declarator_list(&mut self) -> PR<Vec<Spanned<InitDeclarator<R>>>> {
        let mut result = Vec::new();
        loop {
            let cp = self.checkpoint();
            let item_start = self.pos;
            let decl = match self.parse_declarator() {
                Ok(d) => d,
                Err(_) => {
                    self.restore(cp);
                    break;
                }
            };
            if !declarator_has_name(&decl.0) || !function_decl_direct_inner_is_not_function(&decl.0)
            {
                self.restore(cp);
                break;
            }
            if let Some(ident) = decl.0.ident() {
                let r = self.resolver.clone();
                self.resolver = r.declare_ident_as_local(&ident);
            }
            // `= init` is optional-lookahead: on failure rewind the input
            // (keeping the resolver, like the old `or_not`) and continue
            // without an initializer.
            let init = if self.at(&Token::Assign) {
                let init_pos = self.pos;
                self.pos += 1; // `=`
                match crate::exp::parse_initializer(self) {
                    Ok(init) => Some(init),
                    Err(_) => {
                        self.pos = init_pos;
                        None
                    }
                }
            } else {
                None
            };
            let is_transparent_union = self.eat(&Token::TransparentUnionAttr).is_some();
            let span = self.span_since(item_start);
            result.push((
                InitDeclarator {
                    declarator: decl,
                    initializer: init,
                    is_transparent_union,
                },
                span,
            ));
            if self.eat(&Token::Comma).is_none() {
                break;
            }
        }
        Ok(result)
    }

    pub fn parse_declaration(&mut self) -> PR<Spanned<Declaration<R>>> {
        let start = self.pos;
        if self.at(&Token::StaticAssert) {
            let d = self.parse_static_assert()?;
            let span = self.span_since(start);
            let nr = self.resolver.register_decl(&d);
            self.resolver = nr;
            return Ok((d, span));
        }
        if self.resolver.rust_style_syntax_enabled() && self.peek_rust_fn() {
            let d = self.parse_rust_fn_def(Vec::new())?;
            let span = self.span_since(start);
            let nr = self.resolver.register_decl(&d);
            self.resolver = nr;
            return Ok((d, span));
        }
        if self.resolver.rust_style_syntax_enabled() && self.peek_rust_type_alias() {
            let d = self.parse_rust_type_def(Vec::new())?;
            let span = self.span_since(start);
            let nr = self.resolver.register_decl(&d);
            self.resolver = nr;
            return Ok((d, span));
        }
        // C declaration: specifiers first, then function or object form.
        let mut specs = vec![self.parse_decl_specifier()?];
        loop {
            // Function-definition base: declarator (function) + attrs? + `{`.
            {
                let cp = self.checkpoint();
                match self.parse_declarator() {
                    Ok(decl) => {
                        if declarator_has_name(&decl.0)
                            && decl.0.is_function()
                            && function_decl_direct_inner_is_not_function(&decl.0)
                        {
                            let attrs = self.parse_attr_list().unwrap_or_default();
                            if self.at(&Token::LBrace) {
                                let body = self.parse_lazy_compound()?;
                                let span = self.span_since(start);
                                let d = Declaration::FunctionDefinition {
                                    attrs,
                                    signature: FunctionDefinitionSignature::C {
                                        declaration_specifiers: specs,
                                        declarator: decl,
                                    },
                                    body,
                                };
                                let nr = self.resolver.register_decl(&d);
                                self.resolver = nr;
                                return Ok((d, span));
                            }
                        }
                        self.restore(cp);
                    }
                    Err(_) => {
                        self.restore(cp);
                    }
                }
            }
            // Object-declaration base: init-declarators + attrs? + `;`.
            {
                let cp = self.checkpoint();
                match self.parse_init_declarator_list() {
                    Ok(declarators) => {
                        let trailing = self.parse_attr_list().unwrap_or_default();
                        if self.eat(&Token::Semicolon).is_some() {
                            let span = self.span_since(start);
                            let mut leading: Vec<Spanned<RustAttribute>> = specs
                                .iter()
                                .filter_map(|spec| match &spec.0 {
                                    DeclarationSpecifier::GNUAttribute(attrs) => {
                                        Some(attrs.clone())
                                    }
                                    _ => None,
                                })
                                .flatten()
                                .collect();
                            leading.extend(trailing);
                            let d = Declaration::Declaration {
                                attrs: leading,
                                declaration_specifiers: specs,
                                declarators,
                            };
                            let nr = self.resolver.register_decl(&d);
                            self.resolver = nr;
                            return Ok((d, span));
                        }
                        self.restore(cp);
                    }
                    Err(_) => {
                        self.restore(cp);
                    }
                }
            }
            specs.push(self.parse_decl_specifier()?);
        }
    }

    fn peek_after_pub(&self) -> usize {
        // Offset of the token after an optional `pub` / `pub(...)` prefix.
        if !matches!(self.peek(0), Some(Token::Ident(s)) if s == "pub") {
            return 0;
        }
        if self.peek(1) != Some(&Token::LParen) {
            return 1;
        }
        let mut depth = 0u32;
        let mut i = 1;
        while let Some(t) = self.peek(i) {
            if t == &Token::LParen {
                depth += 1;
            } else if t == &Token::RParen {
                if depth == 1 {
                    return i + 1;
                }
                depth -= 1;
            }
            i += 1;
        }
        i
    }

    fn peek_rust_fn(&self) -> bool {
        matches!(self.peek(self.peek_after_pub()), Some(Token::Ident(s)) if s == "fn")
    }

    fn peek_rust_type_alias(&self) -> bool {
        matches!(self.peek(self.peek_after_pub()), Some(Token::Ident(s)) if s == "type")
    }

    // ── Rust-style items ──

    fn parse_rust_fn_def(&mut self, attrs: Vec<Spanned<RustAttribute>>) -> PR<Declaration<R>> {
        let (visibility, err_span) = self.parse_pub_token()?;
        if let Some(span) = err_span {
            co2_ast::emit_errors(vec![co2_ast::Rich::custom(span, "invalid pub specifier")]);
        }
        if self.eat_ident("fn").is_none() {
            return Err(self.fail_here(format!("expected fn, found {}", self.describe())));
        }
        let (name, name_span) = self.parse_identifier()?;
        let params = self.parse_rust_params()?;
        let ret_ty = if self.eat(&Token::Arrow).is_some() {
            self.parse_rust_ty()?
        } else {
            (RustTy::Tuple(vec![]), self.cur_span())
        };
        let body = self.parse_lazy_compound()?;
        Ok(Declaration::FunctionDefinition {
            attrs: Vec::new(),
            signature: FunctionDefinitionSignature::Rust(RustFunctionSignature {
                attrs,
                name: (self.resolver.register_ident(name), name_span),
                params,
                ret_ty,
                visibility,
            }),
            body,
        })
    }

    fn parse_rust_params(&mut self) -> PR<Vec<RustFunctionParam<R>>> {
        self.expect(&Token::LParen, "(")?;
        let mut out = Vec::new();
        if !self.at(&Token::RParen) {
            loop {
                let (name, name_span) = self.parse_identifier()?;
                self.expect(&Token::Colon, ":")?;
                let ty = self.parse_rust_ty()?;
                out.push(RustFunctionParam {
                    name: (self.resolver.register_ident(name), name_span),
                    ty,
                });
                if self.eat(&Token::Comma).is_some() {
                    if self.at(&Token::RParen) {
                        break;
                    }
                    continue;
                }
                break;
            }
        }
        self.expect(&Token::RParen, ")")?;
        Ok(out)
    }

    fn parse_rust_type_def(&mut self, attrs: Vec<Spanned<RustAttribute>>) -> PR<Declaration<R>> {
        let visibility = match self.peek(0) {
            Some(Token::Ident(s)) if s == "pub" => {
                self.pos += 1;
                Visibility::Public
            }
            _ => Visibility::Private,
        };
        if self.eat_ident("type").is_none() {
            return Err(self.fail_here(format!("expected type, found {}", self.describe())));
        }
        let (name, name_span) = self.parse_identifier()?;
        self.expect(&Token::Assign, "=")?;
        let ty = self.parse_rust_ty()?;
        Ok(Declaration::RustTypeAlias {
            attrs,
            ident: (self.resolver.register_ident(name), name_span),
            ty,
            visibility,
        })
    }

    fn parse_rust_struct_def(&mut self, attrs: Vec<Spanned<RustAttribute>>) -> PR<Declaration<R>> {
        let (visibility, err_span) = self.parse_pub_token()?;
        if let Some(span) = err_span {
            co2_ast::emit_errors(vec![co2_ast::Rich::custom(span, "invalid pub specifier")]);
        }
        if !matches!(self.peek(0), Some(Token::Struct)) {
            return Err(self.fail_here(format!("expected struct, found {}", self.describe())));
        }
        self.pos += 1;
        let (name, name_span) = self.parse_identifier()?;
        self.expect(&Token::LBrace, "{")?;
        let mut fields = Vec::new();
        if !self.at(&Token::RBrace) {
            loop {
                let (vis, err_span) = self.parse_pub_token()?;
                if let Some(span) = err_span {
                    co2_ast::emit_errors(vec![co2_ast::Rich::custom(
                        span,
                        "invalid pub specifier",
                    )]);
                }
                let (fname, fspan) = self.parse_identifier()?;
                self.expect(&Token::Colon, ":")?;
                let ty = self.parse_rust_ty()?;
                fields.push(RustStructField {
                    name: (self.resolver.register_ident(fname), fspan),
                    visibility: vis,
                    ty,
                });
                if self.eat(&Token::Comma).is_some() {
                    if self.at(&Token::RBrace) {
                        break;
                    }
                    continue;
                }
                break;
            }
        }
        self.expect(&Token::RBrace, "}")?;
        Ok(Declaration::RustStruct {
            attrs,
            ident: (self.resolver.register_ident(name), name_span),
            fields,
            visibility,
        })
    }

    /// `{ ... }` captured lazily (braces included).
    pub(crate) fn parse_lazy_compound(&mut self) -> PR<Spanned<LazyCompoundStatement>> {
        let start = self.pos;
        let full = self.capture_balanced_full(&Token::LBrace, &Token::RBrace)?;
        let span = self.span_since(start);
        Ok((
            LazyCompoundStatement {
                tokens: (full, span),
            },
            span,
        ))
    }

    // ── use / mod ──

    fn parse_braced_use_tree(&mut self) -> PR<Vec<(Vec<Spanned<String>>, Option<Spanned<String>>)>> {
        self.pos += 1; // `{`
        let mut out = Vec::new();
        if !self.at(&Token::RBrace) {
            loop {
                out.extend(self.parse_use_tree()?);
                if self.eat(&Token::Comma).is_some() {
                    if self.at(&Token::RBrace) {
                        break;
                    }
                    continue;
                }
                break;
            }
        }
        self.expect(&Token::RBrace, "}")?;
        Ok(out)
    }

    fn parse_use_tree(&mut self) -> PR<Vec<(Vec<Spanned<String>>, Option<Spanned<String>>)>> {
        if self.at(&Token::LBrace) {
            return self.parse_braced_use_tree();
        }
        if self.at(&Token::Star) {
            let span = self.peek_span(0);
            self.pos += 1;
            return Ok(vec![(vec![("*".to_string(), span)], None)]);
        }
        let mut prefix = vec![self.parse_identifier()?];
        while self.at(&Token::ColonColon) {
            // Look ahead: `{` or `*` after `::` starts a nested/group form.
            let is_nested = matches!(self.peek(1), Some(Token::LBrace | Token::Star));
            if !is_nested {
                self.pos += 1;
                prefix.push(self.parse_identifier()?);
                continue;
            }
            break;
        }
        let mut nested: Option<Vec<(Vec<Spanned<String>>, Option<Spanned<String>>)>> = None;
        if self.at(&Token::ColonColon) {
            self.pos += 1;
            if self.at(&Token::Star) {
                let span = self.peek_span(0);
                self.pos += 1;
                nested = Some(vec![(vec![("*".to_string(), span)], None)]);
            } else if self.at(&Token::LBrace) {
                nested = Some(self.parse_braced_use_tree()?);
            } else {
                return Err(self.fail_here(format!("expected {{ or *, found {}", self.describe())));
            }
        }
        let alias = if self.eat_ident("as").is_some() {
            Some(self.parse_identifier()?)
        } else {
            None
        };
        if let Some(items) = nested {
            let mut flat = Vec::new();
            for (mut path, a) in items {
                let mut full = prefix.clone();
                full.append(&mut path);
                flat.push((full, a));
            }
            return Ok(flat);
        }
        Ok(vec![(prefix, alias)])
    }

    fn parse_use_items(&mut self, attrs: Vec<Spanned<RustAttribute>>) -> PR<Vec<Spanned<UseItem>>> {
        let start = self.pos;
        if self.eat_ident("use").is_none() {
            return Err(self.fail_here(format!("expected use, found {}", self.describe())));
        }
        if self.at(&Token::ColonColon) {
            self.pos += 1;
        }
        let items = self.parse_use_tree()?;
        self.expect(&Token::Semicolon, ";")?;
        let span = self.span_since(start);
        Ok(items
            .into_iter()
            .map(|(path, alias)| {
                (
                    UseItem {
                        attrs: attrs.clone(),
                        path,
                        alias,
                    },
                    span,
                )
            })
            .collect())
    }

    fn parse_mod_item(&mut self, attrs: Vec<Spanned<RustAttribute>>) -> PR<Spanned<ModItem>> {
        let start = self.pos;
        if self.eat_ident("mod").is_none() {
            return Err(self.fail_here(format!("expected mod, found {}", self.describe())));
        }
        let name = self.parse_identifier()?;
        let item = if self.eat(&Token::Semicolon).is_some() {
            ModItem {
                attrs,
                name,
                inline_content: None,
            }
        } else if self.at(&Token::LBrace) {
            let (inner, span) = self.capture_balanced(&Token::LBrace, &Token::RBrace)?;
            ModItem {
                attrs,
                name,
                inline_content: Some((inner, span)),
            }
        } else {
            return Err(self.fail_here(format!("expected ; or {{, found {}", self.describe())));
        };
        Ok((item, self.span_since(start)))
    }

    fn parse_break_co2(&mut self) -> PR<Spanned<Declaration<R>>> {
        let start = self.pos;
        self.expect(&Token::Break, "break")?;
        if self.eat_ident("co2").is_none() {
            return Err(self.fail_here(format!("expected co2, found {}", self.describe())));
        }
        self.expect(&Token::Semicolon, ";")?;
        Ok((Declaration::BreakCo2, self.span_since(start)))
    }

    fn parse_pragma_pack(&mut self) -> PR<Spanned<Declaration<R>>> {
        let start = self.pos;
        let action = match self.peek(0) {
            Some(Token::Ident(s)) => pragma_pack_action(s),
            _ => None,
        };
        let Some(action) = action else {
            return Err(self.fail_here(format!(
                "expected pragma pack action, found {}",
                self.describe()
            )));
        };
        self.pos += 1;
        self.expect(&Token::Semicolon, ";")?;
        Ok((Declaration::PragmaPack { action }, self.span_since(start)))
    }
}

fn pragma_pack_action(ident: &str) -> Option<co2_ast::PackAction> {
    use co2_ast::PackAction;
    match ident {
        "__ccc_pack_pop" => Some(PackAction::Pop),
        "__ccc_pack_reset" => Some(PackAction::Reset),
        "__ccc_pack_push_only" => Some(PackAction::PushOnly),
        _ => {
            if let Some(n) = ident
                .strip_prefix("__ccc_pack_push_")
                .and_then(|s| s.parse::<u32>().ok())
            {
                return Some(PackAction::PushSet(n));
            }
            ident
                .strip_prefix("__ccc_pack_set_")
                .and_then(|s| s.parse::<u32>().ok())
                .map(PackAction::Set)
        }
    }
}

fn attach_attrs_to_declaration<R: TypeResolver>(
    mut decl: Declaration<R>,
    attrs: Vec<Spanned<RustAttribute>>,
) -> Declaration<R> {
    match &mut decl {
        Declaration::FunctionDefinition {
            attrs: decl_attrs, ..
        }
        | Declaration::Declaration {
            attrs: decl_attrs, ..
        }
        | Declaration::RustTypeAlias {
            attrs: decl_attrs, ..
        }
        | Declaration::RustStruct {
            attrs: decl_attrs, ..
        } => *decl_attrs = attrs,
        Declaration::PragmaPack { .. } | Declaration::BreakCo2 => {}
        Declaration::StaticAssert { .. } => {}
    }
    decl
}

fn attrs_are_outer(attrs: &[Spanned<RustAttribute>]) -> bool {
    attrs.iter().all(|(attr, _)| !attr.is_inner())
}

// ── Translation unit ─────────────────────────────────────────────────

/// Parse a whole TU. Aborts on the first bad item (matching the old
/// combinator behavior, where a failing item kills the whole TU parse).
pub(crate) fn parse_tu<R: TypeResolver>(
    toks: &[Spanned<Token>],
    end_span: Span,
    resolver: R,
) -> PR<Spanned<TranslationUnit<R>>> {
    let mut p = P::new(toks, end_span, resolver);
    let mut rust_use_items = Vec::new();
    let mut rust_mod_items = Vec::new();
    let mut declarations = Vec::new();
    let mut tu_attrs = Vec::new();

    while p.peek(0).is_some() {
        let mut attrs = p.parse_attr_list()?;
        // Split leading inner attrs into TU attrs.
        if let Some(first_outer) = attrs.iter().position(|(attr, _)| !attr.is_inner())
            && first_outer > 0
        {
            tu_attrs.extend(attrs.drain(..first_outer));
        }
        if !attrs.is_empty() && attrs.iter().all(|(attr, _)| attr.is_inner()) {
            tu_attrs.extend(attrs);
            continue;
        }
        parse_tu_item(
            &mut p,
            attrs,
            &mut rust_use_items,
            &mut rust_mod_items,
            &mut declarations,
        )?;
    }

    let span = slice_span(toks, end_span);
    Ok((
        TranslationUnit {
            attrs: tu_attrs,
            rust_use_items,
            rust_mod_items,
            items: declarations,
        },
        span,
    ))
}

#[allow(clippy::too_many_arguments)]
fn parse_tu_item<R: TypeResolver>(
    p: &mut P<'_, R>,
    attrs: Vec<Spanned<RustAttribute>>,
    rust_use_items: &mut Vec<Spanned<UseItem>>,
    rust_mod_items: &mut Vec<Spanned<ModItem>>,
    declarations: &mut Vec<Spanned<Declaration<R>>>,
) -> PR<()> {
    if !attrs.is_empty() {
        if !attrs_are_outer(&attrs) {
            let span = attrs
                .first()
                .zip(attrs.last())
                .map_or(Span::from_parts(FileId::INVALID, 0..0), |(f, l)| {
                    join_spans(f.1, l.1)
                });
            return Err(p.fail_at(
                span,
                "inner doc comments are only supported before module contents".to_string(),
            ));
        }
        // try use / mod / rust-style-with-attrs / declaration+attach
        let cp = p.checkpoint();
        match p.parse_use_items(attrs.clone()) {
            Ok(items) => {
                rust_use_items.extend(items);
                return Ok(());
            }
            Err(_) => p.restore(cp),
        }
        let cp = p.checkpoint();
        match p.parse_mod_item(attrs.clone()) {
            Ok(item) => {
                rust_mod_items.push(item);
                return Ok(());
            }
            Err(_) => p.restore(cp),
        }
        if p.resolver.rust_style_syntax_enabled() {
            if p.peek_rust_fn()
                || p.peek_rust_type_alias()
                || matches!(p.peek(0), Some(Token::Struct) | Some(Token::Ident(_)))
            {
                let start = p.pos;
                let cp = p.checkpoint();
                let r = if p.peek_rust_fn() {
                    p.parse_rust_fn_def(attrs.clone())
                } else if p.peek_rust_type_alias() {
                    p.parse_rust_type_def(attrs.clone())
                } else {
                    p.parse_rust_struct_def(attrs.clone())
                };
                match r {
                    Ok(d) => {
                        let span = p.span_since(start);
                        let nr = p.resolver.register_decl(&d);
                        p.resolver = nr;
                        declarations.push((d, span));
                        return Ok(());
                    }
                    Err(_) => p.restore(cp),
                }
            }
        }
        let cp = p.checkpoint();
        match p.parse_declaration() {
            Ok((d, span)) => {
                let d = attach_attrs_to_declaration(d, attrs);
                declarations.push((d, span));
                return Ok(());
            }
            Err(_) => p.restore(cp),
        }
        let span = attrs
            .first()
            .zip(attrs.last())
            .map_or(Span::from_parts(FileId::INVALID, 0..0), |(f, l)| {
                join_spans(f.1, l.1)
            });
        return Err(p.fail_at(
            span,
            "attributes are only supported on rust items".to_string(),
        ));
    }
    // No attrs: use / mod / break / pack / rust-struct / declaration / `;`.
    let cp = p.checkpoint();
    match p.parse_use_items(Vec::new()) {
        Ok(items) => {
            rust_use_items.extend(items);
            return Ok(());
        }
        Err(_) => p.restore(cp),
    }
    let cp = p.checkpoint();
    match p.parse_mod_item(Vec::new()) {
        Ok(item) => {
            rust_mod_items.push(item);
            return Ok(());
        }
        Err(_) => p.restore(cp),
    }
    let cp = p.checkpoint();
    match p.parse_break_co2() {
        Ok((d, span)) => {
            let nr = p.resolver.register_decl(&d);
            p.resolver = nr;
            declarations.push((d, span));
            return Ok(());
        }
        Err(_) => p.restore(cp),
    }
    let cp = p.checkpoint();
    match p.parse_pragma_pack() {
        Ok((d, span)) => {
            let nr = p.resolver.register_decl(&d);
            p.resolver = nr;
            declarations.push((d, span));
            return Ok(());
        }
        Err(_) => p.restore(cp),
    }
    // Rust struct (no attrs) before plain declaration, mirroring old order.
    if p.resolver.rust_style_syntax_enabled()
        && matches!(p.peek(0), Some(Token::Struct) | Some(Token::Ident(_)))
    {
        let start = p.pos;
        let cp = p.checkpoint();
        match p.parse_rust_struct_def(Vec::new()) {
            Ok(d) => {
                let span = p.span_since(start);
                let nr = p.resolver.register_decl(&d);
                p.resolver = nr;
                declarations.push((d, span));
                return Ok(());
            }
            Err(_) => p.restore(cp),
        }
    }
    let cp = p.checkpoint();
    match p.parse_declaration() {
        Ok(decl) => {
            declarations.push(decl);
            return Ok(());
        }
        Err(_) => p.restore(cp),
    }
    if p.eat(&Token::Semicolon).is_some() {
        return Ok(());
    }
    // Match the old TU-final `just(Token::Semicolon)` failure wording.
    Err(p.fail_here(match p.peek(0) {
        Some(t) => format!("found '{t}' expected ';'"),
        None => "found end of input expected ';'".to_string(),
    }))
}

// ── Statements ───────────────────────────────────────────────────────

impl<'a, R: TypeResolver> P<'a, R> {
    /// `{` ... `}` with scope handling. Caller must be on `{`.
    pub fn parse_compound_inner(&mut self) -> PR<Spanned<CompoundStatement<R>>> {
        let start = self.pos;
        self.expect(&Token::LBrace, "{")?;
        let outer = self.resolver.clone();
        self.resolver = outer.clone().start_new_scope();
        let mut statements = Vec::new();
        let res = loop {
            if self.eat(&Token::RBrace).is_some() {
                break Ok(());
            }
            if self.peek(0).is_none() {
                break Err(
                    self.fail_here("expected } or statement, found end of input".to_string())
                );
            }
            match self.parse_stmt_or_decl() {
                Ok(item) => statements.push(item),
                Err(e) => break Err(e),
            }
        };
        self.resolver = outer;
        res?;
        Ok((CompoundStatement { statements }, self.span_since(start)))
    }

    fn parse_stmt_or_decl(&mut self) -> PR<Spanned<StatementOrDeclaration<R>>> {
        if self.prefer_declaration() {
            let d = self.parse_declaration()?;
            let span = d.1;
            Ok((StatementOrDeclaration::Declaration(d), span))
        } else {
            let s = self.parse_statement()?;
            let span = s.1;
            Ok((StatementOrDeclaration::Statement(s), span))
        }
    }

    fn prefer_declaration(&self) -> bool {
        match self.peek(0) {
            Some(
                Token::Typedef
                | Token::Extern
                | Token::Static
                | Token::Constexpr
                | Token::Atomic
                | Token::ThreadLocal
                | Token::Auto
                | Token::Register
                | Token::Inline
                | Token::Const
                | Token::Restrict
                | Token::Volatile
                | Token::Struct
                | Token::Union
                | Token::Enum
                | Token::Int
                | Token::Bool
                | Token::Void
                | Token::Char
                | Token::Short
                | Token::Long
                | Token::Float
                | Token::Double
                | Token::Signed
                | Token::Unsigned
                | Token::Typeof
                | Token::Alignas
                | Token::StaticAssert,
            ) => true,
            Some(Token::Ident(_)) => {
                if self.is_label_ahead() {
                    return false;
                }
                matches!(
                    self.peek_classify(),
                    Some((TypeQueryResult::Unsure | TypeQueryResult::Type, _))
                )
            }
            Some(Token::ColonColon) => {
                matches!(self.peek_classify(), Some((TypeQueryResult::Type, _)))
            }
            _ => false,
        }
    }

    /// Classify the path at the cursor without consuming anything.
    fn peek_classify(&self) -> Option<(TypeQueryResult, R::ResolvedRustPath)> {
        let mut tmp = P {
            toks: self.toks,
            pos: self.pos,
            end_span: self.end_span,
            resolver: self.resolver.clone(),
        };
        match tmp.parse_rust_path(true) {
            Ok((path, _)) => self.resolver.classify_path(&path).ok(),
            Err(_) => None,
        }
    }

    fn is_label_ahead(&self) -> bool {
        let mut tmp = P {
            toks: self.toks,
            pos: self.pos,
            end_span: self.end_span,
            resolver: self.resolver.clone(),
        };
        match tmp.parse_rust_path(true) {
            Ok(_) => tmp.at(&Token::Colon),
            Err(_) => false,
        }
    }

    pub fn parse_statement(&mut self) -> PR<Spanned<Statement<R>>> {
        let start = self.pos;
        let stmt = match self.peek(0) {
            Some(Token::If) => self.parse_if()?,
            Some(Token::While) => self.parse_while()?,
            Some(Token::Do) => self.parse_do_while()?,
            Some(Token::For) => self.parse_for()?,
            Some(Token::Switch) => {
                self.pos += 1;
                self.expect(&Token::LParen, "(")?;
                let expr = crate::exp::parse_expression(self)?;
                self.expect(&Token::RParen, ")")?;
                let body = self.parse_statement()?;
                Statement::Switch {
                    expr,
                    body: Box::new(body),
                }
            }
            Some(Token::Case) => {
                self.pos += 1;
                let expr = crate::exp::parse_expression(self)?;
                self.expect(&Token::Colon, ":")?;
                let statement = self.parse_statement()?;
                Statement::Case {
                    expr,
                    statement: Box::new(statement),
                }
            }
            Some(Token::Default) => {
                let kw_span = self.peek_span(0);
                self.pos += 1;
                self.expect(&Token::Colon, ":")?;
                let statement = self.parse_statement()?;
                Statement::Default {
                    keyword_span: kw_span,
                    statement: Box::new(statement),
                }
            }
            Some(Token::Goto) => {
                self.pos += 1;
                let s = if self.eat(&Token::Star).is_some() {
                    Statement::IndirectGoto(crate::exp::parse_expression(self)?)
                } else {
                    Statement::Goto(self.parse_identifier()?)
                };
                self.expect(&Token::Semicolon, ";")?;
                s
            }
            Some(Token::Break) => {
                self.pos += 1;
                if self.eat_ident("co2").is_some() {
                    self.expect(&Token::Semicolon, ";")?;
                    Statement::BreakCo2
                } else {
                    self.expect(&Token::Semicolon, ";")?;
                    Statement::Break
                }
            }
            Some(Token::Continue) => {
                self.pos += 1;
                self.expect(&Token::Semicolon, ";")?;
                Statement::Continue
            }
            Some(Token::Return) => {
                self.pos += 1;
                let exp = if self.at(&Token::Semicolon) {
                    None
                } else {
                    Some(crate::exp::parse_expression(self)?)
                };
                self.expect(&Token::Semicolon, ";")?;
                Statement::Return(exp)
            }
            Some(Token::Semicolon) => {
                self.pos += 1;
                Statement::Empty
            }
            Some(Token::LBrace) => {
                let body = self.parse_compound_inner()?;
                Statement::Compound(body)
            }
            _ => {
                // Label (`ident :`) or expression statement.
                if matches!(self.peek(0), Some(Token::Ident(_)))
                    && matches!(self.peek(1), Some(Token::Colon))
                {
                    let name = self.parse_identifier()?;
                    self.expect(&Token::Colon, ":")?;
                    let statement = self.parse_statement()?;
                    Statement::Label {
                        name,
                        statement: Box::new(statement),
                    }
                } else {
                    let exp = crate::exp::parse_expression(self)?;
                    self.expect(&Token::Semicolon, ";")?;
                    Statement::Expression(exp)
                }
            }
        };
        Ok((stmt, self.span_since(start)))
    }

    fn parse_if(&mut self) -> PR<Statement<R>> {
        self.expect(&Token::If, "if")?;
        self.expect(&Token::LParen, "(")?;
        let cond = crate::exp::parse_expression(self)?;
        self.expect(&Token::RParen, ")")?;
        let then_branch = self.parse_statement()?;
        let else_branch = if self.at(&Token::Else) {
            self.pos += 1;
            Some(self.parse_statement()?)
        } else {
            None
        };
        Ok(Statement::If {
            cond,
            then_branch: Box::new(then_branch),
            else_branch: else_branch.map(Box::new),
        })
    }

    fn parse_while(&mut self) -> PR<Statement<R>> {
        self.expect(&Token::While, "while")?;
        self.expect(&Token::LParen, "(")?;
        let cond = crate::exp::parse_expression(self)?;
        self.expect(&Token::RParen, ")")?;
        let body = self.parse_statement()?;
        Ok(Statement::While {
            cond,
            body: Box::new(body),
        })
    }

    fn parse_do_while(&mut self) -> PR<Statement<R>> {
        self.expect(&Token::Do, "do")?;
        let body = self.parse_statement()?;
        self.expect(&Token::While, "while")?;
        self.expect(&Token::LParen, "(")?;
        let cond = crate::exp::parse_expression(self)?;
        self.expect(&Token::RParen, ")")?;
        self.expect(&Token::Semicolon, ";")?;
        Ok(Statement::DoWhile {
            body: Box::new(body),
            cond,
        })
    }

    fn parse_for(&mut self) -> PR<Statement<R>> {
        self.expect(&Token::For, "for")?;
        self.expect(&Token::LParen, "(")?;
        let outer = self.resolver.clone();
        let (init, loop_resolver) = {
            let cp = self.checkpoint();
            match self.parse_declaration() {
                Ok(d) => {
                    let nr = self.resolver.clone();
                    (Some(ForInit::Declaration(d)), nr)
                }
                Err(_) => {
                    self.restore(cp);
                    let init = if self.at(&Token::Semicolon) {
                        None
                    } else {
                        Some(ForInit::Expression(crate::exp::parse_expression(self)?))
                    };
                    self.expect(&Token::Semicolon, ";")?;
                    (init, self.resolver.clone())
                }
            }
        };
        let cond = if self.at(&Token::Semicolon) {
            None
        } else {
            Some(crate::exp::parse_expression(self)?)
        };
        self.expect(&Token::Semicolon, ";")?;
        let post = if self.at(&Token::RParen) {
            None
        } else {
            Some(crate::exp::parse_expression(self)?)
        };
        self.expect(&Token::RParen, ")")?;
        self.resolver = loop_resolver;
        let body = self.parse_statement();
        self.resolver = outer;
        Ok(Statement::For {
            init,
            cond,
            post,
            body: Box::new(body?),
        })
    }
}

// ── External entries ─────────────────────────────────────────────────

/// Parse lazily-captured `{ ... }` body tokens. Fatal on error.
pub(crate) fn try_parse_compound<R: TypeResolver>(
    toks: &[Spanned<Token>],
    end_span: Span,
    resolver: R,
) -> Result<Spanned<CompoundStatement<R>>, (Span, String)> {
    let mut p = P::new(toks, end_span, resolver);
    match p.parse_compound_inner() {
        Ok(b) => Ok(b),
        Err(e) => Err((e.span, e.msg)),
    }
}

/// Parse expression tokens with trailing-end enforcement. Fatal on error.
pub(crate) fn try_parse_expr_full<R: TypeResolver>(
    toks: &[Spanned<Token>],
    end_span: Span,
    resolver: R,
) -> Result<Spanned<Expression<R>>, (Span, String)> {
    let mut p = P::new(toks, end_span, resolver);
    match crate::exp::parse_expression(&mut p) {
        Ok(e) => {
            if p.peek(0).is_some() {
                let f = p.fail_here(format!(
                    "expected end of expression, found {}",
                    p.describe()
                ));
                return Err((f.span, f.msg));
            }
            Ok(e)
        }
        Err(e) => Err((e.span, e.msg)),
    }
}
