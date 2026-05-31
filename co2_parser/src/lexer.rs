use chumsky::{error::Rich, input::MapExtra, prelude::*, span::SimpleSpan};
use co2_ast::{FloatSuffix, IntegerSuffix, StringLiteral, StringLiteralPrefix, Token};

type LexerError<'src> = Rich<'src, char, SimpleSpan<usize>>;
type LexerWarning<'src> = Rich<'src, String, SimpleSpan<usize>>;
type LexerExtra<'src> =
    extra::Full<LexerError<'src>, extra::SimpleState<Vec<LexerWarning<'src>>>, ()>;

fn literal_subspan(
    literal_span: SimpleSpan<usize>,
    start_offset: usize,
    end_offset: usize,
) -> SimpleSpan<usize> {
    SimpleSpan::new(
        (),
        (literal_span.start + start_offset)..(literal_span.start + end_offset),
    )
}

fn push_escape_overflow_warning<'src>(
    e: &mut MapExtra<'src, '_, &'src str, LexerExtra<'src>>,
    span: SimpleSpan<usize>,
    kind: &str,
) {
    e.state().push(Rich::custom(
        span,
        format!("{kind} escape sequence out of range; using low 8 bits"),
    ));
}

fn decode_literal_bytes<'src>(
    raw: &str,
    literal_span: SimpleSpan<usize>,
    e: &mut MapExtra<'src, '_, &'src str, LexerExtra<'src>>,
) -> Result<Vec<u8>, LexerError<'src>> {
    let mut out = Vec::with_capacity(raw.len());
    let mut chars = raw.char_indices().peekable();

    while let Some((idx, ch)) = chars.next() {
        if ch != '\\' {
            let mut buf = [0u8; 4];
            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
            continue;
        }

        let Some((escape_idx, escape)) = chars.next() else {
            return Err(Rich::custom(
                literal_subspan(literal_span, idx, raw.len()),
                "unterminated escape sequence",
            ));
        };

        match escape {
            '\n' => {}
            '\r' => {
                if let Some(&(_, '\n')) = chars.peek() {
                    chars.next();
                }
            }
            'a' => out.push(b'\x07'),
            'b' => out.push(b'\x08'),
            'f' => out.push(b'\x0c'),
            'n' => out.push(b'\n'),
            'r' => out.push(b'\r'),
            't' => out.push(b'\t'),
            'v' => out.push(b'\x0b'),
            '\\' => out.push(b'\\'),
            '\'' => out.push(b'\''),
            '"' => out.push(b'"'),
            '?' => out.push(b'?'),
            '0'..='7' => {
                let mut digits = String::from(escape);
                let mut end_offset = escape_idx + escape.len_utf8();

                for _ in 0..2 {
                    let Some(&(next_idx, next_ch)) = chars.peek() else {
                        break;
                    };
                    if !matches!(next_ch, '0'..='7') {
                        break;
                    }
                    digits.push(next_ch);
                    end_offset = next_idx + next_ch.len_utf8();
                    chars.next();
                }

                let value = u16::from_str_radix(&digits, 8).expect("octal digits are validated");
                if value > u16::from(u8::MAX) {
                    push_escape_overflow_warning(
                        e,
                        literal_subspan(literal_span, idx, end_offset),
                        "octal",
                    );
                }
                out.push(value as u8);
            }
            'x' => {
                let Some(&(_, next)) = chars.peek() else {
                    return Err(Rich::custom(
                        literal_subspan(literal_span, idx, escape_idx + escape.len_utf8()),
                        "invalid hex escape sequence",
                    ));
                };
                if !next.is_ascii_hexdigit() {
                    return Err(Rich::custom(
                        literal_subspan(literal_span, idx, escape_idx + escape.len_utf8()),
                        "invalid hex escape sequence",
                    ));
                }

                let mut value = 0u8;
                let mut overflowed = false;
                let mut end_offset = escape_idx + escape.len_utf8();

                while let Some(&(digit_idx, digit)) = chars.peek() {
                    if !digit.is_ascii_hexdigit() {
                        break;
                    }
                    let digit_value = digit.to_digit(16).expect("hex digit was validated") as u16;
                    overflowed |= u16::from(value) * 16 + digit_value > u16::from(u8::MAX);
                    value = value.wrapping_mul(16).wrapping_add(digit_value as u8);
                    end_offset = digit_idx + digit.len_utf8();
                    chars.next();
                }

                if overflowed {
                    push_escape_overflow_warning(
                        e,
                        literal_subspan(literal_span, idx, end_offset),
                        "hex",
                    );
                }
                out.push(value);
            }
            'u' | 'U' => {
                let digits = if escape == 'u' { 4 } else { 8 };
                let mut value = 0u32;
                let mut end_offset = escape_idx + escape.len_utf8();

                for _ in 0..digits {
                    let Some((digit_idx, digit)) = chars.next() else {
                        return Err(Rich::custom(
                            literal_subspan(literal_span, idx, end_offset),
                            "invalid universal character name",
                        ));
                    };
                    if !digit.is_ascii_hexdigit() {
                        return Err(Rich::custom(
                            literal_subspan(literal_span, idx, digit_idx + digit.len_utf8()),
                            "invalid universal character name",
                        ));
                    }
                    value = value
                        .checked_mul(16)
                        .and_then(|value| value.checked_add(digit.to_digit(16).unwrap()))
                        .expect("universal character values fit in u32");
                    end_offset = digit_idx + digit.len_utf8();
                }

                let Some(ch) = char::from_u32(value) else {
                    return Err(Rich::custom(
                        literal_subspan(literal_span, idx, end_offset),
                        "invalid universal character name",
                    ));
                };
                let mut buf = [0u8; 4];
                out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
            }
            _ => {
                return Err(Rich::custom(
                    literal_subspan(literal_span, idx, escape_idx + escape.len_utf8()),
                    "invalid escape sequence",
                ));
            }
        }
    }

    Ok(out)
}

// Helper function to parse integer suffixes
fn integer_suffix_parser<'src>() -> impl Parser<'src, &'src str, IntegerSuffix, LexerExtra<'src>> {
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
fn float_suffix_parser<'src>() -> impl Parser<'src, &'src str, FloatSuffix, LexerExtra<'src>> {
    just('f')
        .or(just('F'))
        .to(FloatSuffix::Float)
        .or(just('l').or(just('L')).to(FloatSuffix::Long))
        .or_not()
        .map(|opt| opt.unwrap_or(FloatSuffix::None))
}

pub fn lexer<'src>()
-> impl Parser<'src, &'src str, Vec<(Token, SimpleSpan<usize>)>, LexerExtra<'src>> {
    // ----- Comments -----
    let line_comment = just("//")
        .then(choice((just('!'), just('/'))))
        .not()
        .ignore_then(just("//"))
        .then(any().and_is(just('\n').not()).repeated())
        .to_slice()
        .ignored();
    let doc_comment = just("//")
        .ignore_then(choice((just('!'), just('/'))))
        .then(any().and_is(just('\n').not()).repeated())
        .to_slice()
        .map(|comment: &str| {
            let inner = comment.as_bytes()[2] == b'!';
            Token::DocComment {
                inner,
                text: comment[3..].to_owned(),
            }
        });

    let block_comment = just("/*")
        .then(any().and_is(just("*/").not()).repeated())
        .then(just("*/"))
        .to_slice()
        .ignored();

    let attribute = just("__attribute__")
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
        .to_slice();
    let transparent_union_attr = attribute
        .clone()
        .try_map(|attr: &str, span| {
            if attr.contains("__transparent_union__") {
                Ok(attr)
            } else {
                Err(Rich::custom(span, "not a transparent union attribute"))
            }
        })
        .to(Token::TransparentUnionAttr);
    let attributes = attribute
        .try_map(|attr: &str, span| {
            if attr.contains("__transparent_union__") {
                Err(Rich::custom(
                    span,
                    "transparent union attribute is not trivia",
                ))
            } else {
                Ok(attr)
            }
        })
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
    let hex_digits0 = one_of("0123456789abcdefABCDEF").repeated().to_slice();

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

    let hex_float = just("0x")
        .or(just("0X"))
        .then(choice((
            hex_digits
                .then(just('.').then(hex_digits0).or_not())
                .to_slice(),
            just('.').then(hex_digits).to_slice(),
        )))
        .then(
            one_of("pP")
                .then(just('+').or(just('-')).or_not())
                .then(text::digits(10).to_slice())
                .to_slice(),
        )
        .to_slice()
        .then(float_suffix_parser())
        .map(|(num, suffix)| Token::FloatLit(num.to_string(), suffix));

    // Octal integer (starting with 0)
    let octal_integer = just('0')
        .then(one_of("01234567").repeated().at_least(1))
        .to_slice()
        .then(integer_suffix_parser())
        .map(|(num, suffix)| Token::Integer(num.to_string(), suffix));

    // Float constants
    let decimal_float = text::int(10)
        .then(
            just('.')
                .ignore_then(text::digits(10).to_slice().or(empty().to("")))
                .or_not(),
        )
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
    let literal_prefix = choice((
        just("u8").to(StringLiteralPrefix::Utf8),
        just("u").to(StringLiteralPrefix::Utf16),
        just("U").to(StringLiteralPrefix::Utf32),
        just("L").to(StringLiteralPrefix::Wide),
    ))
    .or_not();
    let char_literal_prefix = literal_prefix.ignored().or_not();

    let char_content = choice((
        just('\\').ignore_then(any()).ignored(),
        none_of("'\\").ignored(),
    ))
    .repeated()
    .at_least(1)
    .to_slice()
    .try_map_with(|raw, e| decode_literal_bytes(raw, e.span(), e));

    let char_literal = char_literal_prefix
        .ignore_then(just('\''))
        .ignore_then(char_content)
        .then_ignore(just('\''))
        .map(Token::CharLit);

    let string_content = choice((
        just('\\').ignore_then(any()).ignored(),
        none_of("\"\\").ignored(),
    ))
    .repeated()
    .to_slice()
    .try_map_with(|raw, e| decode_literal_bytes(raw, e.span(), e));

    let string_literal = literal_prefix
        .then_ignore(just('"'))
        .then(string_content)
        .then_ignore(just('"'))
        .map(|(prefix, bytes)| {
            Token::StringLit(StringLiteral {
                prefix: prefix.unwrap_or(StringLiteralPrefix::None),
                bytes,
            })
        });

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
            "inline" | "__inline__" => Token::Inline,
            "__int128" | "int" => Token::Int,
            "long" => Token::Long,
            "register" => Token::Register,
            "restrict" | "__restrict" => Token::Restrict,
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

            _ => Token::Ident(ident),
        });

    // ----- Combined token parser -----
    let token = choice((
        preprocessor,
        hex_float,
        hex_integer,
        octal_integer,
        decimal_float,
        decimal_integer,
        char_literal,
        string_literal,
        doc_comment,
        transparent_union_attr,
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
