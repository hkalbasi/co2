use chumsky::{input::MapExtra, prelude::*};
use std::fmt;

use crate::{Span, Spanned};

// C Token types
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Token {
    // Keywords
    Auto,
    Bool,
    Break,
    Case,
    Char,
    Const,
    Continue,
    Default,
    Do,
    Double,
    Else,
    Enum,
    Extern,
    Float,
    For,
    Goto,
    If,
    Inline,
    Int,
    Long,
    Register,
    Restrict,
    Return,
    Short,
    Signed,
    Sizeof,
    Static,
    Struct,
    Switch,
    Typedef,
    Union,
    Unsigned,
    Void,
    Volatile,
    While,

    // Identifiers
    Ident(String),

    // Constants
    Integer(String, IntegerSuffix),
    FloatLit(String, FloatSuffix),
    CharLit(String),
    StringLit(String),

    // Operators
    Plus,       // +
    Minus,      // -
    Star,       // *
    Slash,      // /
    Percent,    // %
    Amp,        // &
    Pipe,       // |
    Caret,      // ^
    Tilde,      // ~
    Bang,       // !
    Question,   // ?
    ColonColon, // ::
    Colon,      // :
    Semicolon,  // ;
    Comma,      // ,
    Dot,        // .
    Arrow,      // ->

    // Assignment operators
    Assign,        // =
    PlusAssign,    // +=
    MinusAssign,   // -=
    StarAssign,    // *=
    SlashAssign,   // /=
    PercentAssign, // %=
    AmpAssign,     // &=
    PipeAssign,    // |=
    CaretAssign,   // ^=
    ShlAssign,     // <<=
    ShrAssign,     // >>=

    // Comparison operators
    EqEq, // ==
    Ne,   // !=
    Lt,   // <
    Gt,   // >
    Le,   // <=
    Ge,   // >=

    // Increment/Decrement
    Inc, // ++
    Dec, // --

    // Logical operators
    And, // &&
    Or,  // ||

    // Bitwise shift
    Shl, // <<
    Shr, // >>

    // Brackets and delimiters
    LParen,   // (
    RParen,   // )
    LBracket, // [
    RBracket, // ]
    LBrace,   // {
    RBrace,   // }

    // Preprocessor
    Preprocessor(String),

    // Special
    Ellipsis, // ...
    Hash,     // #
    HashHash, // ##
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum IntegerSuffix {
    None,
    Unsigned,
    Long,
    LongLong,
    UnsignedLong,
    UnsignedLongLong,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum FloatSuffix {
    None,
    Float, // f or F
    Long,  // l or L
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Token::Auto => write!(f, "auto"),
            Token::Bool => write!(f, "_Bool"),
            Token::Break => write!(f, "break"),
            Token::Case => write!(f, "case"),
            Token::Char => write!(f, "char"),
            Token::Const => write!(f, "const"),
            Token::Continue => write!(f, "continue"),
            Token::Default => write!(f, "default"),
            Token::Do => write!(f, "do"),
            Token::Double => write!(f, "double"),
            Token::Else => write!(f, "else"),
            Token::Enum => write!(f, "enum"),
            Token::Extern => write!(f, "extern"),
            Token::Float => write!(f, "float"),
            Token::For => write!(f, "for"),
            Token::Goto => write!(f, "goto"),
            Token::If => write!(f, "if"),
            Token::Inline => write!(f, "inline"),
            Token::Int => write!(f, "int"),
            Token::Long => write!(f, "long"),
            Token::Register => write!(f, "register"),
            Token::Restrict => write!(f, "restrict"),
            Token::Return => write!(f, "return"),
            Token::Short => write!(f, "short"),
            Token::Signed => write!(f, "signed"),
            Token::Sizeof => write!(f, "sizeof"),
            Token::Static => write!(f, "static"),
            Token::Struct => write!(f, "struct"),
            Token::Switch => write!(f, "switch"),
            Token::Typedef => write!(f, "typedef"),
            Token::Union => write!(f, "union"),
            Token::Unsigned => write!(f, "unsigned"),
            Token::Void => write!(f, "void"),
            Token::Volatile => write!(f, "volatile"),
            Token::While => write!(f, "while"),

            Token::Ident(s) => write!(f, "{}", s),

            Token::Integer(num, suffix) => {
                write!(f, "{}", num)?;
                match suffix {
                    IntegerSuffix::None => Ok(()),
                    IntegerSuffix::Unsigned => write!(f, "u"),
                    IntegerSuffix::Long => write!(f, "l"),
                    IntegerSuffix::LongLong => write!(f, "ll"),
                    IntegerSuffix::UnsignedLong => write!(f, "ul"),
                    IntegerSuffix::UnsignedLongLong => write!(f, "ull"),
                }
            }
            Token::FloatLit(num, suffix) => {
                write!(f, "{}", num)?;
                match suffix {
                    FloatSuffix::None => Ok(()),
                    FloatSuffix::Float => write!(f, "f"),
                    FloatSuffix::Long => write!(f, "l"),
                }
            }
            Token::CharLit(s) => write!(f, "'{}'", s),
            Token::StringLit(s) => write!(f, "\"{}\"", s),

            Token::Plus => write!(f, "+"),
            Token::Minus => write!(f, "-"),
            Token::Star => write!(f, "*"),
            Token::Slash => write!(f, "/"),
            Token::Percent => write!(f, "%"),
            Token::Amp => write!(f, "&"),
            Token::Pipe => write!(f, "|"),
            Token::Caret => write!(f, "^"),
            Token::Tilde => write!(f, "~"),
            Token::Bang => write!(f, "!"),
            Token::Question => write!(f, "?"),
            Token::ColonColon => write!(f, "::"),
            Token::Colon => write!(f, ":"),
            Token::Semicolon => write!(f, ";"),
            Token::Comma => write!(f, ","),
            Token::Dot => write!(f, "."),
            Token::Arrow => write!(f, "->"),

            Token::Assign => write!(f, "="),
            Token::PlusAssign => write!(f, "+="),
            Token::MinusAssign => write!(f, "-="),
            Token::StarAssign => write!(f, "*="),
            Token::SlashAssign => write!(f, "/="),
            Token::PercentAssign => write!(f, "%="),
            Token::AmpAssign => write!(f, "&="),
            Token::PipeAssign => write!(f, "|="),
            Token::CaretAssign => write!(f, "^="),
            Token::ShlAssign => write!(f, "<<="),
            Token::ShrAssign => write!(f, ">>="),

            Token::EqEq => write!(f, "=="),
            Token::Ne => write!(f, "!="),
            Token::Lt => write!(f, "<"),
            Token::Gt => write!(f, ">"),
            Token::Le => write!(f, "<="),
            Token::Ge => write!(f, ">="),

            Token::Inc => write!(f, "++"),
            Token::Dec => write!(f, "--"),

            Token::And => write!(f, "&&"),
            Token::Or => write!(f, "||"),

            Token::Shl => write!(f, "<<"),
            Token::Shr => write!(f, ">>"),

            Token::LParen => write!(f, "("),
            Token::RParen => write!(f, ")"),
            Token::LBracket => write!(f, "["),
            Token::RBracket => write!(f, "]"),
            Token::LBrace => write!(f, "{{"),
            Token::RBrace => write!(f, "}}"),

            Token::Preprocessor(dir) => write!(f, "#{}", dir),
            Token::Ellipsis => write!(f, "..."),
            Token::Hash => write!(f, "#"),
            Token::HashHash => write!(f, "##"),
        }
    }
}

// Helper function to parse integer suffixes
fn integer_suffix_parser<'src>()
-> impl Parser<'src, &'src str, IntegerSuffix, extra::Err<Rich<'src, char, Span>>> {
    // First try to parse combinations
    let unsigned_long_long = just("ull")
        .or(just("ULL"))
        .or(just("llu"))
        .or(just("LLU"))
        .to(IntegerSuffix::UnsignedLongLong);

    let unsigned_long = just("ul")
        .or(just("UL"))
        .or(just("lu"))
        .or(just("LU"))
        .to(IntegerSuffix::UnsignedLong);

    let long_long = just("ll").or(just("LL")).to(IntegerSuffix::LongLong);
    let unsigned = just('u').or(just('U')).to(IntegerSuffix::Unsigned);
    let long = just('l').or(just('L')).to(IntegerSuffix::Long);

    choice((unsigned_long_long, unsigned_long, long_long, unsigned, long))
        .or_not()
        .map(|opt| opt.unwrap_or(IntegerSuffix::None))
}

// Helper function to parse float suffixes
fn float_suffix_parser<'src>()
-> impl Parser<'src, &'src str, FloatSuffix, extra::Err<Rich<'src, char, Span>>> {
    just('f')
        .or(just('F'))
        .to(FloatSuffix::Float)
        .or(just('l').or(just('L')).to(FloatSuffix::Long))
        .or_not()
        .map(|opt| opt.unwrap_or(FloatSuffix::None))
}

pub fn lexer<'src: 'static>()
-> impl Parser<'src, &'src str, Vec<Spanned<Token>>, extra::Err<Rich<'src, char, Span>>> {
    // ----- Comments -----
    let line_comment = just("//")
        .then(any().and_is(just('\n').not()).repeated())
        .to_slice()
        .ignored();

    let block_comment = just("/*")
        .then(any().and_is(just("*/").not()).repeated())
        .then(just("*/"))
        .to_slice()
        .ignored();

    let attributes = just("__attribute__")
        .or(just("__asm__"))
        .then(recursive(|block| {
            let content = choice((
                any().filter(|t| !matches!(t, '(' | ')')).ignored(),
                // Recursively skip balanced blocks
                block.ignored(),
            ))
            .repeated()
            .ignored();

            content.delimited_by(just('('), just(')'))
        }))
        .to_slice()
        .ignored();

    // ----- Whitespace -----
    let whitespace = text::whitespace().at_least(1).to_slice().ignored();

    // ----- Preprocessor directives -----
    let preprocessor = just('#')
        .ignore_then(text::ascii::ident().map(|s: &str| s.to_string()))
        .map(Token::Preprocessor);

    // ----- Numeric constants -----
    let hex_digits = one_of("0123456789abcdefABCDEF")
        .repeated()
        .at_least(1)
        .to_slice();

    // Decimal integer
    let decimal_integer = text::int(10)
        .then(integer_suffix_parser())
        .map(|(num, suffix)| Token::Integer(num.to_string(), suffix));

    // Hexadecimal integer
    let hex_integer = just("0x")
        .or(just("0X"))
        .then(hex_digits)
        .to_slice()
        .then(integer_suffix_parser())
        .map(|(num, suffix)| Token::Integer(num.to_string(), suffix));

    // Octal integer (starting with 0)
    let octal_integer = just('0')
        .then(one_of("01234567").repeated().at_least(1))
        .to_slice()
        .then(integer_suffix_parser())
        .map(|(num, suffix)| Token::Integer(num.to_string(), suffix));

    // Float constants
    let decimal_float = text::int(10)
        .then(just('.').ignore_then(text::digits(10).to_slice()).or_not())
        .then(
            just('e')
                .or(just('E'))
                .ignore_then(just('+').or(just('-')).or_not())
                .then(text::digits(10).to_slice())
                .or_not(),
        )
        .try_map(
            |((int, frac), exp): ((&str, Option<&str>), Option<(Option<char>, &str)>), span| {
                if frac.is_none() && exp.is_none() {
                    return Err(Rich::custom(span, "not a float literal"));
                }
                Ok((int, frac, exp))
            },
        )
        .map(
            |(int, frac, exp): (&str, Option<&str>, Option<(Option<char>, &str)>)| {
                let mut result = int.to_string();
                if let Some(frac_digits) = frac {
                    result.push('.');
                    result.push_str(frac_digits);
                }
                if let Some((sign, exp_digits)) = exp {
                    result.push('e');
                    if let Some(s) = sign {
                        result.push(s);
                    }
                    result.push_str(exp_digits);
                }
                result
            },
        )
        .then(float_suffix_parser())
        .map(|(num, suffix)| Token::FloatLit(num, suffix));

    // ----- Character and string literals -----
    let escape_sequence = just('\\').ignore_then(choice((
        just('a').to('\x07'),
        just('b').to('\x08'),
        just('f').to('\x0c'),
        just('n').to('\n'),
        just('r').to('\r'),
        just('t').to('\t'),
        just('v').to('\x0b'),
        just('\\').to('\\'),
        just('\'').to('\''),
        just('"').to('"'),
        just('?').to('?'),
        one_of("01234567")
            .repeated()
            .at_least(1)
            .at_most(3)
            .to_slice()
            .try_map(|s: &str, span| {
                u32::from_str_radix(s, 8)
                    .ok()
                    .and_then(char::from_u32)
                    .ok_or(Rich::custom(span, "invalid octal escape sequence"))
            }),
        just('x').ignore_then(
            one_of("0123456789abcdefABCDEF")
                .repeated()
                .at_least(1)
                .to_slice()
                .try_map(|s: &str, span| {
                    u32::from_str_radix(s, 16)
                        .ok()
                        .and_then(char::from_u32)
                        .ok_or(Rich::custom(span, "invalid hex escape sequence"))
                }),
        ),
    )));

    let char_content = escape_sequence
        .or(none_of("'\\"))
        .repeated()
        .at_least(1)
        .collect::<String>();

    let char_literal = just('\'')
        .ignore_then(char_content)
        .then_ignore(just('\''))
        .map(Token::CharLit);

    let string_content = escape_sequence
        .or(none_of("\"\\"))
        .repeated()
        .collect::<String>();

    let string_literal = just('"')
        .ignore_then(string_content)
        .then_ignore(just('"'))
        .map(Token::StringLit);

    // ----- Operators and punctuators -----
    let operators = choice([
        // Multi-character operators
        just("...").to(Token::Ellipsis),
        just("->").to(Token::Arrow),
        just("++").to(Token::Inc),
        just("--").to(Token::Dec),
        just("<<=").to(Token::ShlAssign),
        just(">>=").to(Token::ShrAssign),
        just("<<").to(Token::Shl),
        just(">>").to(Token::Shr),
        just("<=").to(Token::Le),
        just(">=").to(Token::Ge),
        just("==").to(Token::EqEq),
        just("!=").to(Token::Ne),
        just("&&").to(Token::And),
        just("||").to(Token::Or),
        just("+=").to(Token::PlusAssign),
        just("-=").to(Token::MinusAssign),
        just("*=").to(Token::StarAssign),
        just("/=").to(Token::SlashAssign),
        just("%=").to(Token::PercentAssign),
        just("&=").to(Token::AmpAssign),
        just("|=").to(Token::PipeAssign),
        just("^=").to(Token::CaretAssign),
        just("##").to(Token::HashHash),
        // Single-character operators
        just("+").to(Token::Plus),
        just("-").to(Token::Minus),
        just("*").to(Token::Star),
        just("/").to(Token::Slash),
        just("%").to(Token::Percent),
        just("&").to(Token::Amp),
        just("|").to(Token::Pipe),
        just("^").to(Token::Caret),
        just("~").to(Token::Tilde),
        just("!").to(Token::Bang),
        just("?").to(Token::Question),
        just("::").to(Token::ColonColon),
        just(":").to(Token::Colon),
        just(";").to(Token::Semicolon),
        just(",").to(Token::Comma),
        just(".").to(Token::Dot),
        just("=").to(Token::Assign),
        just("<").to(Token::Lt),
        just(">").to(Token::Gt),
        just("#").to(Token::Hash),
        // Brackets
        just("(").to(Token::LParen),
        just(")").to(Token::RParen),
        just("[").to(Token::LBracket),
        just("]").to(Token::RBracket),
        just("{").to(Token::LBrace),
        just("}").to(Token::RBrace),
    ]);

    // ----- Identifiers and Keywords -----
    let ident = text::ascii::ident()
        .map(|ident: &str| ident.to_string())
        .map(|ident| match ident.as_str() {
            "auto" => Token::Auto,
            "_Bool" => Token::Bool,
            "break" => Token::Break,
            "case" => Token::Case,
            "char" => Token::Char,
            "const" => Token::Const,
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
            "inline" => Token::Inline,
            "int" => Token::Int,
            "long" => Token::Long,
            "register" => Token::Register,
            "restrict" => Token::Restrict,
            "__restrict" => Token::Restrict,
            "return" => Token::Return,
            "short" => Token::Short,
            "signed" => Token::Signed,
            "sizeof" => Token::Sizeof,
            "static" => Token::Static,
            "struct" => Token::Struct,
            "switch" => Token::Switch,
            "typedef" => Token::Typedef,
            "union" => Token::Union,
            "unsigned" => Token::Unsigned,
            "void" => Token::Void,
            "volatile" => Token::Volatile,
            "while" => Token::While,
            _ => Token::Ident(ident),
        });

    // ----- Combined token parser -----
    let token = choice((
        preprocessor,
        hex_integer,
        octal_integer,
        decimal_float,
        decimal_integer,
        char_literal,
        string_literal,
        operators,
        ident,
    ));

    // Skip comments and whitespace between tokens
    let skip = choice((line_comment, block_comment, whitespace, attributes));

    token
        .map_with(|tok, e: &mut MapExtra<_, _>| (tok, e.span()))
        .padded_by(skip.repeated())
        .repeated()
        .collect()
}
