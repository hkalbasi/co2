//! Preprocessing tokenizer — state machine that produces tokens with source-byte spans.
//!
//! Handles line splicing, comment stripping, and C23 digraph conversion inline
//! during tokenization. The input is raw C source text (pre-preprocessing).

use std::ops::Range;

use co2_ast::{FloatSuffix, IntegerSuffix, StringLiteral, StringLiteralPrefix, Token};

use super::utils::{is_ident_cont_byte, is_ident_start_byte};

/// A warning produced during tokenization (e.g. hex escape overflow).
#[derive(Debug, Clone)]
pub(crate) struct TokenizerWarning {
    pub(crate) range: Range<usize>,
    pub(crate) message: String,
}

/// An error produced during tokenization (e.g. invalid escape sequence).
#[derive(Debug, Clone)]
pub(crate) struct TokenizerError {
    pub(crate) range: Range<usize>,
    pub(crate) message: String,
}

/// State machine states for the preprocessing tokenizer.
#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Start,
    BackslashNewline,
    Ident,
    LineComment,
    BlockComment,
    DecimalInteger,
    ZeroLeading,
    HexInteger,
    HexFloatFraction,
    OctalInteger,
    DecimalFloatFraction,
    DecimalFloatExponent,
    HexFloatExponent,
}

/// Internal tokenizer state.
struct Tokenizer<'a> {
    bytes: &'a [u8],
    len: usize,
    pos: usize,
    state: State,
    warnings: Vec<TokenizerWarning>,
    errors: Vec<TokenizerError>,
    token_start: usize,
    buf: Vec<u8>,
}

impl<'a> Tokenizer<'a> {
    fn new(input: &'a str) -> Self {
        let bytes = input.as_bytes();
        Self {
            bytes,
            len: bytes.len(),
            pos: 0,
            state: State::Start,
            warnings: Vec::new(),
            errors: Vec::new(),
            token_start: 0,
            buf: Vec::new(),
        }
    }

    fn remaining(&self) -> &[u8] {
        &self.bytes[self.pos..]
    }

    fn matches(&self, s: &[u8]) -> bool {
        self.remaining().starts_with(s)
    }

    fn warn(&mut self, start: usize, end: usize, msg: String) {
        self.warnings.push(TokenizerWarning {
            range: start..end,
            message: msg,
        });
    }

    fn err(&mut self, start: usize, end: usize, msg: String) {
        self.errors.push(TokenizerError {
            range: start..end,
            message: msg,
        });
    }

    fn ident_to_keyword(&self, ident: &str) -> Token {
        match ident {
            "auto" => Token::Auto,
            "_Bool" => Token::Bool,
            "break" => Token::Break,
            "case" => Token::Case,
            "char" => Token::Char,
            "const" | "__const__" => Token::Const,
            "constexpr" => Token::Constexpr,
            "continue" => Token::Continue,
            "default" => Token::Default,
            "do" => Token::Do,
            "double" => Token::Double,
            "else" => Token::Else,
            "enum" => Token::Enum,
            "extern" => Token::Extern,
            "float" => Token::Float,
            "for" => Token::For,
            "goto" => Token::Goto,
            "if" => Token::If,
            "inline" | "__inline" | "__inline__" => Token::Inline,
            "__int128" | "int" => Token::Int,
            "long" => Token::Long,
            "register" => Token::Register,
            "restrict" | "__restrict" | "__restrict__" => Token::Restrict,
            "return" => Token::Return,
            "short" => Token::Short,
            "signed" | "__signed__" => Token::Signed,
            "sizeof" => Token::Sizeof,
            "typeof" | "__typeof" | "__typeof__" => Token::Typeof,
            "_Alignof" => Token::Alignof,
            "__builtin_offsetof" => Token::Offsetof,
            "static" => Token::Static,
            "_Atomic" => Token::Atomic,
            "struct" => Token::Struct,
            "switch" => Token::Switch,
            "typedef" => Token::Typedef,
            "union" => Token::Union,
            "unsigned" => Token::Unsigned,
            "void" => Token::Void,
            "volatile" => Token::Volatile,
            "while" => Token::While,
            "_Generic" => Token::Generic,
            "__builtin_va_start" | "__builtin_c23_va_start" => Token::VaStart,
            "__builtin_va_arg" | "__builtin_c23_va_arg" => Token::VaArg,
            "__builtin_va_copy" | "__builtin_c23_va_copy" => Token::VaCopy,
            "__builtin_va_end" | "__builtin_c23_va_end" => Token::VaEnd,
            "__builtin_inf"
            | "__builtin_inff"
            | "__builtin_infl"
            | "__builtin_huge_val"
            | "__builtin_huge_valf"
            | "__builtin_huge_vall" => Token::BuiltinInf,
            "__builtin_nan" | "__builtin_nanf" | "__builtin_nanl" => Token::BuiltinNan,
            "__builtin_constant_p" => Token::BuiltinConstantP,
            "__builtin_types_compatible_p" => Token::BuiltinTypesCompatibleP,
            "__co2_transparent_union_attr" => Token::TransparentUnionAttr,
            _ => Token::Ident(ident.to_string()),
        }
    }

    /// Scan the next token from the input. Newlines are skipped (line-level handling by preprocessor).
    #[allow(clippy::needless_continue)]
    fn advance(&mut self, out: &mut Vec<(Token, usize, usize)>) {
        loop {
            if self.pos >= self.len {
                self.flush_incomplete(out);
                return;
            }

            let b = self.bytes[self.pos];

            match self.state {
                State::Start => {
                    if b.is_ascii_whitespace() && b != b'\n' {
                        self.pos += 1;
                        continue;
                    }
                    // Skip newlines — line-level processing is handled by the preprocessor
                    if b == b'\n' {
                        self.pos += 1;
                        continue;
                    }

                    self.token_start = self.pos;

                    // Backslash — might be line splice
                    if b == b'\\' {
                        self.state = State::BackslashNewline;
                        self.pos += 1;
                        continue;
                    }

                    // Digraphs
                    if b == b'%' && self.matches(b"%:%:") {
                        self.pos += 4;
                        out.push((Token::HashHash, self.token_start, self.pos));
                        continue;
                    }
                    if b == b'%' && self.matches(b"%:") {
                        self.pos += 2;
                        out.push((Token::Hash, self.token_start, self.pos));
                        continue;
                    }
                    if b == b'<' && self.matches(b"<%") {
                        self.pos += 2;
                        out.push((Token::LBrace, self.token_start, self.pos));
                        continue;
                    }
                    if b == b'<' && self.matches(b"<:") {
                        self.pos += 2;
                        out.push((Token::LBracket, self.token_start, self.pos));
                        continue;
                    }
                    if b == b':' && self.matches(b":>") {
                        self.pos += 2;
                        out.push((Token::RBracket, self.token_start, self.pos));
                        continue;
                    }
                    if b == b'%' && self.matches(b"%>") {
                        self.pos += 2;
                        out.push((Token::RBrace, self.token_start, self.pos));
                        continue;
                    }

                    // Line comments
                    if b == b'/' && self.pos + 1 < self.len && self.bytes[self.pos + 1] == b'/' {
                        if self.pos + 2 < self.len
                            && matches!(self.bytes[self.pos + 2], b'/' | b'!')
                        {
                            let inner = self.bytes[self.pos + 2] == b'!';
                            let start = self.pos;
                            self.pos += 3;
                            let text_start = self.pos;
                            while self.pos < self.len && self.bytes[self.pos] != b'\n' {
                                self.pos += 1;
                            }
                            let text = std::str::from_utf8(&self.bytes[text_start..self.pos])
                                .unwrap_or("")
                                .to_string();
                            out.push((Token::DocComment { inner, text }, start, self.pos));
                            continue;
                        }
                        self.state = State::LineComment;
                        self.pos += 2;
                        continue;
                    }

                    // Block comments
                    if b == b'/' && self.pos + 1 < self.len && self.bytes[self.pos + 1] == b'*' {
                        self.state = State::BlockComment;
                        self.pos += 2;
                        continue;
                    }

                    // String/char literal prefixes — must check BEFORE identifiers
                    // since u, U, L are valid identifier start chars.
                    if b == b'u' && self.matches(b"u8\"") {
                        self.pos += 3;
                        self.scan_string_body(out, StringLiteralPrefix::Utf8);
                        continue;
                    }
                    if b == b'u' && self.matches(b"u8'") {
                        self.pos += 3;
                        self.scan_char_body(out);
                        continue;
                    }
                    if b == b'u' && self.pos + 1 < self.len && self.bytes[self.pos + 1] == b'"' {
                        self.pos += 2;
                        self.scan_string_body(out, StringLiteralPrefix::Utf16);
                        continue;
                    }
                    if b == b'u' && self.pos + 1 < self.len && self.bytes[self.pos + 1] == b'\'' {
                        self.pos += 2;
                        self.scan_char_body(out);
                        continue;
                    }
                    if b == b'U' && self.pos + 1 < self.len && self.bytes[self.pos + 1] == b'"' {
                        self.pos += 2;
                        self.scan_string_body(out, StringLiteralPrefix::Utf32);
                        continue;
                    }
                    if b == b'U' && self.pos + 1 < self.len && self.bytes[self.pos + 1] == b'\'' {
                        self.pos += 2;
                        self.scan_char_body(out);
                        continue;
                    }
                    if b == b'L' && self.pos + 1 < self.len && self.bytes[self.pos + 1] == b'"' {
                        self.pos += 2;
                        self.scan_string_body(out, StringLiteralPrefix::Wide);
                        continue;
                    }
                    if b == b'L' && self.pos + 1 < self.len && self.bytes[self.pos + 1] == b'\'' {
                        self.pos += 2;
                        self.scan_char_body(out);
                        continue;
                    }

                    // s"..." - Rust &str string literal
                    if b == b's' && self.pos + 1 < self.len && self.bytes[self.pos + 1] == b'"' {
                        self.pos += 2;
                        self.scan_string_body(out, StringLiteralPrefix::Str);
                        continue;
                    }

                    // String literal "
                    if b == b'"' {
                        self.pos += 1;
                        self.scan_string_body(out, StringLiteralPrefix::None);
                        continue;
                    }

                    // Char literal '
                    if b == b'\'' {
                        self.pos += 1;
                        self.scan_char_body(out);
                        continue;
                    }

                    // Identifiers and keywords
                    if is_ident_start_byte(b) {
                        self.buf.clear();
                        self.buf.push(b);
                        self.pos += 1;
                        self.state = State::Ident;
                        continue;
                    }

                    // Numeric literals
                    if b.is_ascii_digit() {
                        self.buf.clear();
                        self.buf.push(b);
                        self.pos += 1;
                        if b == b'0' {
                            self.state = State::ZeroLeading;
                        } else {
                            self.state = State::DecimalInteger;
                        }
                        continue;
                    }
                    if b == b'.'
                        && self.pos + 1 < self.len
                        && self.bytes[self.pos + 1].is_ascii_digit()
                    {
                        self.buf.clear();
                        self.buf.push(b'.');
                        self.pos += 1;
                        self.state = State::DecimalFloatFraction;
                        continue;
                    }

                    // Operators and punctuation
                    let (token, consumed) = self.match_op();
                    if let Some(token) = token {
                        self.pos += consumed;
                        out.push((token, self.token_start, self.pos));
                        continue;
                    }

                    // Unknown byte
                    self.err(
                        self.pos,
                        self.pos + 1,
                        format!("unexpected token '{}'", b as char),
                    );
                    self.pos += 1;
                    continue;
                }

                State::BackslashNewline => {
                    while self.pos < self.len
                        && matches!(self.bytes[self.pos], b' ' | b'\t' | b'\r')
                    {
                        self.pos += 1;
                    }
                    if self.pos < self.len && self.bytes[self.pos] == b'\n' {
                        self.pos += 1;
                        self.state = State::Start;
                        continue;
                    }
                    self.state = State::Start;
                    continue;
                }

                State::Ident => {
                    // Check for line-splice backslash (backslash at end of line)
                    if b == b'\\' && self.pos + 1 < self.len {
                        let mut j = self.pos + 1;
                        while j < self.len && matches!(self.bytes[j], b' ' | b'\t' | b'\r') {
                            j += 1;
                        }
                        if j < self.len && self.bytes[j] == b'\n' {
                            // Line splice: skip backslash, whitespace, newline; continue ident
                            self.pos = j + 1;
                            continue;
                        }
                    }
                    if is_ident_cont_byte(b) {
                        self.buf.push(b);
                        self.pos += 1;
                        continue;
                    }
                    let ident = std::str::from_utf8(&self.buf).unwrap_or("");
                    let token = self.ident_to_keyword(ident);
                    out.push((token, self.token_start, self.pos));
                    self.state = State::Start;
                    self.buf.clear();
                    continue;
                }

                State::ZeroLeading => {
                    if b == b'x' || b == b'X' {
                        self.buf.push(b);
                        self.pos += 1;
                        self.state = State::HexInteger;
                        continue;
                    }
                    if (b'0'..=b'7').contains(&b) {
                        self.buf.push(b);
                        self.pos += 1;
                        self.state = State::OctalInteger;
                        continue;
                    }
                    // 0.4 or 0e10 — float literal starting with '0'.
                    if b == b'.' {
                        self.buf.push(b);
                        self.pos += 1;
                        self.state = State::DecimalFloatFraction;
                        continue;
                    }
                    if b == b'e' || b == b'E' {
                        self.buf.push(b);
                        self.pos += 1;
                        self.state = State::DecimalFloatExponent;
                        continue;
                    }
                    if b == b'_' {
                        self.buf.push(b);
                        self.pos += 1;
                        continue;
                    }
                    self.finish_integer(out);
                    continue;
                }

                State::HexInteger => {
                    if b.is_ascii_hexdigit() {
                        self.buf.push(b);
                        self.pos += 1;
                        continue;
                    }
                    if b == b'.' {
                        self.buf.push(b);
                        self.pos += 1;
                        self.state = State::HexFloatFraction;
                        continue;
                    }
                    if b == b'p' || b == b'P' {
                        self.buf.push(b);
                        self.pos += 1;
                        self.state = State::HexFloatExponent;
                        continue;
                    }
                    if b == b'_' {
                        self.buf.push(b);
                        self.pos += 1;
                        continue;
                    }
                    self.finish_int_suffix(out);
                    continue;
                }

                State::HexFloatFraction => {
                    if b.is_ascii_hexdigit() {
                        self.buf.push(b);
                        self.pos += 1;
                        continue;
                    }
                    if b == b'p' || b == b'P' {
                        self.buf.push(b);
                        self.pos += 1;
                        self.state = State::HexFloatExponent;
                        continue;
                    }
                    self.finish_float_suffix(out);
                    continue;
                }

                State::HexFloatExponent => {
                    if b == b'+' || b == b'-' {
                        self.buf.push(b);
                        self.pos += 1;
                        self.state = State::DecimalFloatExponent;
                        continue;
                    }
                    if b.is_ascii_digit() {
                        self.buf.push(b);
                        self.pos += 1;
                        self.state = State::DecimalFloatExponent;
                        continue;
                    }
                    self.finish_float_suffix(out);
                    continue;
                }

                State::OctalInteger => {
                    if (b'0'..=b'7').contains(&b) {
                        self.buf.push(b);
                        self.pos += 1;
                        continue;
                    }
                    if b == b'e' || b == b'E' || b == b'p' || b == b'P' {
                        self.buf.push(b);
                        self.pos += 1;
                        self.state = State::DecimalFloatExponent;
                        continue;
                    }
                    if b == b'.' {
                        self.buf.push(b);
                        self.pos += 1;
                        self.state = State::DecimalFloatFraction;
                        continue;
                    }
                    if b == b'_' {
                        self.buf.push(b);
                        self.pos += 1;
                        continue;
                    }
                    self.finish_int_suffix(out);
                    continue;
                }

                State::DecimalInteger => {
                    if b.is_ascii_digit() {
                        self.buf.push(b);
                        self.pos += 1;
                        continue;
                    }
                    if b == b'.' {
                        self.buf.push(b);
                        self.pos += 1;
                        self.state = State::DecimalFloatFraction;
                        continue;
                    }
                    if b == b'e' || b == b'E' {
                        self.buf.push(b);
                        self.pos += 1;
                        self.state = State::DecimalFloatExponent;
                        continue;
                    }
                    if b == b'_' {
                        self.buf.push(b);
                        self.pos += 1;
                        continue;
                    }
                    self.finish_int_suffix(out);
                    continue;
                }

                State::DecimalFloatFraction => {
                    if b.is_ascii_digit() {
                        self.buf.push(b);
                        self.pos += 1;
                        continue;
                    }
                    if b == b'e' || b == b'E' {
                        self.buf.push(b);
                        self.pos += 1;
                        self.state = State::DecimalFloatExponent;
                        continue;
                    }
                    self.finish_float_suffix(out);
                    continue;
                }

                State::DecimalFloatExponent => {
                    if b == b'+' || b == b'-' {
                        self.buf.push(b);
                        self.pos += 1;
                        // stay in exponent state — need at least one digit after sign
                        continue;
                    }
                    if b.is_ascii_digit() {
                        self.buf.push(b);
                        self.pos += 1;
                        // Stay in same state to collect all exponent digits
                        continue;
                    }
                    self.finish_float_suffix(out);
                    continue;
                }

                State::LineComment => {
                    if b == b'\n' {
                        self.state = State::Start;
                        continue;
                    }
                    self.pos += 1;
                    continue;
                }

                State::BlockComment => {
                    if b == b'*' && self.pos + 1 < self.len && self.bytes[self.pos + 1] == b'/' {
                        self.pos += 2;
                        self.state = State::Start;
                        continue;
                    }
                    if b == b'\n' {
                        self.pos += 1;
                        continue;
                    }
                    self.pos += 1;
                    continue;
                }
            }
        }
    }

    fn scan_string_body(
        &mut self,
        out: &mut Vec<(Token, usize, usize)>,
        prefix: StringLiteralPrefix,
    ) {
        let start = self.token_start;
        let body = self.scan_string_literal_body();
        out.push((
            Token::StringLit(StringLiteral {
                prefix,
                bytes: body,
            }),
            start,
            self.pos,
        ));
        self.state = State::Start;
        self.buf.clear();
    }

    fn scan_string_literal_body(&mut self) -> Vec<u8> {
        let body_start = self.pos;
        let mut out = Vec::new();
        while self.pos < self.len {
            let b = self.bytes[self.pos];
            if b == b'"' {
                self.pos += 1;
                return out;
            }
            if b == b'\\' && self.pos + 1 < self.len {
                let esc_start = self.pos;
                self.pos += 1;
                let escape = self.bytes[self.pos];
                self.pos += 1;
                self.decode_escape(escape, body_start, esc_start - body_start, &mut out);
                continue;
            }
            out.push(b);
            self.pos += 1;
        }
        out
    }

    fn scan_char_body(&mut self, out: &mut Vec<(Token, usize, usize)>) {
        let start = self.token_start;
        let body = self.scan_char_literal_body();
        out.push((Token::CharLit(body), start, self.pos));
        self.state = State::Start;
        self.buf.clear();
    }

    fn scan_char_literal_body(&mut self) -> Vec<u8> {
        let body_start = self.pos;
        let mut out = Vec::new();
        while self.pos < self.len {
            let b = self.bytes[self.pos];
            if b == b'\'' {
                self.pos += 1;
                return out;
            }
            if b == b'\\' && self.pos + 1 < self.len {
                let esc_start = self.pos;
                self.pos += 1;
                let escape = self.bytes[self.pos];
                self.pos += 1;
                self.decode_escape(escape, body_start, esc_start - body_start, &mut out);
                continue;
            }
            out.push(b);
            self.pos += 1;
        }
        out
    }

    fn decode_escape(
        &mut self,
        escape: u8,
        body_start: usize,
        esc_offset: usize,
        out: &mut Vec<u8>,
    ) {
        match escape {
            b'\n' => {}
            b'\r' => {
                if self.pos < self.len && self.bytes[self.pos] == b'\n' {
                    self.pos += 1;
                }
            }
            b'a' => out.push(b'\x07'),
            b'b' => out.push(b'\x08'),
            b'f' => out.push(b'\x0c'),
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'v' => out.push(b'\x0b'),
            b'\\' => out.push(b'\\'),
            b'\'' => out.push(b'\''),
            b'"' => out.push(b'"'),
            b'?' => out.push(b'?'),
            b'0'..=b'7' => {
                let escape_start_offset = esc_offset;
                let mut digits = String::from(escape as char);
                for _ in 0..2 {
                    if self.pos < self.len && matches!(self.bytes[self.pos], b'0'..=b'7') {
                        digits.push(self.bytes[self.pos] as char);
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                let value = u16::from_str_radix(&digits, 8).unwrap_or(0);
                if value > u16::from(u8::MAX) {
                    let range_start = body_start + escape_start_offset;
                    self.warn(
                        range_start,
                        self.pos,
                        "octal escape sequence out of range; using low 8 bits".to_string(),
                    );
                }
                out.push(value as u8);
            }
            b'x' | b'X' => {
                let escape_start_offset = esc_offset;
                let mut value = 0u8;
                let mut overflowed = false;
                let mut saw_digit = false;
                while self.pos < self.len && self.bytes[self.pos].is_ascii_hexdigit() {
                    let digit_value = match self.bytes[self.pos] {
                        b'0'..=b'9' => self.bytes[self.pos] - b'0',
                        b'a'..=b'f' => self.bytes[self.pos] - b'a' + 10,
                        b'A'..=b'F' => self.bytes[self.pos] - b'A' + 10,
                        _ => unreachable!(),
                    };
                    overflowed |=
                        u16::from(value) * 16 + u16::from(digit_value) > u16::from(u8::MAX);
                    value = value.wrapping_mul(16).wrapping_add(digit_value);
                    saw_digit = true;
                    self.pos += 1;
                }
                if saw_digit {
                    if overflowed {
                        let range_start = body_start + escape_start_offset;
                        self.warn(
                            range_start,
                            self.pos,
                            "hex escape sequence out of range; using low 8 bits".to_string(),
                        );
                    }
                    out.push(value);
                }
            }
            b'u' | b'U' => {
                let digits = if escape == b'u' { 4 } else { 8 };
                let mut value = 0u32;
                let mut valid = true;
                for _ in 0..digits {
                    if self.pos >= self.len || !self.bytes[self.pos].is_ascii_hexdigit() {
                        valid = false;
                        break;
                    }
                    let digit_value = match self.bytes[self.pos] {
                        b'0'..=b'9' => self.bytes[self.pos] - b'0',
                        b'a'..=b'f' => self.bytes[self.pos] - b'a' + 10,
                        b'A'..=b'F' => self.bytes[self.pos] - b'A' + 10,
                        _ => unreachable!(),
                    };
                    value = value * 16 + u32::from(digit_value);
                    self.pos += 1;
                }
                if valid && let Some(ch) = char::from_u32(value) {
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
            }
            _ => {
                out.push(escape);
            }
        }
    }

    fn finish_integer(&mut self, out: &mut Vec<(Token, usize, usize)>) {
        let text = std::str::from_utf8(&self.buf).unwrap_or("").to_string();
        let suffix = self.parse_int_suffix();
        out.push((Token::Integer(text, suffix), self.token_start, self.pos));
        self.state = State::Start;
        self.buf.clear();
    }

    fn finish_int_suffix(&mut self, out: &mut Vec<(Token, usize, usize)>) {
        let text = std::str::from_utf8(&self.buf).unwrap_or("").to_string();
        let suffix = self.parse_int_suffix();
        out.push((Token::Integer(text, suffix), self.token_start, self.pos));
        self.state = State::Start;
        self.buf.clear();
    }

    fn finish_float_suffix(&mut self, out: &mut Vec<(Token, usize, usize)>) {
        let text = std::str::from_utf8(&self.buf).unwrap_or("").to_string();
        let suffix = self.parse_float_suffix();
        out.push((Token::FloatLit(text, suffix), self.token_start, self.pos));
        self.state = State::Start;
        self.buf.clear();
    }

    fn flush_incomplete(&mut self, out: &mut Vec<(Token, usize, usize)>) {
        if self.pos > self.token_start {
            match self.state {
                State::Ident => {
                    let ident = std::str::from_utf8(&self.buf).unwrap_or("").to_string();
                    let token = self.ident_to_keyword(&ident);
                    out.push((token, self.token_start, self.pos));
                }
                State::DecimalInteger
                | State::ZeroLeading
                | State::HexInteger
                | State::OctalInteger => {
                    let text = std::str::from_utf8(&self.buf).unwrap_or("").to_string();
                    let suffix = self.parse_int_suffix();
                    out.push((Token::Integer(text, suffix), self.token_start, self.pos));
                }
                State::DecimalFloatFraction
                | State::DecimalFloatExponent
                | State::HexFloatFraction
                | State::HexFloatExponent => {
                    let text = std::str::from_utf8(&self.buf).unwrap_or("").to_string();
                    out.push((
                        Token::FloatLit(text, FloatSuffix::None),
                        self.token_start,
                        self.pos,
                    ));
                }
                _ => {}
            }
        }
        self.state = State::Start;
        self.buf.clear();
    }

    fn parse_int_suffix(&mut self) -> IntegerSuffix {
        let start = self.pos;

        // Try Rust-style integer suffixes (longest first to avoid partial matches).
        // Check from longest suffix to shortest, so that e.g. "u32" is not
        // misidentified as "u" + "32".
        if start + 5 <= self.len {
            let five = std::str::from_utf8(&self.bytes[start..start + 5]).unwrap_or("");
            match five {
                "usize" => {
                    self.pos += 5;
                    return IntegerSuffix::Usize;
                }
                "isize" => {
                    self.pos += 5;
                    return IntegerSuffix::Isize;
                }
                _ => {}
            }
        }
        if start + 4 <= self.len {
            let four = std::str::from_utf8(&self.bytes[start..start + 4]).unwrap_or("");
            match four {
                "u128" => {
                    self.pos += 4;
                    return IntegerSuffix::U128;
                }
                "i128" => {
                    self.pos += 4;
                    return IntegerSuffix::I128;
                }
                _ => {}
            }
        }
        if start + 3 <= self.len {
            let tri = std::str::from_utf8(&self.bytes[start..start + 3]).unwrap_or("");
            match tri {
                "u32" => {
                    self.pos += 3;
                    return IntegerSuffix::U32;
                }
                "u64" => {
                    self.pos += 3;
                    return IntegerSuffix::U64;
                }
                "u16" => {
                    self.pos += 3;
                    return IntegerSuffix::U16;
                }
                "i32" => {
                    self.pos += 3;
                    return IntegerSuffix::I32;
                }
                "i64" => {
                    self.pos += 3;
                    return IntegerSuffix::I64;
                }
                "i16" => {
                    self.pos += 3;
                    return IntegerSuffix::I16;
                }
                _ => {}
            }
        }
        if start + 2 <= self.len {
            let pair = std::str::from_utf8(&self.bytes[start..start + 2]).unwrap_or("");
            match pair {
                "u8" => {
                    self.pos += 2;
                    return IntegerSuffix::U8;
                }
                "i8" => {
                    self.pos += 2;
                    return IntegerSuffix::I8;
                }
                _ => {}
            }
        }

        // C-style suffixes
        if start + 3 <= self.len {
            let quad = std::str::from_utf8(&self.bytes[start..start + 3]).unwrap_or("");
            match quad {
                "ull" | "ULL" | "llu" | "LLU" => {
                    self.pos += 3;
                    return IntegerSuffix::UnsignedLongLong;
                }
                _ => {}
            }
        }
        if start + 2 <= self.len {
            let pair = std::str::from_utf8(&self.bytes[start..start + 2]).unwrap_or("");
            match pair {
                "ul" | "UL" | "lu" | "LU" => {
                    self.pos += 2;
                    return IntegerSuffix::UnsignedLong;
                }
                "ll" | "LL" => {
                    self.pos += 2;
                    return IntegerSuffix::LongLong;
                }
                _ => {}
            }
        }
        if start < self.len {
            match self.bytes[start] {
                b'u' | b'U' => {
                    self.pos += 1;
                    return IntegerSuffix::Unsigned;
                }
                b'l' | b'L' => {
                    self.pos += 1;
                    return IntegerSuffix::Long;
                }
                _ => {}
            }
        }
        IntegerSuffix::None
    }

    fn parse_float_suffix(&mut self) -> FloatSuffix {
        if self.pos < self.len {
            match self.bytes[self.pos] {
                b'f' | b'F' => {
                    self.pos += 1;
                    return FloatSuffix::Float;
                }
                b'l' | b'L' => {
                    self.pos += 1;
                    return FloatSuffix::Long;
                }
                _ => {}
            }
        }
        FloatSuffix::None
    }

    fn match_op(&self) -> (Option<Token>, usize) {
        let i = self.pos;
        if i + 2 < self.len {
            let triple = std::str::from_utf8(&self.bytes[i..i + 3]).unwrap_or("");
            let token = match triple {
                "..." => Some(Token::Ellipsis),
                "<<=" => Some(Token::ShlAssign),
                ">>=" => Some(Token::ShrAssign),
                _ => None,
            };
            if let Some(token) = token {
                return (Some(token), 3);
            }
        }
        if i + 1 < self.len {
            let pair = std::str::from_utf8(&self.bytes[i..i + 2]).unwrap_or("");
            let token = match pair {
                "->" => Some(Token::Arrow),
                "++" => Some(Token::Inc),
                "--" => Some(Token::Dec),
                "<<" => Some(Token::Shl),
                ">>" => Some(Token::Shr),
                "<=" => Some(Token::Le),
                ">=" => Some(Token::Ge),
                "==" => Some(Token::EqEq),
                "!=" => Some(Token::Ne),
                "&&" => Some(Token::And),
                "||" => Some(Token::Or),
                "+=" => Some(Token::PlusAssign),
                "-=" => Some(Token::MinusAssign),
                "*=" => Some(Token::StarAssign),
                "/=" => Some(Token::SlashAssign),
                "%=" => Some(Token::PercentAssign),
                "&=" => Some(Token::AmpAssign),
                "|=" => Some(Token::PipeAssign),
                "^=" => Some(Token::CaretAssign),
                "##" => Some(Token::HashHash),
                "::" => Some(Token::ColonColon),
                _ => None,
            };
            if let Some(token) = token {
                return (Some(token), 2);
            }
        }
        if self.bytes[i] == b'#' && i + 1 < self.len && is_ident_start_byte(self.bytes[i + 1]) {
            let start = i + 1;
            let mut j = start;
            while j < self.len && is_ident_cont_byte(self.bytes[j]) {
                j += 1;
            }
            let name = std::str::from_utf8(&self.bytes[start..j])
                .unwrap_or("")
                .to_string();
            return (Some(Token::Preprocessor(name)), j - i);
        }
        let token = match self.bytes[i] {
            b'+' => Some(Token::Plus),
            b'-' => Some(Token::Minus),
            b'*' => Some(Token::Star),
            b'/' => Some(Token::Slash),
            b'%' => Some(Token::Percent),
            b'&' => Some(Token::Amp),
            b'|' => Some(Token::Pipe),
            b'^' => Some(Token::Caret),
            b'~' => Some(Token::Tilde),
            b'!' => Some(Token::Bang),
            b'?' => Some(Token::Question),
            b':' => Some(Token::Colon),
            b';' => Some(Token::Semicolon),
            b',' => Some(Token::Comma),
            b'.' => Some(Token::Dot),
            b'=' => Some(Token::Assign),
            b'<' => Some(Token::Lt),
            b'>' => Some(Token::Gt),
            b'#' => Some(Token::Hash),
            b'(' => Some(Token::LParen),
            b')' => Some(Token::RParen),
            b'[' => Some(Token::LBracket),
            b']' => Some(Token::RBracket),
            b'{' => Some(Token::LBrace),
            b'}' => Some(Token::RBrace),
            _ => None,
        };
        if let Some(token) = token {
            return (Some(token), 1);
        }
        (None, 1)
    }
}

/// Tokenize a string, returning all tokens with byte offsets.
pub(crate) fn tokenize(input: &str) -> Vec<(Token, usize, usize)> {
    tokenize_with_diagnostics(input).0
}

/// Tokenize a string, returning tokens and any diagnostics.
pub(crate) fn tokenize_with_diagnostics(
    input: &str,
) -> (
    Vec<(Token, usize, usize)>,
    Vec<TokenizerWarning>,
    Vec<TokenizerError>,
) {
    let mut tok = Tokenizer::new(input);
    let mut tokens = Vec::new();
    tok.advance(&mut tokens);
    (tokens, tok.warnings, tok.errors)
}
