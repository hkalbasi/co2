#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start_byte: usize,
    pub end_byte: usize,
    pub line: u32,
    pub col: u32,
}

#[derive(Debug, Clone)]
pub struct Token<'src> {
    pub kind: TokenKind,
    pub lexeme: &'src str,
    pub span: Span,
}

/// Every distinct token kind in C/C++. No payload: the text lives in `Token.lexeme`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    // ── Control flow ────────────────────────────────────────────────────────
    KwIf,
    KwElse,
    KwFor,
    KwWhile,
    KwDo,
    KwSwitch,
    KwCase,
    KwDefault,
    KwBreak,
    KwContinue,
    KwReturn,
    KwGoto,

    // ── Type / declaration ───────────────────────────────────────────────────
    KwStruct,
    KwClass,
    KwUnion,
    KwEnum,
    KwTypedef,
    KwNamespace,
    KwTemplate,
    KwTypename,
    KwUsing,

    // ── Access specifiers ────────────────────────────────────────────────────
    KwPublic,
    KwPrivate,
    KwProtected,

    // ── Exception handling ───────────────────────────────────────────────────
    KwTry,
    KwCatch,
    KwThrow,

    // ── Memory ───────────────────────────────────────────────────────────────
    KwNew,
    KwDelete,

    // ── Other keywords (stored in lexeme: void, int, const, static, …) ──────
    Keyword,

    // ── Identifiers ──────────────────────────────────────────────────────────
    Ident,

    // ── Literals (actual text in lexeme) ────────────────────────────────────
    LitInt,
    LitFloat,
    LitStr,  // includes all prefix variants: L"", u"", U"", u8"", and raw R"()"
    LitChar, // includes L'', u'', U''

    // ── Comments ─────────────────────────────────────────────────────────────
    CommentLine,  // // … \n
    CommentBlock, // /* … */

    // ── Preprocessor (entire logical line, including \ continuations) ────────
    PreprocLine,

    // ── Line continuation outside preprocessor ───────────────────────────────
    // A bare `\` immediately before a newline (or EOF) in regular C/C++ code.
    // Phase-1 splicing makes it invisible to the compiler, but we preserve it
    // so the formatted output remains byte-for-byte identical in this regard.
    LineContinuation,

    // ── Delimiters ───────────────────────────────────────────────────────────
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,

    // ── Punctuation ──────────────────────────────────────────────────────────
    Semi,
    Comma,
    Colon,
    ColonColon,
    Question,
    Dot,
    DotDotDot,
    Arrow,
    DotStar,
    ArrowStar,

    // ── Arithmetic operators ──────────────────────────────────────────────────
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    PlusPlus,
    MinusMinus,

    // ── Bitwise operators ─────────────────────────────────────────────────────
    Amp,
    Pipe,
    Caret,
    Tilde,
    LtLt,
    GtGt,

    // ── Logical operators ─────────────────────────────────────────────────────
    AmpAmp,
    PipePipe,
    Bang,

    // ── Comparison operators ──────────────────────────────────────────────────
    EqEq,
    BangEq,
    Lt,
    Gt,
    LtEq,
    GtEq,
    LtEqGt, // <=>

    // ── Assignment operators ──────────────────────────────────────────────────
    Eq,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    AmpEq,
    PipeEq,
    CaretEq,
    LtLtEq,
    GtGtEq,

    // ── Whitespace (kept for blank-line counting in the formatter) ────────────
    Whitespace,
    Newline,

    /// Unrecognised character — emitted during error recovery so the rest of
    /// the file can still be processed. The lexeme holds the raw character.
    Unknown,

    Eof,
}

impl TokenKind {
    /// True for keywords that take a parenthesised expression: if/for/while/switch/…
    pub fn is_control_kw(self) -> bool {
        matches!(
            self,
            Self::KwIf
                | Self::KwFor
                | Self::KwWhile
                | Self::KwSwitch
                | Self::KwReturn
                | Self::KwCase
                | Self::KwGoto
                | Self::KwThrow
                | Self::KwNew
                | Self::KwDelete
                | Self::KwCatch
        )
    }

    /// True for any keyword token variant.
    pub fn is_any_kw(self) -> bool {
        matches!(
            self,
            Self::KwIf
                | Self::KwElse
                | Self::KwFor
                | Self::KwWhile
                | Self::KwDo
                | Self::KwSwitch
                | Self::KwCase
                | Self::KwDefault
                | Self::KwBreak
                | Self::KwContinue
                | Self::KwReturn
                | Self::KwGoto
                | Self::KwStruct
                | Self::KwClass
                | Self::KwUnion
                | Self::KwEnum
                | Self::KwTypedef
                | Self::KwNamespace
                | Self::KwTemplate
                | Self::KwTypename
                | Self::KwUsing
                | Self::KwPublic
                | Self::KwPrivate
                | Self::KwProtected
                | Self::KwTry
                | Self::KwCatch
                | Self::KwThrow
                | Self::KwNew
                | Self::KwDelete
                | Self::Keyword
        )
    }

    /// True for tokens that can be the last token of an expression, so the
    /// next operator would be binary rather than unary.
    pub fn ends_expr(self) -> bool {
        matches!(
            self,
            Self::Ident
                | Self::Keyword
                | Self::LitInt
                | Self::LitFloat
                | Self::LitStr
                | Self::LitChar
                | Self::RParen
                | Self::RBracket
                | Self::PlusPlus
                | Self::MinusMinus
                | Self::CommentBlock
        )
    }

    pub fn is_binary_op(self) -> bool {
        matches!(
            self,
            Self::Plus
                | Self::Minus
                | Self::Star
                | Self::Slash
                | Self::Percent
                | Self::Amp
                | Self::Pipe
                | Self::Caret
                | Self::LtLt
                | Self::GtGt
                | Self::AmpAmp
                | Self::PipePipe
                | Self::EqEq
                | Self::BangEq
                | Self::Lt
                | Self::Gt
                | Self::LtEq
                | Self::GtEq
                | Self::LtEqGt
                | Self::Eq
                | Self::PlusEq
                | Self::MinusEq
                | Self::StarEq
                | Self::SlashEq
                | Self::PercentEq
                | Self::AmpEq
                | Self::PipeEq
                | Self::CaretEq
                | Self::LtLtEq
                | Self::GtGtEq
                | Self::Question
        )
    }
}
