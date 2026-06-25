use crate::error::FunkyError;
use crate::token::{Span, Token, TokenKind};

// ── Cursor ───────────────────────────────────────────────────────────────────

struct Cursor<'src> {
    src: &'src str,
    chars: std::str::Chars<'src>,
    byte_pos: usize,
    line: u32,
    col: u32,
}

impl<'src> Cursor<'src> {
    fn new(src: &'src str) -> Self {
        Self {
            src,
            chars: src.chars(),
            byte_pos: 0,
            line: 1,
            col: 1,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.clone().next()
    }

    fn peek2(&self) -> Option<char> {
        let mut it = self.chars.clone();
        it.next();
        it.next()
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.chars.next()?;
        self.byte_pos += c.len_utf8();
        if c == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    fn eat_if(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn eat_while(&mut self, mut pred: impl FnMut(char) -> bool) {
        while let Some(c) = self.peek() {
            if pred(c) {
                self.advance();
            } else {
                break;
            }
        }
    }

    /// Slice of the source from `start_byte` up to the current position.
    fn slice_from(&self, start_byte: usize) -> &'src str {
        &self.src[start_byte..self.byte_pos]
    }
}

// ── Keyword table ────────────────────────────────────────────────────────────

fn keyword_kind(s: &str) -> Option<TokenKind> {
    Some(match s {
        "if" => TokenKind::KwIf,
        "else" => TokenKind::KwElse,
        "for" => TokenKind::KwFor,
        "while" => TokenKind::KwWhile,
        "do" => TokenKind::KwDo,
        "switch" => TokenKind::KwSwitch,
        "case" => TokenKind::KwCase,
        "default" => TokenKind::KwDefault,
        "break" => TokenKind::KwBreak,
        "continue" => TokenKind::KwContinue,
        "return" => TokenKind::KwReturn,
        "goto" => TokenKind::KwGoto,
        "struct" => TokenKind::KwStruct,
        "class" => TokenKind::KwClass,
        "union" => TokenKind::KwUnion,
        "enum" => TokenKind::KwEnum,
        "typedef" => TokenKind::KwTypedef,
        "namespace" => TokenKind::KwNamespace,
        "template" => TokenKind::KwTemplate,
        "typename" => TokenKind::KwTypename,
        "using" => TokenKind::KwUsing,
        "public" => TokenKind::KwPublic,
        "private" => TokenKind::KwPrivate,
        "protected" => TokenKind::KwProtected,
        "try" => TokenKind::KwTry,
        "catch" => TokenKind::KwCatch,
        "throw" => TokenKind::KwThrow,
        "new" => TokenKind::KwNew,
        "delete" => TokenKind::KwDelete,
        // All remaining C/C++ keywords become generic Keyword tokens.
        "auto" | "extern" | "register" | "static" | "inline" | "volatile" | "const"
        | "constexpr" | "consteval" | "constinit" | "mutable" | "explicit" | "virtual"
        | "override" | "final" | "friend" | "operator" | "sizeof" | "alignof" | "alignas"
        | "decltype" | "noexcept" | "static_assert" | "thread_local" | "export" | "import"
        | "module" | "requires" | "concept" | "co_await" | "co_yield" | "co_return" | "void"
        | "bool" | "char" | "short" | "int" | "long" | "float" | "double" | "signed"
        | "unsigned" | "wchar_t" | "char8_t" | "char16_t" | "char32_t" | "nullptr" | "true"
        | "false" | "this" | "typeid" | "static_cast" | "dynamic_cast" | "reinterpret_cast"
        | "const_cast" | "and" | "and_eq" | "bitand" | "bitor" | "compl" | "not" | "not_eq"
        | "or" | "or_eq" | "xor" | "xor_eq" | "_Bool" | "_Complex" | "_Imaginary" | "_Alignas"
        | "_Alignof" | "_Atomic" | "_Generic" | "_Noreturn" | "_Static_assert"
        | "_Thread_local" | "restrict" | "asm" | "__asm__" | "__asm" | "__volatile__" => {
            TokenKind::Keyword
        }
        _ => return None,
    })
}

/// True for identifier-like prefixes that may precede a string/char literal.
fn is_str_prefix(s: &str) -> bool {
    matches!(s, "L" | "u" | "U" | "u8" | "R" | "LR" | "uR" | "UR" | "u8R")
}

fn is_char_prefix(s: &str) -> bool {
    matches!(s, "L" | "u" | "U" | "u8")
}

// ── Lexer ────────────────────────────────────────────────────────────────────

pub struct Lexer<'src> {
    cursor: Cursor<'src>,
    filename: String,
    warnings: Vec<FunkyError>,
}

impl<'src> Lexer<'src> {
    pub fn new(src: &'src str, filename: impl Into<String>) -> Self {
        Self {
            cursor: Cursor::new(src),
            filename: filename.into(),
            warnings: Vec::new(),
        }
    }

    pub fn tokenize(mut self) -> Result<(Vec<Token<'src>>, Vec<FunkyError>), FunkyError> {
        let mut tokens = Vec::new();
        loop {
            let tok = self.next_token()?;
            let done = tok.kind == TokenKind::Eof;
            tokens.push(tok);
            if done {
                break;
            }
        }
        Ok((tokens, self.warnings))
    }

    fn make(&self, kind: TokenKind, start_byte: usize, line: u32, col: u32) -> Token<'src> {
        Token {
            kind,
            lexeme: self.cursor.slice_from(start_byte),
            span: Span {
                start_byte,
                end_byte: self.cursor.byte_pos,
                line,
                col,
            },
        }
    }

    fn lex_err(&self, msg: impl Into<String>, line: u32, col: u32) -> FunkyError {
        FunkyError::Lex {
            file: self.filename.clone(),
            line,
            col,
            message: msg.into(),
        }
    }

    fn next_token(&mut self) -> Result<Token<'src>, FunkyError> {
        let start = self.cursor.byte_pos;
        let line = self.cursor.line;
        let col = self.cursor.col;

        let c = match self.cursor.advance() {
            None => return Ok(self.make(TokenKind::Eof, start, line, col)),
            Some(c) => c,
        };

        let kind = match c {
            // ── Whitespace ──────────────────────────────────────────────────
            '\r' => {
                self.cursor.eat_if('\n');
                TokenKind::Newline
            }
            '\n' => TokenKind::Newline,
            ' ' | '\t' => {
                self.cursor.eat_while(|c| c == ' ' || c == '\t');
                TokenKind::Whitespace
            }

            // ── Preprocessor directive ──────────────────────────────────────
            '#' => {
                self.scan_preproc();
                TokenKind::PreprocLine
            }

            // ── Single-char delimiters ───────────────────────────────────────
            '(' => TokenKind::LParen,
            ')' => TokenKind::RParen,
            '{' => TokenKind::LBrace,
            '}' => TokenKind::RBrace,
            '[' => TokenKind::LBracket,
            ']' => TokenKind::RBracket,
            ';' => TokenKind::Semi,
            ',' => TokenKind::Comma,
            '?' => TokenKind::Question,
            '~' => TokenKind::Tilde,

            // ── Colon / scope ────────────────────────────────────────────────
            ':' => {
                if self.cursor.eat_if(':') {
                    TokenKind::ColonColon
                } else {
                    TokenKind::Colon
                }
            }

            // ── Dot / ellipsis / pointer-to-member ───────────────────────────
            '.' => {
                if self.cursor.peek() == Some('.') && self.cursor.peek2() == Some('.') {
                    self.cursor.advance();
                    self.cursor.advance();
                    TokenKind::DotDotDot
                } else if self.cursor.eat_if('*') {
                    TokenKind::DotStar
                } else if matches!(self.cursor.peek(), Some('0'..='9')) {
                    // float starting with '.'
                    self.scan_float_tail();
                    TokenKind::LitFloat
                } else {
                    TokenKind::Dot
                }
            }

            // ── / comment / divide ───────────────────────────────────────────
            '/' => {
                if self.cursor.eat_if('/') {
                    self.scan_line_comment();
                    TokenKind::CommentLine
                } else if self.cursor.eat_if('*') {
                    self.scan_block_comment(line, col)?;
                    TokenKind::CommentBlock
                } else if self.cursor.eat_if('=') {
                    TokenKind::SlashEq
                } else {
                    TokenKind::Slash
                }
            }

            // ── + ────────────────────────────────────────────────────────────
            '+' => {
                if self.cursor.eat_if('+') {
                    TokenKind::PlusPlus
                } else if self.cursor.eat_if('=') {
                    TokenKind::PlusEq
                } else {
                    TokenKind::Plus
                }
            }

            // ── - ────────────────────────────────────────────────────────────
            '-' => {
                if self.cursor.eat_if('-') {
                    TokenKind::MinusMinus
                } else if self.cursor.eat_if('=') {
                    TokenKind::MinusEq
                } else if self.cursor.peek() == Some('>') {
                    self.cursor.advance();
                    if self.cursor.eat_if('*') {
                        TokenKind::ArrowStar
                    } else {
                        TokenKind::Arrow
                    }
                } else {
                    TokenKind::Minus
                }
            }

            // ── * ────────────────────────────────────────────────────────────
            '*' => {
                if self.cursor.eat_if('=') {
                    TokenKind::StarEq
                } else {
                    TokenKind::Star
                }
            }

            // ── % ────────────────────────────────────────────────────────────
            '%' => {
                if self.cursor.eat_if('=') {
                    TokenKind::PercentEq
                } else {
                    TokenKind::Percent
                }
            }

            // ── ^ ────────────────────────────────────────────────────────────
            '^' => {
                if self.cursor.eat_if('=') {
                    TokenKind::CaretEq
                } else {
                    TokenKind::Caret
                }
            }

            // ── & ────────────────────────────────────────────────────────────
            '&' => {
                if self.cursor.eat_if('&') {
                    TokenKind::AmpAmp
                } else if self.cursor.eat_if('=') {
                    TokenKind::AmpEq
                } else {
                    TokenKind::Amp
                }
            }

            // ── | ────────────────────────────────────────────────────────────
            '|' => {
                if self.cursor.eat_if('|') {
                    TokenKind::PipePipe
                } else if self.cursor.eat_if('=') {
                    TokenKind::PipeEq
                } else {
                    TokenKind::Pipe
                }
            }

            // ── ! ────────────────────────────────────────────────────────────
            '!' => {
                if self.cursor.eat_if('=') {
                    TokenKind::BangEq
                } else {
                    TokenKind::Bang
                }
            }

            // ── = ────────────────────────────────────────────────────────────
            '=' => {
                if self.cursor.eat_if('=') {
                    TokenKind::EqEq
                } else {
                    TokenKind::Eq
                }
            }

            // ── < ─ << <<= <= <=> ────────────────────────────────────────────
            '<' => {
                if self.cursor.peek() == Some('<') {
                    self.cursor.advance();
                    if self.cursor.eat_if('=') {
                        TokenKind::LtLtEq
                    } else {
                        TokenKind::LtLt
                    }
                } else if self.cursor.eat_if('=') {
                    if self.cursor.eat_if('>') {
                        TokenKind::LtEqGt
                    } else {
                        TokenKind::LtEq
                    }
                } else {
                    TokenKind::Lt
                }
            }

            // ── > >> >>= >= ──────────────────────────────────────────────────
            '>' => {
                if self.cursor.peek() == Some('>') {
                    self.cursor.advance();
                    if self.cursor.eat_if('=') {
                        TokenKind::GtGtEq
                    } else {
                        TokenKind::GtGt
                    }
                } else if self.cursor.eat_if('=') {
                    TokenKind::GtEq
                } else {
                    TokenKind::Gt
                }
            }

            // ── String literals ──────────────────────────────────────────────
            '"' => {
                self.scan_string_content(false)?;
                TokenKind::LitStr
            }

            // ── Char literals ────────────────────────────────────────────────
            '\'' => {
                self.scan_char_content(line, col)?;
                TokenKind::LitChar
            }

            // ── Numeric literals ─────────────────────────────────────────────
            '0'..='9' => self.scan_number(c),

            // ── Identifiers, keywords, and string/char prefixes ───────────────
            // `$` is a GCC/Clang extension allowed in identifiers.
            c if c.is_alphabetic() || c == '_' || c == '$' => {
                self.cursor
                    .eat_while(|c| c.is_alphanumeric() || c == '_' || c == '$');
                let word = self.cursor.slice_from(start);

                // Check for string/char literal prefix
                match self.cursor.peek() {
                    Some('"') if is_str_prefix(word) => {
                        let is_raw = word.contains('R');
                        self.cursor.advance(); // consume "
                        if is_raw {
                            self.scan_raw_string(line, col)?;
                        } else {
                            self.scan_string_content(false)?;
                        }
                        TokenKind::LitStr
                    }
                    Some('\'') if is_char_prefix(word) => {
                        self.cursor.advance(); // consume '
                        self.scan_char_content(line, col)?;
                        TokenKind::LitChar
                    }
                    _ => keyword_kind(word).unwrap_or(TokenKind::Ident),
                }
            }

            // `\` immediately before a newline (or at EOF) is a line-continuation
            // splice.  It is valid outside #define too (C translation phase 1).
            // Emit as LineContinuation without a warning; the Newline token follows.
            '\\' if matches!(self.cursor.peek(), Some('\n') | Some('\r') | None) => {
                TokenKind::LineContinuation
            }

            other => {
                let warn = self.lex_err(
                    format!("unexpected character U+{:04X} {:?}", other as u32, other),
                    line,
                    col,
                );
                self.warnings.push(warn);
                TokenKind::Unknown
            }
        };

        Ok(self.make(kind, start, line, col))
    }

    // ── Scan helpers ─────────────────────────────────────────────────────────

    /// Scan to end of line (after `//` has been consumed). Consumes the `\n`.
    fn scan_line_comment(&mut self) {
        loop {
            match self.cursor.peek() {
                None | Some('\n') => {
                    // Consume the newline so the whole comment including it is
                    // in the lexeme, keeping line numbering correct.
                    self.cursor.advance();
                    break;
                }
                Some('\r') => {
                    self.cursor.advance();
                    self.cursor.eat_if('\n');
                    break;
                }
                _ => {
                    self.cursor.advance();
                }
            }
        }
    }

    /// Scan a block comment body after `/*` has been consumed.
    fn scan_block_comment(&mut self, line: u32, col: u32) -> Result<(), FunkyError> {
        loop {
            match self.cursor.advance() {
                None => {
                    return Err(self.lex_err("unterminated block comment", line, col));
                }
                Some('*') if self.cursor.eat_if('/') => {
                    return Ok(());
                }
                Some('*') => {}
                _ => {}
            }
        }
    }

    /// Scan a preprocessor directive from `#` to end of logical line
    /// (handles `\` line continuations and `/* */` block comments that span lines).
    fn scan_preproc(&mut self) {
        loop {
            match self.cursor.peek() {
                None => break,
                Some('\\') => {
                    self.cursor.advance();
                    // A `\` followed immediately by a newline is a line continuation.
                    if self.cursor.peek() == Some('\r') {
                        self.cursor.advance();
                    }
                    if self.cursor.peek() == Some('\n') {
                        self.cursor.advance();
                    }
                }
                Some('/') => {
                    self.cursor.advance();
                    if self.cursor.peek() == Some('*') {
                        // Block comment — consume through `*/` even across newlines
                        // so the continuation line stays part of this PreprocLine.
                        self.cursor.advance();
                        loop {
                            match self.cursor.advance() {
                                None => break,
                                Some('*') if self.cursor.peek() == Some('/') => {
                                    self.cursor.advance();
                                    break;
                                }
                                _ => {}
                            }
                        }
                    }
                    // `/` alone or `//` — just continue the outer loop
                }
                Some('\n') | Some('\r') => {
                    // End of logical line — consume the newline into the token.
                    if self.cursor.peek() == Some('\r') {
                        self.cursor.advance();
                    }
                    self.cursor.advance();
                    break;
                }
                _ => {
                    self.cursor.advance();
                }
            }
        }
    }

    /// Scan a regular string body after the opening `"` has been consumed.
    fn scan_string_content(&mut self, _wide: bool) -> Result<(), FunkyError> {
        let line = self.cursor.line;
        let col = self.cursor.col;
        loop {
            match self.cursor.advance() {
                None | Some('\n') => {
                    return Err(self.lex_err("unterminated string literal", line, col));
                }
                Some('\\') => {
                    self.cursor.advance(); // skip escaped char (including Unicode)
                }
                Some('"') => return Ok(()),
                _ => {}
            }
        }
    }

    /// Scan a raw string body after `R"` has been consumed.
    /// Syntax: R"delimiter(content)delimiter"
    fn scan_raw_string(&mut self, line: u32, col: u32) -> Result<(), FunkyError> {
        // Read delimiter (up to 16 chars, ends at `(`)
        let delim_start = self.cursor.byte_pos;
        loop {
            match self.cursor.peek() {
                None | Some('\n') | Some('\r') | Some(' ') | Some('\\') | Some('"') => {
                    return Err(self.lex_err("invalid raw string delimiter", line, col));
                }
                Some('(') => {
                    break;
                }
                _ => {
                    if self.cursor.byte_pos - delim_start >= 16 {
                        return Err(self.lex_err("raw string delimiter too long", line, col));
                    }
                    self.cursor.advance();
                }
            }
        }
        let delim = self.cursor.slice_from(delim_start).to_owned();
        self.cursor.advance(); // consume `(`

        // Scan content until `)delimiter"`.
        let close = format!("){}\"", delim);
        loop {
            match self.cursor.advance() {
                None => return Err(self.lex_err("unterminated raw string literal", line, col)),
                Some(')') => {
                    // Try to match the delimiter and closing quote.
                    let remaining = &self.cursor.src[self.cursor.byte_pos..];
                    let need = format!("{}\"", delim);
                    if remaining.starts_with(&need) {
                        // Consume delimiter + closing quote.
                        for _ in 0..need.chars().count() {
                            self.cursor.advance();
                        }
                        let _ = close; // suppress unused warning
                        return Ok(());
                    }
                }
                _ => {}
            }
        }
    }

    /// Scan a char literal body after the opening `'` has been consumed.
    fn scan_char_content(&mut self, line: u32, col: u32) -> Result<(), FunkyError> {
        loop {
            match self.cursor.advance() {
                None | Some('\n') => {
                    return Err(self.lex_err("unterminated character literal", line, col));
                }
                Some('\\') => {
                    self.cursor.advance();
                }
                Some('\'') => return Ok(()),
                _ => {}
            }
        }
    }

    /// Scan a numeric literal. `first` is the first digit already consumed.
    fn scan_number(&mut self, first: char) -> TokenKind {
        let mut is_float = false;

        if first == '0' {
            match self.cursor.peek() {
                Some('x') | Some('X') => {
                    // Hexadecimal
                    self.cursor.advance();
                    self.cursor.eat_while(|c| c.is_ascii_hexdigit() || c == '_');
                    if self.cursor.peek() == Some('.') {
                        // Hex float
                        is_float = true;
                        self.cursor.advance();
                        self.cursor.eat_while(|c| c.is_ascii_hexdigit() || c == '_');
                    }
                    if matches!(self.cursor.peek(), Some('p') | Some('P')) {
                        is_float = true;
                        self.cursor.advance();
                        self.cursor.eat_if('+');
                        self.cursor.eat_if('-');
                        self.cursor.eat_while(|c| c.is_ascii_digit());
                    }
                }
                Some('b') | Some('B') => {
                    // Binary
                    self.cursor.advance();
                    self.cursor.eat_while(|c| c == '0' || c == '1' || c == '_');
                }
                Some('0'..='7') => {
                    // Octal
                    self.cursor
                        .eat_while(|c| ('0'..='7').contains(&c) || c == '_');
                }
                _ => {}
            }
        } else {
            self.cursor.eat_while(|c| c.is_ascii_digit() || c == '_');
        }

        // Decimal / float suffix
        if !is_float {
            if self.cursor.peek() == Some('.') && self.cursor.peek2().is_some_and(|c| c != '.') {
                is_float = true;
                self.cursor.advance();
                self.cursor.eat_while(|c| c.is_ascii_digit() || c == '_');
            }
            if matches!(self.cursor.peek(), Some('e') | Some('E')) {
                is_float = true;
                self.cursor.advance();
                self.cursor.eat_if('+');
                self.cursor.eat_if('-');
                self.cursor.eat_while(|c| c.is_ascii_digit());
            }
        }

        // Consume suffix (u, l, ll, f, etc.)
        self.cursor
            .eat_while(|c| matches!(c, 'u' | 'U' | 'l' | 'L' | 'f' | 'F' | 'z' | 'Z'));

        if is_float {
            TokenKind::LitFloat
        } else {
            TokenKind::LitInt
        }
    }

    /// Consume digits/exponent when a float starts with `.` (e.g. `.5e-3`).
    fn scan_float_tail(&mut self) {
        self.cursor.eat_while(|c| c.is_ascii_digit() || c == '_');
        if matches!(self.cursor.peek(), Some('e') | Some('E')) {
            self.cursor.advance();
            self.cursor.eat_if('+');
            self.cursor.eat_if('-');
            self.cursor.eat_while(|c| c.is_ascii_digit());
        }
        self.cursor
            .eat_while(|c| matches!(c, 'f' | 'F' | 'l' | 'L'));
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

pub fn tokenize<'src>(
    src: &'src str,
    filename: impl Into<String>,
) -> Result<(Vec<Token<'src>>, Vec<FunkyError>), FunkyError> {
    Lexer::new(src, filename).tokenize()
}
