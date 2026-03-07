use std::fmt::Display;

use itertools::Itertools;

mod diagnostic;

// Type definitions
pub type Span = chumsky::span::SimpleSpan<usize>;
pub type Spanned<T> = (T, Span);

pub use chumsky::prelude::Rich;
pub use diagnostic::{print_errors_and_terminate, take_errors};

#[derive(Debug, Clone)]
pub enum Statement {
    Empty,
    Goto(Spanned<String>),
    Break,
    Continue,
    Switch {
        expr: Spanned<Expression>,
        body: Box<Spanned<Statement>>,
    },
    Case {
        expr: Spanned<Expression>,
        statement: Box<Spanned<Statement>>,
    },
    Default {
        statement: Box<Spanned<Statement>>,
    },
    Label {
        name: Spanned<String>,
        statement: Box<Spanned<Statement>>,
    },
    Return(Option<Spanned<Expression>>),
    Expression(Spanned<Expression>),
    Compound(Spanned<CompoundStatement>),
    If {
        cond: Spanned<Expression>,
        then_branch: Box<Spanned<Statement>>,
        else_branch: Option<Box<Spanned<Statement>>>,
    },
    While {
        cond: Spanned<Expression>,
        body: Box<Spanned<Statement>>,
    },
    DoWhile {
        body: Box<Spanned<Statement>>,
        cond: Spanned<Expression>,
    },
    For {
        init: Option<Spanned<Expression>>,
        cond: Option<Spanned<Expression>>,
        post: Option<Spanned<Expression>>,
        body: Box<Spanned<Statement>>,
    },
}


#[derive(Debug, Clone)]
pub enum Constant {
    Int(i64, IntegerSuffix),
    Float(f64),
    Char(char),
    String(String),
}

#[derive(Debug, Clone)]
pub enum Expression {
    Empty,
    Constant(Constant),
    Identifier(Spanned<RustPath>),
    Field(Box<Spanned<Expression>>, Spanned<String>),
    Arrow(Box<Spanned<Expression>>, Spanned<String>),
    Subscript(Box<Spanned<Expression>>, Box<Spanned<Expression>>),
    Call {
        func: Box<Spanned<Expression>>,
        params: Vec<Spanned<Expression>>,
    },
    Update {
        expr: Box<Spanned<Expression>>,
        op: UpdateOp,
        is_postfix: bool,
    },
    AssignWithOp {
        lhs: Box<Spanned<Expression>>,
        op: BinOp,
        rhs: Box<Spanned<Expression>>,
    },
    Cast {
        type_name: Box<TypeName>,
        expr: Box<Spanned<Expression>>,
    },
    SizeofType(Box<TypeName>),
    Sizeof(Box<Spanned<Expression>>),
    UnaryOp(UnaryOp, Box<Spanned<Expression>>),
    BinOp(Box<Spanned<Expression>>, BinOp, Box<Spanned<Expression>>),
    Conditional {
        cond: Box<Spanned<Expression>>,
        then_expr: Box<Spanned<Expression>>,
        else_expr: Box<Spanned<Expression>>,
    },
    CompoundLiteral {
        type_name: Box<TypeName>,
        initializer: Box<Spanned<Initializer>>,
    },
    GnuStatementExpr {
        body: Box<Spanned<CompoundStatement>>,
    },
}

#[derive(Debug, Clone, Copy)]
pub enum UpdateOp {
    Inc,
    Dec,
}

#[derive(Debug, Clone, Copy)]
pub enum UnaryOp {
    Not,
    Com,
    AddrOf,
    Deref,
    Plus,
    Minus,
}

#[derive(Debug, Clone, Copy)]
pub enum BinOp {
    Assign,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Or,
    And,
    BitOr,
    BitXor,
    BitAnd,
    Eq,
    Lt,
    Le,
    Ne,
    Ge,
    Gt,
    Shl,
    Shr,
}


#[derive(Debug, Clone)]
pub struct LazyCompoundStatement {
    pub tokens: Spanned<Vec<Spanned<Token>>>,
}

#[derive(Debug, Clone)]
pub struct CompoundStatement {
    pub statements: Vec<Spanned<StatementOrDeclaration>>,
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

#[derive(Debug, Clone)]
pub struct RustPath {
    pub segments: Vec<Spanned<RustPathSegment>>,
}


#[derive(Debug, Clone)]
pub enum RustPathSegment {
    Ident(String),
    Generics(Vec<Spanned<RustPath>>),
}

impl RustPath {
    pub fn from_ident((ident, span): Spanned<String>) -> RustPath {
        RustPath {
            segments: vec![(RustPathSegment::Ident(ident), span)],
        }
    }

    pub fn decompose(&self) -> (RustPath, Vec<RustPath>) {
        let mut base = self.clone();
        if let Some((RustPathSegment::Generics(_), _)) = base.segments.last() {
            let Some((RustPathSegment::Generics(last), _)) = base.segments.pop() else {
                unreachable!();
            };
            (base, last.into_iter().map(|x| x.0.clone()).collect())
        } else {
            (base, vec![])
        }
    }

    pub fn to_pretty(&self) -> String {
        self.segments
            .iter()
            .map(|seg| match &seg.0 {
                RustPathSegment::Ident(s) => s.clone(),
                RustPathSegment::Generics(parts) => {
                    let inner = parts
                        .iter()
                        .map(|p| p.0.to_pretty())
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("<{inner}>")
                }
            })
            .collect::<Vec<_>>()
            .join("::")
    }
}

impl Display for RustPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            &self
                .segments
                .iter()
                .map(|x| match &x.0 {
                    RustPathSegment::Ident(ident) => ident.clone(),
                    RustPathSegment::Generics(rust_paths) => {
                        format!(
                            "<{}>",
                            rust_paths.iter().map(|x| x.0.to_string()).join(", ")
                        )
                    }
                })
                .join("::"),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeQueryResult {
    Type,
    Expr,
    Unsure,
}

#[derive(Debug, Clone)]
pub enum TypeSpecifier {
    Int,
    Bool,
    Void,
    Char,
    Short,
    Long,
    Float,
    Double,
    Signed,
    Unsigned,
    StructOrUnion {
        kind: StructOrUnionKind,
        specifier: StructOrUnionSpecifier,
    },
    Enum(EnumSpecifier),
    TypedefName(Spanned<RustPath>),
}

#[derive(Debug, Clone)]
pub struct StructOrUnionField {
    pub specifiers: Vec<Spanned<TypeSpecifier>>,
    pub declarators: Vec<Spanned<StructDeclarator>>,
}

#[derive(Debug, Clone)]
pub struct StructDeclarator {
    pub declarator: Spanned<Declarator>,
    pub bits: Option<Spanned<String>>,
}

#[derive(Debug, Clone)]
pub enum StructOrUnionSpecifier {
    Defined {
        ident: Spanned<String>,
        fields: Vec<Spanned<StructOrUnionField>>,
    },
    Declared {
        ident: Spanned<String>,
    },
    Anonymous {
        fields: Vec<Spanned<StructOrUnionField>>,
    },
}

#[derive(Debug, Clone, Copy)]
pub enum StructOrUnionKind {
    Struct,
    Union,
}

#[derive(Debug, Clone)]
pub struct Enumerator {
    pub ident: Spanned<String>,
    pub value: Option<Spanned<Expression>>,
}

#[derive(Debug, Clone)]
pub enum EnumSpecifier {
    Defined {
        ident: Spanned<String>,
        enumerators: Vec<Spanned<Enumerator>>,
    },
    Declared {
        ident: Spanned<String>,
    },
    Anonymous {
        enumerators: Vec<Spanned<Enumerator>>,
    },
}

#[derive(Debug, Clone)]
pub enum TypeQualifier {
    Const,
    Restrict,
    Volatile,
}

#[derive(Debug, Clone)]
pub enum SpecifierQualifier {
    TypeSpecifier(Spanned<TypeSpecifier>),
    TypeQualifier(Spanned<TypeQualifier>),
}

#[derive(Debug, Clone)]
pub struct TypeName {
    pub specifier_qualifier_list: Vec<Spanned<SpecifierQualifier>>,
    pub abstract_declarator: Option<Spanned<Declarator>>,
}


#[derive(Debug, Clone)]
pub struct InitDeclarator {
    pub declarator: Spanned<Declarator>,
    pub initializer: Option<Spanned<Initializer>>,
}

#[derive(Debug, Clone)]
pub enum Designator {
    Subscript(Spanned<Expression>),
    Field(Spanned<String>),
}

#[derive(Debug, Clone)]
pub struct InitializerItem {
    pub designators: Option<Vec<Spanned<Designator>>>,
    pub initializer: Spanned<Initializer>,
}

#[derive(Debug, Clone)]
pub enum Initializer {
    Expr(Spanned<Expression>),
    List(Vec<Spanned<InitializerItem>>),
}


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
    Atomic,
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
pub enum FloatSuffix {
    None,
    Float, // f or F
    Long,  // l or L
}

impl Display for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
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
            Token::Atomic => write!(f, "_Atomic"),
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

#[derive(Debug, Clone)]
pub enum StatementOrDeclaration {
    Declaration(Spanned<Declaration>),
    Statement(Spanned<Statement>),
}

#[derive(Debug, Clone)]
pub enum Declaration {
    FunctionDefinition {
        declaration_specifiers: Vec<Spanned<DeclarationSpecifier>>,
        declarator: Spanned<Declarator>,
        body: Spanned<LazyCompoundStatement>,
    },
    Declaration {
        declaration_specifiers: Vec<Spanned<DeclarationSpecifier>>,
        declarators: Vec<Spanned<InitDeclarator>>,
    },
}

#[derive(Debug, Clone)]
pub enum StorageClassSpecifier {
    Typedef,
    Extern,
    Static,
    Atomic,
    ThreadLocal,
    Auto,
    Register,
}

#[derive(Debug, Clone)]
pub struct LazySubscription {
    pub tokens: Vec<Spanned<Token>>,
}

impl LazySubscription {
    pub fn constant_len(&self) -> Option<u64> {
        let mut len = None;
        for (token, _) in &self.tokens {
            let next = match token {
                Token::Integer(text, suffix) => {
                    if !matches!(suffix, IntegerSuffix::None) {
                        return None;
                    }
                    parse_unsigned_integer_constant(text).ok()
                }
                Token::FloatLit(text, suffix) => {
                    if !matches!(suffix, FloatSuffix::None) {
                        return None;
                    }
                    if !text.chars().all(|c| c.is_ascii_digit()) {
                        return None;
                    }
                    text.parse::<u64>().ok()
                }
                _ => None,
            };
            if let Some(parsed) = next {
                if len.is_some() {
                    return None;
                }
                len = Some(parsed);
            }
        }
        len
    }
}

#[derive(Debug, Clone)]
pub struct ParameterList {
    pub parameters: Vec<(Vec<Spanned<DeclarationSpecifier>>, Spanned<Declarator>)>,
    pub ellipsis: bool,
}

#[derive(Debug, Clone)]
pub enum Declarator {
    Abstract,
    Identifier(Spanned<String>),
    FunctionDeclarator {
        declarator: Box<Spanned<Declarator>>,
        param_list: ParameterList,
    },
    PointerDeclarator {
        declarator: Box<Spanned<Declarator>>,
        qualifiers: Vec<Spanned<TypeQualifier>>,
    },
    ArrayDeclarator {
        declarator: Box<Spanned<Declarator>>,
        subscription: Spanned<LazySubscription>,
    },
}

impl Declarator {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Declarator::Identifier(_) | Declarator::Abstract)
    }

    pub fn is_function(&self) -> bool {
        match self {
            Declarator::FunctionDeclarator { declarator, .. } => {
                if declarator.0.is_terminal() {
                    true
                } else {
                    declarator.0.is_function()
                }
            }
            Declarator::Identifier(_) | Declarator::Abstract => false,
            Declarator::PointerDeclarator { declarator, .. } => declarator.0.is_function(),
            Declarator::ArrayDeclarator { declarator, .. } => declarator.0.is_function(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum DeclarationSpecifier {
    TypeSpecifier(Spanned<TypeSpecifier>),
    TypeQualifier(Spanned<TypeQualifier>),
    StorageSpecifier(Spanned<StorageClassSpecifier>),
}

#[derive(Debug)]
pub struct TranslationUnit {
    pub rust_use_items: Vec<Spanned<UseItem>>,
    pub items: Vec<Spanned<Declaration>>,
}

#[derive(Debug, Clone)]
pub struct UseItem {
    pub path: Vec<Spanned<String>>,
}

pub trait TypeResolver {
    fn classify_path(&self, path: &RustPath) -> TypeQueryResult;
}

pub struct AllowAllTypes;

impl TypeResolver for AllowAllTypes {
    fn classify_path(&self, _path: &RustPath) -> TypeQueryResult {
        TypeQueryResult::Type
    }
}

pub fn parse_unsigned_integer_constant(text: &str) -> Result<u64, String> {
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        return u64::from_str_radix(hex, 16).map_err(|e| e.to_string());
    }

    if text.len() > 1
        && let Some(octal) = text.strip_prefix('0')
    {
        return u64::from_str_radix(octal, 8).map_err(|e| e.to_string());
    }

    text.parse::<u64>().map_err(|e| e.to_string())
}
