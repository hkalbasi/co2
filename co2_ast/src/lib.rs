use std::fmt::Display;

use itertools::Itertools;

mod diagnostic;
mod resolver;
mod transform;

// Type definitions
pub type Span = chumsky::span::SimpleSpan<usize>;
pub type Spanned<T> = (T, Span);

pub use chumsky::prelude::Rich;
pub use diagnostic::{print_errors_and_terminate, safe_range, take_errors};
pub use resolver::{StatelessResolver, TypeResolver};
pub use transform::{DoTransform, Transformable};

#[derive(Debug, Clone)]
pub enum Statement<R: TypeResolver> {
    Empty,
    Goto(Spanned<String>),
    Break,
    Continue,
    Switch {
        expr: Spanned<Expression<R>>,
        body: Box<Spanned<Statement<R>>>,
    },
    Case {
        expr: Spanned<Expression<R>>,
        statement: Box<Spanned<Statement<R>>>,
    },
    Default {
        statement: Box<Spanned<Statement<R>>>,
    },
    Label {
        name: Spanned<String>,
        statement: Box<Spanned<Statement<R>>>,
    },
    Return(Option<Spanned<Expression<R>>>),
    Expression(Spanned<Expression<R>>),
    Compound(Spanned<CompoundStatement<R>>),
    If {
        cond: Spanned<Expression<R>>,
        then_branch: Box<Spanned<Statement<R>>>,
        else_branch: Option<Box<Spanned<Statement<R>>>>,
    },
    While {
        cond: Spanned<Expression<R>>,
        body: Box<Spanned<Statement<R>>>,
    },
    DoWhile {
        body: Box<Spanned<Statement<R>>>,
        cond: Spanned<Expression<R>>,
    },
    For {
        init: Option<ForInit<R>>,
        cond: Option<Spanned<Expression<R>>>,
        post: Option<Spanned<Expression<R>>>,
        body: Box<Spanned<Statement<R>>>,
    },
}

#[derive(Debug, Clone)]
pub enum ForInit<R: TypeResolver> {
    Expression(Spanned<Expression<R>>),
    Declaration(Spanned<Declaration<R>>),
}

#[derive(Debug, Clone)]
pub enum Constant {
    Int(i128, IntegerSuffix),
    Float(f64),
    Char(char),
    String(String),
}

#[derive(Debug, Clone)]
pub enum Expression<R: TypeResolver> {
    Empty,
    Constant(Constant),
    Identifier(Spanned<R::ResolvedRustPath>),
    Field(Box<Spanned<Expression<R>>>, Spanned<String>),
    Arrow(Box<Spanned<Expression<R>>>, Spanned<String>),
    Subscript(Box<Spanned<Expression<R>>>, Box<Spanned<Expression<R>>>),
    Call {
        func: Box<Spanned<Expression<R>>>,
        params: Vec<Spanned<Expression<R>>>,
    },
    Update {
        expr: Box<Spanned<Expression<R>>>,
        op: UpdateOp,
        is_postfix: bool,
    },
    AssignWithOp {
        lhs: Box<Spanned<Expression<R>>>,
        op: BinOp,
        rhs: Box<Spanned<Expression<R>>>,
    },
    Cast {
        type_name: Box<TypeName<R>>,
        expr: Box<Spanned<Expression<R>>>,
    },
    SizeofType(Box<TypeName<R>>),
    Sizeof(Box<Spanned<Expression<R>>>),
    Offsetof {
        ty: Box<TypeName<R>>,
        field: String,
    },
    UnaryOp(UnaryOp, Box<Spanned<Expression<R>>>),
    BinOp(
        Box<Spanned<Expression<R>>>,
        BinOp,
        Box<Spanned<Expression<R>>>,
    ),
    Conditional {
        cond: Box<Spanned<Expression<R>>>,
        then_expr: Box<Spanned<Expression<R>>>,
        else_expr: Box<Spanned<Expression<R>>>,
    },
    CompoundLiteral {
        type_name: Box<TypeName<R>>,
        initializer: Box<Spanned<Initializer<R>>>,
    },
    GnuStatementExpr {
        body: Box<Spanned<CompoundStatement<R>>>,
    },
    VaStart {
        args: Box<Spanned<Expression<R>>>,
        last_param: Spanned<String>,
    },
    VaArg {
        args: Box<Spanned<Expression<R>>>,
        type_name: TypeName<R>,
    },
    VaEnd {
        args: Box<Spanned<Expression<R>>>,
    },
    GenericSelection {
        controlling: Box<Spanned<Expression<R>>>,
        associations: Vec<Spanned<GenericAssociation<R>>>,
    },
}

#[derive(Debug, Clone)]
pub enum GenericAssociation<R: TypeResolver> {
    Type {
        type_name: TypeName<R>,
        expr: Spanned<Expression<R>>,
    },
    Default {
        expr: Spanned<Expression<R>>,
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
    Comma,
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
pub struct CompoundStatement<R: TypeResolver> {
    pub statements: Vec<Spanned<StatementOrDeclaration<R>>>,
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
pub enum TypeSpecifier<R: TypeResolver> {
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
        specifier: Spanned<R::StructOrUnionIdentifier>,
    },
    Enum(Spanned<R::EnumIdentifier>),
    TypedefName(Spanned<R::ResolvedRustPath>),
}

#[derive(Debug, Clone)]
pub struct StructOrUnionField<R: TypeResolver> {
    pub specifiers: Vec<Spanned<SpecifierQualifier<R>>>,
    pub declarators: Vec<Spanned<StructDeclarator<R>>>,
}

#[derive(Debug, Clone)]
pub struct StructDeclarator<R: TypeResolver> {
    pub declarator: Spanned<Declarator<R>>,
    pub bits: Option<Spanned<String>>,
}

#[derive(Debug, Clone)]
pub enum StructOrUnionSpecifier<R: TypeResolver> {
    Defined {
        ident: Spanned<String>,
        fields: Vec<Spanned<StructOrUnionField<R>>>,
    },
    Declared {
        ident: Spanned<String>,
    },
    Anonymous {
        fields: Vec<Spanned<StructOrUnionField<R>>>,
    },
}

#[derive(Debug, Clone, Copy)]
pub enum StructOrUnionKind {
    Struct,
    Union,
}

#[derive(Debug, Clone)]
pub struct Enumerator<R: TypeResolver> {
    pub ident: Spanned<String>,
    pub value: Option<Spanned<Expression<R>>>,
}

#[derive(Debug, Clone)]
pub enum EnumSpecifier<R: TypeResolver> {
    Defined {
        ident: Spanned<String>,
        enumerators: Vec<Spanned<R::EnumeratorIdentifier>>,
    },
    Declared {
        ident: Spanned<String>,
    },
    Anonymous {
        enumerators: Vec<Spanned<R::EnumeratorIdentifier>>,
    },
}

#[derive(Debug, Clone, Copy)]
pub enum TypeQualifier {
    Const,
    Restrict,
    Volatile,
}

#[derive(Debug, Clone, Copy)]
pub enum FunctionSpecifier {
    Inline,
}

#[derive(Debug, Clone)]
pub enum SpecifierQualifier<R: TypeResolver> {
    TypeSpecifier(Spanned<TypeSpecifier<R>>),
    TypeQualifier(Spanned<TypeQualifier>),
}

#[derive(Debug, Clone)]
pub struct TypeName<R: TypeResolver> {
    pub specifier_qualifier_list: Vec<Spanned<SpecifierQualifier<R>>>,
    pub abstract_declarator: Option<Spanned<Declarator<R>>>,
}

#[derive(Debug, Clone)]
pub struct InitDeclarator<R: TypeResolver> {
    pub declarator: Spanned<Declarator<R>>,
    pub initializer: Option<Spanned<Initializer<R>>>,
}

#[derive(Debug, Clone)]
pub enum Designator<R: TypeResolver> {
    Subscript(Spanned<Expression<R>>),
    Field(Spanned<String>),
}

#[derive(Debug, Clone)]
pub struct InitializerItem<R: TypeResolver> {
    pub designators: Option<Vec<Spanned<Designator<R>>>>,
    pub initializer: Spanned<Initializer<R>>,
}

#[derive(Debug, Clone)]
pub enum Initializer<R: TypeResolver> {
    Expr(Spanned<Expression<R>>),
    List(Vec<Spanned<InitializerItem<R>>>),
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
    Offsetof,
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
    Generic,

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

    // Vararg tokens
    VaStart,
    VaArg,
    VaEnd,

    // GCC float constants
    BuiltinInf,
    BuiltinNan,

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
            Token::Offsetof => write!(f, "offsetof"),
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
            Token::Generic => write!(f, "_Generic"),

            Token::VaStart => write!(f, "va_start"),
            Token::VaArg => write!(f, "va_arg"),
            Token::VaEnd => write!(f, "va_end"),
            Token::BuiltinInf => write!(f, "__builtin_inf"),
            Token::BuiltinNan => write!(f, "__builtin_nan"),

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
pub enum StatementOrDeclaration<R: TypeResolver> {
    Declaration(Spanned<Declaration<R>>),
    Statement(Spanned<Statement<R>>),
}

#[derive(Debug, Clone)]
pub enum Declaration<R: TypeResolver> {
    FunctionDefinition {
        signature: FunctionDefinitionSignature<R>,
        body: Spanned<LazyCompoundStatement>,
    },
    Declaration {
        declaration_specifiers: Vec<Spanned<DeclarationSpecifier<R>>>,
        declarators: Vec<Spanned<InitDeclarator<R>>>,
    },
}

#[derive(Debug, Clone)]
pub enum FunctionDefinitionSignature<R: TypeResolver> {
    C {
        declaration_specifiers: Vec<Spanned<DeclarationSpecifier<R>>>,
        declarator: Spanned<Declarator<R>>,
    },
    Rust(RustFunctionSignature<R>),
}

impl<R: TypeResolver> FunctionDefinitionSignature<R> {
    pub fn ident(&self) -> Option<R::DeclarationIdent> {
        match self {
            FunctionDefinitionSignature::C { declarator, .. } => declarator.0.ident(),
            FunctionDefinitionSignature::Rust(sig) => Some(sig.name.0.clone()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RustFunctionSignature<R: TypeResolver> {
    pub name: Spanned<R::DeclarationIdent>,
    pub params: Vec<RustFunctionParam<R>>,
    pub ret_ty: Spanned<RustTy<R>>,
}

#[derive(Debug, Clone)]
pub struct RustFunctionParam<R: TypeResolver> {
    pub name: Spanned<R::DeclarationIdent>,
    pub ty: Spanned<RustTy<R>>,
}

#[derive(Debug, Clone)]
pub enum RustTy<R: TypeResolver> {
    Path(Spanned<R::ResolvedRustPath>),
    Tuple(Vec<Spanned<RustTy<R>>>),
    Ref {
        mutable: bool,
        inner: Box<Spanned<RustTy<R>>>,
    },
    Ptr {
        mutable: bool,
        inner: Box<Spanned<RustTy<R>>>,
    },
    Slice(Box<Spanned<RustTy<R>>>),
    Array {
        inner: Box<Spanned<RustTy<R>>>,
        len: Spanned<LazyRustConstExpr>,
    },
    BareFn {
        params: Vec<Spanned<RustTy<R>>>,
        ret_ty: Box<Spanned<RustTy<R>>>,
    },
    Never,
}

#[derive(Debug, Clone)]
pub struct LazyRustConstExpr {
    pub tokens: Vec<Spanned<Token>>,
}

impl LazyRustConstExpr {
    pub fn constant_len(&self) -> Option<u128> {
        if let [(token, _)] = &self.tokens[..] {
            match token {
                Token::Integer(text, suffix) if matches!(suffix, IntegerSuffix::None) => {
                    parse_unsigned_integer_constant(text).ok()
                }
                _ => None,
            }
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy)]
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
    pub fn is_unsized(&self) -> bool {
        self.tokens.len() == 2
    }

    pub fn constant_len(&self) -> Option<u128> {
        let mut len = None;
        if let [_, (token, _), _] = &self.tokens[..] {
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
                    text.parse::<u128>().ok()
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
pub struct ParameterList<R: TypeResolver> {
    pub parameters: Vec<(
        Vec<Spanned<DeclarationSpecifier<R>>>,
        Spanned<Declarator<R>>,
    )>,
    pub ellipsis: bool,
    pub empty_is_variadic: bool,
}

impl<R: TypeResolver> ParameterList<R> {
    pub fn empty_params(&self) -> bool {
        match self.parameters.as_slice() {
            [] => true,
            [param] => {
                let declarator_is_abstract = matches!(param.1.0, Declarator::Abstract);
                let has_void_type = param.0.iter().any(|(spec, _)| {
                    matches!(
                        spec,
                        DeclarationSpecifier::TypeSpecifier((TypeSpecifier::Void, _))
                    )
                });
                declarator_is_abstract && has_void_type
            }
            _ => false,
        }
    }

    pub fn effective_ellipsis(&self) -> bool {
        self.ellipsis || (self.empty_is_variadic && self.parameters.is_empty())
    }
}

#[derive(Debug, Clone)]
pub enum Declarator<R: TypeResolver> {
    Abstract,
    Identifier(Spanned<R::DeclarationIdent>),
    FunctionDeclarator {
        declarator: Box<Spanned<Declarator<R>>>,
        param_list: ParameterList<R>,
    },
    PointerDeclarator {
        declarator: Box<Spanned<Declarator<R>>>,
        qualifiers: Vec<Spanned<TypeQualifier>>,
    },
    ArrayDeclarator {
        declarator: Box<Spanned<Declarator<R>>>,
        subscription: Spanned<R::SubscriptionIdentifier>,
    },
}

impl<R: TypeResolver> Declarator<R> {
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

    pub fn ident(&self) -> Option<R::DeclarationIdent> {
        match self {
            Declarator::Abstract => None,
            Declarator::Identifier(ident) => Some(ident.0.clone()),
            Declarator::FunctionDeclarator { declarator, .. }
            | Declarator::PointerDeclarator { declarator, .. }
            | Declarator::ArrayDeclarator { declarator, .. } => declarator.0.ident(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum DeclarationSpecifier<R: TypeResolver> {
    TypeSpecifier(Spanned<TypeSpecifier<R>>),
    TypeQualifier(Spanned<TypeQualifier>),
    StorageSpecifier(Spanned<StorageClassSpecifier>),
    FunctionSpecifier(Spanned<FunctionSpecifier>),
}

impl<R: TypeResolver> DeclarationSpecifier<R> {
    pub fn is_typedef(&self) -> bool {
        match self {
            DeclarationSpecifier::TypeSpecifier(_) => false,
            DeclarationSpecifier::TypeQualifier(_) => false,
            DeclarationSpecifier::StorageSpecifier(c) => {
                matches!(c.0, StorageClassSpecifier::Typedef)
            }
            DeclarationSpecifier::FunctionSpecifier(_) => false,
        }
    }

    pub fn is_extern(&self) -> bool {
        match self {
            DeclarationSpecifier::TypeSpecifier(_) => false,
            DeclarationSpecifier::TypeQualifier(_) => false,
            DeclarationSpecifier::StorageSpecifier(c) => {
                matches!(c.0, StorageClassSpecifier::Extern)
            }
            DeclarationSpecifier::FunctionSpecifier(_) => false,
        }
    }

    pub fn is_static(&self) -> bool {
        match self {
            DeclarationSpecifier::TypeSpecifier(_) => false,
            DeclarationSpecifier::TypeQualifier(_) => false,
            DeclarationSpecifier::StorageSpecifier(c) => {
                matches!(c.0, StorageClassSpecifier::Static)
            }
            DeclarationSpecifier::FunctionSpecifier(_) => false,
        }
    }
}

#[derive(Debug)]
pub struct TranslationUnit<R: TypeResolver> {
    pub rust_use_items: Vec<Spanned<UseItem>>,
    pub items: Vec<Spanned<Declaration<R>>>,
}

#[derive(Debug, Clone)]
pub struct UseItem {
    pub path: Vec<Spanned<String>>,
}

pub fn parse_unsigned_integer_constant(text: &str) -> Result<u128, String> {
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        return u128::from_str_radix(hex, 16).map_err(|e| e.to_string());
    }

    if text.len() > 1
        && let Some(octal) = text.strip_prefix('0')
    {
        return u128::from_str_radix(octal, 8).map_err(|e| e.to_string());
    }

    text.parse::<u128>().map_err(|e| e.to_string())
}
