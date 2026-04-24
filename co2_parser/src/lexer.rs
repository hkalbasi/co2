use chumsky::{input::MapExtra, prelude::*, span::SimpleSpan};
use co2_ast::{FloatSuffix, IntegerSuffix, Token};

// Helper function to parse integer suffixes
fn integer_suffix_parser<'src>()
-> impl Parser<'src, &'src str, IntegerSuffix, extra::Err<Rich<'src, char, SimpleSpan<usize>>>> {
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
-> impl Parser<'src, &'src str, FloatSuffix, extra::Err<Rich<'src, char, SimpleSpan<usize>>>> {
    just('f')
        .or(just('F'))
        .to(FloatSuffix::Float)
        .or(just('l').or(just('L')).to(FloatSuffix::Long))
        .or_not()
        .map(|opt| opt.unwrap_or(FloatSuffix::None))
}

pub fn lexer<'src>() -> impl Parser<
    'src,
    &'src str,
    Vec<(Token, SimpleSpan<usize>)>,
    extra::Err<Rich<'src, char, SimpleSpan<usize>>>,
> {
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
            "const" | "__const__" => Token::Const,
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
            "inline" | "__inline__" => Token::Inline,
            "__int128" | "int" => Token::Int,
            "long" => Token::Long,
            "register" => Token::Register,
            "restrict" | "__restrict" => Token::Restrict,
            "return" => Token::Return,
            "short" => Token::Short,
            "signed" | "__signed__" => Token::Signed,
            "sizeof" => Token::Sizeof,
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
            "__builtin_va_end" | "__builtin_c23_va_end" => Token::VaEnd,
            "__builtin_inf" | "__builtin_inff" | "__builtin_infl" => Token::BuiltinInf,
            "__builtin_nan" | "__builtin_nanf" | "__builtin_nanl" => Token::BuiltinNan,

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
