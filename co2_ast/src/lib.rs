#![feature(rustc_private)]

extern crate rustc_data_structures;

use std::{
    ascii::escape_default,
    borrow::Cow,
    fmt::{self, Display},
};

use itertools::Itertools;

mod diagnostic;
mod pretty;
mod resolver;
mod span;
mod transform;

pub use diagnostic::{
    DiagnosticAbort, DiagnosticSpan, Rich, SourceMap, diagnostic_error_count,
    diagnostics_were_emitted, emit_errors, emit_errors_and_terminate, emit_warnings,
    is_diagnostic_abort, panic_with_diagnostic_abort, print_errors_and_terminate,
    reset_diagnostic_state, safe_range, set_diagnostic_base_path, set_force_json_diagnostics,
    set_source_map, take_errors,
};
pub use pretty::{PrettyConfig, PrettyPrint, PrettyPrinter, pretty_print_compound};
pub use resolver::{StatelessResolver, TypeResolver};
pub use span::{FileId, Span, SpanData, Spanned};
pub use transform::{DoTransform, Transformable};

#[derive(Debug, Clone)]
pub enum Statement<R: TypeResolver> {
    Empty,
    Goto(Spanned<String>),
    IndirectGoto(Spanned<Expression<R>>),
    Break,
    BreakCo2,
    Continue,
    Switch {
        expr: Spanned<Expression<R>>,
        body: Box<Spanned<Statement<R>>>,
    },
    Case {
        expr: Spanned<Expression<R>>,
        statement: Box<Spanned<Statement<R>>>,
    },
    CaseRange {
        lo: Spanned<Expression<R>>,
        hi: Spanned<Expression<R>>,
        statement: Box<Spanned<Statement<R>>>,
    },
    Default {
        keyword_span: Span,
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
    Bool(bool),
    Float(f64, FloatSuffix),
    Imaginary(f64, FloatSuffix),
    Char(u32, CharPrefix),
    String(StringLiteral),
}

#[derive(Debug, Clone)]
pub enum OffsetofMember<R: TypeResolver> {
    Field(Spanned<String>),
    Index(Box<Spanned<Expression<R>>>),
}

/// The old-C null-pointer trick `&((T *)0)->path...`: the cast pointee type
/// plus the member path (first segment always a field), cloned from the AST.
pub struct NullTrick<R: TypeResolver> {
    pub pointee: TypeName<R>,
    pub designator: Vec<OffsetofMember<R>>,
}

/// Matches `&(null-pointer-cast)->field(.field | [index])*` — the pre-`offsetof`
/// idiom for computing a member offset. Returns `None` for anything else
/// (in particular a `->` past the first one is a real dereference, not an offset).
pub fn match_null_trick<R: TypeResolver>(operand: &Spanned<Expression<R>>) -> Option<NullTrick<R>> {
    let Expression::UnaryOp(UnaryOp::AddrOf, inner) = &operand.0 else {
        return None;
    };
    let mut rev: Vec<OffsetofMember<R>> = Vec::new();
    let mut node = &inner.0;
    loop {
        match node {
            Expression::Field(base, name) => {
                rev.push(OffsetofMember::Field(name.clone()));
                node = &base.0;
            }
            Expression::Subscript(base, idx) => {
                rev.push(OffsetofMember::Index(idx.clone()));
                node = &base.0;
            }
            Expression::Arrow(base, name) => {
                let Expression::Cast { type_name, expr } = &base.0 else {
                    return None;
                };
                let mut core = &expr.0;
                while let Expression::Cast { expr, .. } = core {
                    core = &expr.0;
                }
                if !matches!(core, Expression::Constant(Constant::Int(0, _))) {
                    return None;
                }
                rev.push(OffsetofMember::Field(name.clone()));
                rev.reverse();
                return Some(NullTrick {
                    pointee: (**type_name).clone(),
                    designator: rev,
                });
            }
            _ => return None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Expression<R: TypeResolver> {
    Empty,
    Constant(Constant),
    Identifier(Spanned<R::ResolvedRustPath>),
    LabelAddress(Spanned<String>),
    Field(Box<Spanned<Expression<R>>>, Spanned<String>),
    Arrow(Box<Spanned<Expression<R>>>, Spanned<String>),
    MethodCall {
        receiver: Box<Spanned<Expression<R>>>,
        method: Spanned<String>,
        generics: Vec<Spanned<RustTy<StatelessResolver>>>,
        params: Vec<Spanned<Expression<R>>>,
    },
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
    AlignofType(Box<TypeName<R>>),
    Alignof(Box<Spanned<Expression<R>>>),
    CountofType(Box<TypeName<R>>),
    Countof(Box<Spanned<Expression<R>>>),
    Offsetof {
        ty: Box<TypeName<R>>,
        designator: Vec<OffsetofMember<R>>,
    },
    UnaryOp(UnaryOp, Box<Spanned<Expression<R>>>),
    BinOp(
        Box<Spanned<Expression<R>>>,
        BinOp,
        Box<Spanned<Expression<R>>>,
    ),
    Conditional {
        cond: Box<Spanned<Expression<R>>>,
        then_expr: Option<Box<Spanned<Expression<R>>>>,
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
    VaCopy {
        dest: Box<Spanned<Expression<R>>>,
        src: Box<Spanned<Expression<R>>>,
    },
    VaEnd {
        args: Box<Spanned<Expression<R>>>,
    },
    GenericSelection {
        controlling: Box<Spanned<Expression<R>>>,
        associations: Vec<Spanned<GenericAssociation<R>>>,
    },
    BuiltinConstantP {
        expr: Box<Spanned<Expression<R>>>,
    },
    BuiltinTypesCompatibleP {
        ty1: Box<TypeName<R>>,
        ty2: Box<TypeName<R>>,
    },
    BuiltinComplex {
        re: Box<Spanned<Expression<R>>>,
        im: Box<Spanned<Expression<R>>>,
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
    /// Unsuffixed decimal integer constant: per C11 §6.4.4.1 only the
    /// signed types { int, long, long long } are considered. Unlike
    /// `None` (hex/octal, which may also pick unsigned types), the type
    /// of a decimal constant never silently becomes unsigned.
    NoneDecimal,
    Unsigned,
    Long,
    LongLong,
    UnsignedLong,
    UnsignedLongLong,
    Usize,
    Isize,
    U8,
    U16,
    U32,
    U64,
    U128,
    I8,
    I16,
    I32,
    I64,
    I128,
}

#[derive(Debug, Clone)]
pub struct RustPath<R: TypeResolver> {
    pub segments: Vec<Spanned<RustPathSegment<R>>>,
}

#[derive(Debug, Clone)]
pub enum RustPathSegment<R: TypeResolver> {
    Ident(String),
    Generics(Vec<Spanned<RustTy<R>>>),
    Qualified {
        type_segments: Vec<Spanned<RustPathSegment<R>>>,
        trait_segments: Vec<Spanned<RustPathSegment<R>>>,
    },
}

impl<R: TypeResolver> RustPath<R> {
    pub fn from_ident((ident, span): Spanned<String>) -> Self {
        RustPath {
            segments: vec![(RustPathSegment::Ident(ident), span)],
        }
    }

    pub fn decompose(&self) -> (Self, Vec<Spanned<RustTy<R>>>) {
        let mut base = self.clone();
        if let Some((RustPathSegment::Generics(_), _)) = base.segments.last() {
            let Some((RustPathSegment::Generics(last), _)) = base.segments.pop() else {
                unreachable!();
            };
            (base, last)
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
                        .map(|p| rust_ty_to_pretty(&p.0))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("<{inner}>")
                }
                RustPathSegment::Qualified {
                    type_segments,
                    trait_segments,
                } => {
                    let type_path = RustPath {
                        segments: type_segments.clone(),
                    };
                    let trait_path = RustPath {
                        segments: trait_segments.clone(),
                    };
                    format!("<{} as {}>", type_path.to_pretty(), trait_path.to_pretty())
                }
            })
            .collect::<Vec<_>>()
            .join("::")
    }
}

impl<R: TypeResolver> Display for RustPath<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            &self
                .segments
                .iter()
                .map(|x| match &x.0 {
                    RustPathSegment::Ident(ident) => ident.clone(),
                    RustPathSegment::Generics(rust_tys) => format!(
                        "<{}>",
                        rust_tys.iter().map(|x| rust_ty_to_pretty(&x.0)).join(", ")
                    ),
                    RustPathSegment::Qualified {
                        type_segments,
                        trait_segments,
                    } => {
                        let type_path = RustPath {
                            segments: type_segments.clone(),
                        };
                        let trait_path = RustPath {
                            segments: trait_segments.clone(),
                        };
                        format!("<{} as {}>", type_path, trait_path)
                    }
                })
                .join("::"),
        )
    }
}

fn rust_ty_to_pretty<R: TypeResolver>(ty: &RustTy<R>) -> String {
    match ty {
        RustTy::Path((path, _)) => R::pretty_resolved_path(path),
        RustTy::Tuple(elems) => {
            let inner = elems
                .iter()
                .map(|elem| rust_ty_to_pretty(&elem.0))
                .collect::<Vec<_>>()
                .join(", ");
            format!("({inner})")
        }
        RustTy::Ref {
            lifetime,
            mutable,
            inner,
        } => {
            let lifetime = lifetime
                .as_ref()
                .map_or_else(String::new, |(name, _)| format!("'{name} "));
            let mutability = if *mutable { "mut " } else { "" };
            format!("&{lifetime}{mutability}{}", rust_ty_to_pretty(&inner.0))
        }
        RustTy::Ptr { mutable, inner } => {
            let mutability = if *mutable { "mut " } else { "const " };
            format!("*{mutability}{}", rust_ty_to_pretty(&inner.0))
        }
        RustTy::Slice(inner) => format!("[{}]", rust_ty_to_pretty(&inner.0)),
        RustTy::Array { inner, len } => {
            let len = len
                .0
                .constant_len()
                .map(|n| n.to_string())
                .unwrap_or_else(|| {
                    len.0
                        .tokens
                        .iter()
                        .map(|(token, _)| token.to_string())
                        .collect::<Vec<_>>()
                        .join(" ")
                });
            format!("[{}; {len}]", rust_ty_to_pretty(&inner.0))
        }
        RustTy::BareFn { params, ret_ty } => {
            let params = params
                .iter()
                .map(|param| rust_ty_to_pretty(&param.0))
                .collect::<Vec<_>>()
                .join(", ");
            format!("fn({params}) -> {}", rust_ty_to_pretty(&ret_ty.0))
        }
        RustTy::Never => "!".to_owned(),
        RustTy::Wild => "_".to_owned(),
        RustTy::Lifetime((name, _)) => format!("'{name}"),
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
    Int128,
    Bool,
    Void,
    Char,
    Short,
    Long,
    Float,
    Double,
    Complex,
    Signed,
    Unsigned,
    Float16,
    Float32,
    Float64,
    Float128,
    StructOrUnion {
        kind: StructOrUnionKind,
        specifier: Spanned<R::StructOrUnionIdentifier>,
    },
    Enum(Spanned<R::EnumIdentifier>),
    TypedefName(Spanned<R::ResolvedRustPath>),
    TypeofType(Box<TypeName<R>>),
    TypeofExpr(Box<Spanned<Expression<R>>>),
    Alignas,
}

#[derive(Debug, Clone)]
pub struct StructOrUnionField<R: TypeResolver> {
    pub specifiers: Vec<Spanned<SpecifierQualifier<R>>>,
    pub declarators: Vec<Spanned<StructDeclarator<R>>>,
}

#[derive(Debug, Clone)]
pub struct StructDeclarator<R: TypeResolver> {
    pub declarator: Spanned<Declarator<R>>,
    pub bits: Option<Spanned<Expression<R>>>,
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
        underlying_type: Option<TypeName<R>>,
        enumerators: Vec<Spanned<R::EnumeratorIdentifier>>,
    },
    Declared {
        ident: Spanned<String>,
        underlying_type: Option<TypeName<R>>,
    },
    Anonymous {
        underlying_type: Option<TypeName<R>>,
        enumerators: Vec<Spanned<R::EnumeratorIdentifier>>,
    },
}

#[derive(Debug, Clone, Copy)]
pub enum TypeQualifier {
    Const,
    Constexpr,
    Restrict,
    Volatile,
    Atomic,
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
    pub is_transparent_union: bool,
}

#[derive(Debug, Clone)]
pub enum Designator<R: TypeResolver> {
    Subscript(Spanned<Expression<R>>),
    Range(Spanned<Expression<R>>, Spanned<Expression<R>>),
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
    Constexpr,
    Complex,
    Continue,
    Default,
    Do,
    Double,
    Else,
    Enum,
    Extern,
    Float,
    Float16,
    Float32,
    Float64,
    Float128,
    Float32x,
    Float64x,
    Float128x,
    GnuFloat128,
    For,
    Goto,
    If,
    Inline,
    Int,
    Int128,
    Long,
    Register,
    Restrict,
    Return,
    Short,
    Signed,
    Sizeof,
    Typeof,
    Alignof,
    Alignas,
    Countof,
    Offsetof,
    Static,
    Atomic,
    ThreadLocal,
    Struct,
    Switch,
    Typedef,
    Union,
    Unsigned,
    Void,
    Volatile,
    While,
    Generic,
    StaticAssert,

    // Identifiers
    Ident(String),

    // Documentation comments
    DocComment { inner: bool, text: String },

    // Constants
    Integer(String, IntegerSuffix),
    BoolLit(bool),
    FloatLit(String, FloatSuffix),
    ImaginaryLit(String, FloatSuffix),
    CharLit(Vec<u8>, CharPrefix),
    StringLit(StringLiteral),

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
    VaCopy,
    VaEnd,

    // GCC float constants
    BuiltinInf,
    BuiltinNan,

    // GCC builtin predicates
    BuiltinConstantP,
    BuiltinTypesCompatibleP,
    BuiltinComplex,

    // GCC attributes
    TransparentUnionAttr,

    // Special
    Ellipsis, // ...
    Hash,     // #
    HashHash, // ##

    // Lifetimes
    Lifetime(String), // 'static, 'a, etc.
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum FloatSuffix {
    None,
    Float, // f or F
    Long,  // l or L
    F16,   // f16 or F16
    F32,   // f32 or F32
    F64,   // f64 or F64
    F128,  // f128 or F128
    F32x,  // f32x or F32x
    F64x,  // f64x or F64x
    F128x, // f128x or F128x
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CharPrefix {
    None,
    Utf8,
    Utf16,
    Utf32,
    Wide,
}

impl CharPrefix {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Utf8 => "u8",
            Self::Utf16 => "u",
            Self::Utf32 => "U",
            Self::Wide => "L",
        }
    }

    pub fn element_size(self) -> usize {
        match self {
            Self::None | Self::Utf8 => 1,
            Self::Utf16 => 2,
            Self::Utf32 | Self::Wide => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StringLiteralPrefix {
    None,
    Str,
    Utf8,
    Utf16,
    Utf32,
    Wide,
}

impl StringLiteralPrefix {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Str => "s",
            Self::Utf8 => "u8",
            Self::Utf16 => "u",
            Self::Utf32 => "U",
            Self::Wide => "L",
        }
    }

    pub fn element_size(self) -> usize {
        match self {
            Self::None | Self::Str | Self::Utf8 => 1,
            Self::Utf16 => 2,
            Self::Utf32 | Self::Wide => 4,
        }
    }

    pub fn is_wide(self) -> bool {
        matches!(self, Self::Utf16 | Self::Utf32 | Self::Wide)
    }
}

/// A string literal. The variant is the prefix: narrow prefixes (`"…"`,
/// `s"…"`, `u8"…"`) store raw bytes, while wide prefixes (`u"…"`, `U"…"`,
/// `L"…"`) store decoded code units — each escape is its own code unit (never
/// UTF-8-decoded), while raw source bytes are UTF-8-decoded into one code unit
/// per code point.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum StringLiteral {
    None(Vec<u8>),
    Str(Vec<u8>),
    Utf8(Vec<u8>),
    Utf16(Vec<u32>),
    Utf32(Vec<u32>),
    Wide(Vec<u32>),
}

impl StringLiteral {
    pub fn prefix(&self) -> StringLiteralPrefix {
        match self {
            StringLiteral::None(_) => StringLiteralPrefix::None,
            StringLiteral::Str(_) => StringLiteralPrefix::Str,
            StringLiteral::Utf8(_) => StringLiteralPrefix::Utf8,
            StringLiteral::Utf16(_) => StringLiteralPrefix::Utf16,
            StringLiteral::Utf32(_) => StringLiteralPrefix::Utf32,
            StringLiteral::Wide(_) => StringLiteralPrefix::Wide,
        }
    }

    pub fn is_wide(&self) -> bool {
        matches!(
            self,
            StringLiteral::Utf16(_) | StringLiteral::Utf32(_) | StringLiteral::Wide(_)
        )
    }

    pub fn element_size(&self) -> usize {
        self.prefix().element_size()
    }

    pub fn code_unit_len(&self) -> usize {
        match self {
            StringLiteral::None(bytes) | StringLiteral::Str(bytes) | StringLiteral::Utf8(bytes) => {
                bytes.len()
            }
            StringLiteral::Utf16(units)
            | StringLiteral::Utf32(units)
            | StringLiteral::Wide(units) => units.len(),
        }
    }

    pub fn nul_terminated_len(&self) -> usize {
        self.code_unit_len() + 1
    }

    pub fn storage_size(&self) -> usize {
        self.nul_terminated_len() * self.element_size()
    }

    /// Raw bytes of the literal: for narrow strings the stored bytes, for wide
    /// strings the code units re-encoded as UTF-8. Used for rendering and docs.
    pub fn to_bytes(&self) -> Cow<'_, [u8]> {
        match self {
            StringLiteral::None(bytes) | StringLiteral::Str(bytes) | StringLiteral::Utf8(bytes) => {
                Cow::Borrowed(bytes)
            }
            StringLiteral::Utf16(units)
            | StringLiteral::Utf32(units)
            | StringLiteral::Wide(units) => {
                let mut out = Vec::with_capacity(units.len());
                for &unit in units {
                    let ch = char::from_u32(unit).unwrap_or('\u{FFFD}');
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
                Cow::Owned(out)
            }
        }
    }
}

fn fmt_bytes(f: &mut fmt::Formatter<'_>, bytes: &[u8]) -> fmt::Result {
    for &byte in bytes {
        for escaped in escape_default(byte) {
            write!(f, "{}", escaped as char)?;
        }
    }
    Ok(())
}

impl Display for Constant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Constant::Int(v, _) => write!(f, "{v}"),
            Constant::Bool(v) => write!(f, "{v}"),
            Constant::Float(v, _) => write!(f, "{v}"),
            Constant::Imaginary(v, _) => write!(f, "{v}i"),
            Constant::Char(value, _) => {
                write!(f, "'")?;
                if let Ok(byte) = u8::try_from(*value) {
                    fmt_bytes(f, &[byte])?;
                } else {
                    write!(f, "\\x{value:x}")?;
                }
                write!(f, "'")
            }
            Constant::String(literal) => {
                write!(f, "{}\"", literal.prefix().as_str())?;
                fmt_bytes(f, literal.to_bytes().as_ref())?;
                write!(f, "\"")
            }
        }
    }
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
            Token::Constexpr => write!(f, "constexpr"),
            Token::Complex => write!(f, "_Complex"),
            Token::Continue => write!(f, "continue"),
            Token::Default => write!(f, "default"),
            Token::Do => write!(f, "do"),
            Token::Double => write!(f, "double"),
            Token::Else => write!(f, "else"),
            Token::Enum => write!(f, "enum"),
            Token::Extern => write!(f, "extern"),
            Token::Float => write!(f, "float"),
            Token::Float16 => write!(f, "_Float16"),
            Token::Float32 => write!(f, "_Float32"),
            Token::Float64 => write!(f, "_Float64"),
            Token::Float128 => write!(f, "_Float128"),
            Token::Float32x => write!(f, "_Float32x"),
            Token::Float64x => write!(f, "_Float64x"),
            Token::Float128x => write!(f, "_Float128x"),
            Token::GnuFloat128 => write!(f, "__float128"),
            Token::For => write!(f, "for"),
            Token::Goto => write!(f, "goto"),
            Token::If => write!(f, "if"),
            Token::Inline => write!(f, "inline"),
            Token::Int => write!(f, "int"),
            Token::Int128 => write!(f, "__int128"),
            Token::Long => write!(f, "long"),
            Token::Register => write!(f, "register"),
            Token::Restrict => write!(f, "restrict"),
            Token::Return => write!(f, "return"),
            Token::Short => write!(f, "short"),
            Token::Signed => write!(f, "signed"),
            Token::Sizeof => write!(f, "sizeof"),
            Token::Typeof => write!(f, "typeof"),
            Token::Alignof => write!(f, "alignof"),
            Token::Alignas => write!(f, "alignas"),
            Token::Countof => write!(f, "_Countof"),
            Token::Offsetof => write!(f, "offsetof"),
            Token::Static => write!(f, "static"),
            Token::Atomic => write!(f, "_Atomic"),
            Token::ThreadLocal => write!(f, "_Thread_local"),
            Token::Struct => write!(f, "struct"),
            Token::Switch => write!(f, "switch"),
            Token::Typedef => write!(f, "typedef"),
            Token::Union => write!(f, "union"),
            Token::Unsigned => write!(f, "unsigned"),
            Token::Void => write!(f, "void"),
            Token::Volatile => write!(f, "volatile"),
            Token::While => write!(f, "while"),
            Token::Generic => write!(f, "_Generic"),
            Token::StaticAssert => write!(f, "static_assert"),

            Token::VaStart => write!(f, "va_start"),
            Token::VaArg => write!(f, "va_arg"),
            Token::VaCopy => write!(f, "va_copy"),
            Token::VaEnd => write!(f, "va_end"),
            Token::BuiltinInf => write!(f, "__builtin_inf"),
            Token::BuiltinNan => write!(f, "__builtin_nan"),
            Token::BuiltinConstantP => write!(f, "__builtin_constant_p"),
            Token::BuiltinTypesCompatibleP => write!(f, "__builtin_types_compatible_p"),
            Token::BuiltinComplex => write!(f, "__builtin_complex"),
            Token::TransparentUnionAttr => write!(f, "__attribute__((__transparent_union__))"),

            Token::Ident(s) => write!(f, "{s}"),
            Token::DocComment { inner, text } => {
                if *inner {
                    write!(f, "//!{text}")
                } else {
                    write!(f, "///{text}")
                }
            }

            Token::Integer(num, suffix) => {
                write!(f, "{num}")?;
                match suffix {
                    IntegerSuffix::None | IntegerSuffix::NoneDecimal => Ok(()),
                    IntegerSuffix::Unsigned => write!(f, "u"),
                    IntegerSuffix::Long => write!(f, "l"),
                    IntegerSuffix::LongLong => write!(f, "ll"),
                    IntegerSuffix::UnsignedLong => write!(f, "ul"),
                    IntegerSuffix::UnsignedLongLong => write!(f, "ull"),
                    IntegerSuffix::Usize => write!(f, "usize"),
                    IntegerSuffix::Isize => write!(f, "isize"),
                    IntegerSuffix::U8 => write!(f, "u8"),
                    IntegerSuffix::U16 => write!(f, "u16"),
                    IntegerSuffix::U32 => write!(f, "u32"),
                    IntegerSuffix::U64 => write!(f, "u64"),
                    IntegerSuffix::U128 => write!(f, "u128"),
                    IntegerSuffix::I8 => write!(f, "i8"),
                    IntegerSuffix::I16 => write!(f, "i16"),
                    IntegerSuffix::I32 => write!(f, "i32"),
                    IntegerSuffix::I64 => write!(f, "i64"),
                    IntegerSuffix::I128 => write!(f, "i128"),
                }
            }
            Token::FloatLit(num, suffix) => {
                write!(f, "{num}")?;
                match suffix {
                    FloatSuffix::None => Ok(()),
                    FloatSuffix::Float => write!(f, "f"),
                    FloatSuffix::Long => write!(f, "l"),
                    FloatSuffix::F16 => write!(f, "f16"),
                    FloatSuffix::F32 => write!(f, "f32"),
                    FloatSuffix::F64 => write!(f, "f64"),
                    FloatSuffix::F128 => write!(f, "f128"),
                    FloatSuffix::F32x => write!(f, "f32x"),
                    FloatSuffix::F64x => write!(f, "f64x"),
                    FloatSuffix::F128x => write!(f, "f128x"),
                }
            }
            Token::ImaginaryLit(num, suffix) => {
                write!(f, "{num}")?;
                match suffix {
                    FloatSuffix::None => Ok(()),
                    FloatSuffix::Float => write!(f, "f"),
                    FloatSuffix::Long => write!(f, "l"),
                    FloatSuffix::F16 => write!(f, "f16"),
                    FloatSuffix::F32 => write!(f, "f32"),
                    FloatSuffix::F64 => write!(f, "f64"),
                    FloatSuffix::F128 => write!(f, "f128"),
                    FloatSuffix::F32x => write!(f, "f32x"),
                    FloatSuffix::F64x => write!(f, "f64x"),
                    FloatSuffix::F128x => write!(f, "f128x"),
                }?;
                write!(f, "i")
            }
            Token::CharLit(bytes, prefix) => {
                write!(f, "{}'", prefix.as_str())?;
                fmt_bytes(f, bytes)?;
                write!(f, "'")
            }
            Token::StringLit(literal) => {
                write!(f, "{}\"", literal.prefix().as_str())?;
                fmt_bytes(f, literal.to_bytes().as_ref())?;
                write!(f, "\"")
            }
            Token::BoolLit(v) => write!(f, "{v}"),

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

            Token::Preprocessor(dir) => write!(f, "#{dir}"),
            Token::Ellipsis => write!(f, "..."),
            Token::Hash => write!(f, "#"),
            Token::HashHash => write!(f, "##"),
            Token::Lifetime(name) => write!(f, "'{name}"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum StatementOrDeclaration<R: TypeResolver> {
    Declaration(Spanned<Declaration<R>>),
    Statement(Spanned<Statement<R>>),
}

#[derive(Debug, Clone)]
pub enum PackAction {
    /// `#pragma pack(push, N)` — push current alignment and set to N bytes.
    PushSet(u32),
    /// `#pragma pack(push)` — push current alignment without changing it.
    PushOnly,
    /// `#pragma pack(pop)` — restore the previous alignment.
    Pop,
    /// `#pragma pack(N)` — set alignment to N bytes without pushing.
    Set(u32),
    /// `#pragma pack()` — reset to the default alignment without pushing.
    Reset,
}

#[derive(Debug, Clone)]
pub enum Declaration<R: TypeResolver> {
    FunctionDefinition {
        attrs: Vec<Spanned<RustAttribute>>,
        signature: FunctionDefinitionSignature<R>,
        body: Spanned<LazyCompoundStatement>,
    },
    Declaration {
        attrs: Vec<Spanned<RustAttribute>>,
        declaration_specifiers: Vec<Spanned<DeclarationSpecifier<R>>>,
        declarators: Vec<Spanned<InitDeclarator<R>>>,
    },
    RustTypeAlias {
        attrs: Vec<Spanned<RustAttribute>>,
        ident: Spanned<R::DeclarationIdent>,
        ty: Spanned<RustTy<R>>,
        visibility: Visibility,
    },
    RustStruct {
        attrs: Vec<Spanned<RustAttribute>>,
        ident: Spanned<R::DeclarationIdent>,
        fields: Vec<RustStructField<R>>,
        visibility: Visibility,
    },
    PragmaPack {
        action: PackAction,
    },
    BreakCo2,
    StaticAssert {
        expr: Spanned<Expression<R>>,
        message: Spanned<String>,
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

    pub fn ident_span(&self) -> Option<Span> {
        match self {
            FunctionDefinitionSignature::C { declarator, .. } => declarator.0.ident_span(),
            FunctionDefinitionSignature::Rust(sig) => Some(sig.name.1),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RustFunctionSignature<R: TypeResolver> {
    pub attrs: Vec<Spanned<RustAttribute>>,
    pub name: Spanned<R::DeclarationIdent>,
    pub params: Vec<RustFunctionParam<R>>,
    pub ret_ty: Spanned<RustTy<R>>,
    pub visibility: Visibility,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Public,
    Crate,
    Restricted,
    Private,
}

#[derive(Debug, Clone)]
pub struct RustFunctionParam<R: TypeResolver> {
    pub name: Spanned<R::DeclarationIdent>,
    pub ty: Spanned<RustTy<R>>,
}

#[derive(Debug, Clone)]
pub struct RustStructField<R: TypeResolver> {
    pub name: Spanned<R::DeclarationIdent>,
    pub visibility: Visibility,
    pub ty: Spanned<RustTy<R>>,
}

#[derive(Debug, Clone)]
pub enum RustTy<R: TypeResolver> {
    Path(Spanned<R::ResolvedRustPath>),
    Tuple(Vec<Spanned<RustTy<R>>>),
    Ref {
        lifetime: Option<Spanned<String>>,
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
    Wild,
    Lifetime(Spanned<String>),
}

#[derive(Debug, Clone)]
pub struct LazyRustConstExpr {
    pub tokens: Vec<Spanned<Token>>,
}

#[derive(Debug, Clone)]
pub struct RustAttribute {
    pub path: Vec<Spanned<String>>,
    pub args: Vec<Spanned<Token>>,
    pub style: RustAttributeStyle,
}

impl RustAttribute {
    pub fn is_word(&self, name: &str) -> bool {
        self.args.is_empty() && matches!(self.path.as_slice(), [(segment, _)] if segment == name)
    }

    pub fn is_inner(&self) -> bool {
        self.style == RustAttributeStyle::Inner
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RustAttributeStyle {
    Outer,
    Inner,
}

impl LazyRustConstExpr {
    pub fn constant_len(&self) -> Option<u128> {
        if let [(token, _)] = &self.tokens[..] {
            match token {
                Token::Integer(text, IntegerSuffix::None) => parse_unsigned_integer_constant(text),
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
    Constexpr,
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
        if self.tokens.len() == 2 {
            return true;
        }
        // `[qualifier...]` with no size expression (e.g. `[const]`) is also unsized;
        // it means the parameter is a qualified pointer with no specified array length.
        let [_, inner @ .., _] = &self.tokens[..] else {
            return false;
        };
        inner.iter().all(|(token, _)| {
            matches!(
                token,
                Token::Const | Token::Restrict | Token::Volatile | Token::Atomic
            )
        })
    }

    pub fn is_unspecified_vla(&self) -> bool {
        let [_, inner @ .., _] = &self.tokens[..] else {
            return false;
        };
        let inner = inner
            .iter()
            .skip_while(|(token, _)| {
                matches!(
                    token,
                    Token::Static
                        | Token::Const
                        | Token::Restrict
                        | Token::Volatile
                        | Token::Atomic
                )
            })
            .collect::<Vec<_>>();
        matches!(inner.as_slice(), [(Token::Star, _)])
    }

    pub fn constant_len(&self) -> Option<u128> {
        let [_, inner @ .., _] = &self.tokens[..] else {
            return None;
        };
        let inner = inner
            .iter()
            .skip_while(|(token, _)| {
                matches!(
                    token,
                    Token::Static
                        | Token::Const
                        | Token::Restrict
                        | Token::Volatile
                        | Token::Atomic
                )
            })
            .collect::<Vec<_>>();
        let [(token, _)] = inner.as_slice() else {
            return None;
        };
        match token {
            Token::Integer(text, suffix) => {
                if !matches!(suffix, IntegerSuffix::None) {
                    return None;
                }
                parse_unsigned_integer_constant(text)
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
        }
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
            Declarator::PointerDeclarator { declarator, .. }
            | Declarator::ArrayDeclarator { declarator, .. } => declarator.0.is_function(),
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

    pub fn ident_span(&self) -> Option<Span> {
        match self {
            Declarator::Abstract => None,
            Declarator::Identifier(ident) => Some(ident.1),
            Declarator::FunctionDeclarator { declarator, .. }
            | Declarator::PointerDeclarator { declarator, .. }
            | Declarator::ArrayDeclarator { declarator, .. } => declarator.0.ident_span(),
        }
    }
}

impl Declarator<StatelessResolver> {
    pub fn is_unsized_array(&self) -> bool {
        match self {
            Declarator::ArrayDeclarator {
                declarator,
                subscription,
            } => {
                if subscription.0.is_unsized() {
                    true
                } else {
                    declarator.0.is_unsized_array()
                }
            }
            Declarator::PointerDeclarator { declarator, .. }
            | Declarator::FunctionDeclarator { declarator, .. } => declarator.0.is_unsized_array(),
            Declarator::Identifier(_) | Declarator::Abstract => false,
        }
    }
}

#[derive(Debug, Clone)]
pub enum DeclarationSpecifier<R: TypeResolver> {
    TypeSpecifier(Spanned<TypeSpecifier<R>>),
    TypeQualifier(Spanned<TypeQualifier>),
    StorageSpecifier(Spanned<StorageClassSpecifier>),
    FunctionSpecifier(Spanned<FunctionSpecifier>),
    GNUAttribute(Vec<Spanned<RustAttribute>>),
}

impl<R: TypeResolver> DeclarationSpecifier<R> {
    pub fn is_typedef(&self) -> bool {
        match self {
            DeclarationSpecifier::TypeSpecifier(_)
            | DeclarationSpecifier::TypeQualifier(_)
            | DeclarationSpecifier::FunctionSpecifier(_) => false,
            DeclarationSpecifier::GNUAttribute(_) => false,
            DeclarationSpecifier::StorageSpecifier(c) => {
                matches!(c.0, StorageClassSpecifier::Typedef)
            }
        }
    }

    pub fn is_extern(&self) -> bool {
        match self {
            DeclarationSpecifier::TypeSpecifier(_)
            | DeclarationSpecifier::TypeQualifier(_)
            | DeclarationSpecifier::FunctionSpecifier(_) => false,
            DeclarationSpecifier::GNUAttribute(_) => false,
            DeclarationSpecifier::StorageSpecifier(c) => {
                matches!(c.0, StorageClassSpecifier::Extern)
            }
        }
    }

    pub fn is_static(&self) -> bool {
        match self {
            DeclarationSpecifier::TypeSpecifier(_)
            | DeclarationSpecifier::TypeQualifier(_)
            | DeclarationSpecifier::FunctionSpecifier(_) => false,
            DeclarationSpecifier::GNUAttribute(_) => false,
            DeclarationSpecifier::StorageSpecifier(c) => {
                matches!(c.0, StorageClassSpecifier::Static)
            }
        }
    }

    pub fn is_constexpr(&self) -> bool {
        match self {
            DeclarationSpecifier::TypeSpecifier(_)
            | DeclarationSpecifier::TypeQualifier(_)
            | DeclarationSpecifier::FunctionSpecifier(_) => false,
            DeclarationSpecifier::GNUAttribute(_) => false,
            DeclarationSpecifier::StorageSpecifier(c) => {
                matches!(c.0, StorageClassSpecifier::Constexpr)
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct TranslationUnit<R: TypeResolver> {
    pub attrs: Vec<Spanned<RustAttribute>>,
    pub rust_use_items: Vec<Spanned<UseItem>>,
    pub rust_mod_items: Vec<Spanned<ModItem>>,
    pub items: Vec<Spanned<Declaration<R>>>,
}

#[derive(Debug, Clone)]
pub struct UseItem {
    pub attrs: Vec<Spanned<RustAttribute>>,
    pub path: Vec<Spanned<String>>,
    pub alias: Option<Spanned<String>>,
}

#[derive(Debug, Clone)]
pub struct ModItem {
    pub attrs: Vec<Spanned<RustAttribute>>,
    pub name: Spanned<String>,
    /// For inline modules (`mod foo { ... }`), the inner tokens (without braces).
    /// `None` means this is a file-based module (`mod foo;`).
    pub inline_content: Option<Spanned<Vec<Spanned<Token>>>>,
}

pub fn co2_test_symbol_name(module_path: &[String], name: &str) -> String {
    format!("__co2_test{}", co2_test_symbol_suffix(module_path, name))
}

pub fn co2_test_symbol_suffix(module_path: &[String], name: &str) -> String {
    let mut suffix = String::new();
    for segment in module_path.iter().map(String::as_str).chain([name]) {
        suffix.push('_');
        suffix.push_str(&segment.len().to_string());
        suffix.push('_');
        suffix.push_str(segment);
    }
    suffix
}

pub fn parse_unsigned_integer_constant(text: &str) -> Option<u128> {
    let stripped: String = text.chars().filter(|&c| c != '_' && c != '\'').collect();
    let text = &stripped;

    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        return u128::from_str_radix(hex, 16).ok();
    }

    if let Some(binary) = text.strip_prefix("0b").or_else(|| text.strip_prefix("0B")) {
        return u128::from_str_radix(binary, 2).ok();
    }

    if text.len() > 1
        && let Some(octal) = text.strip_prefix('0')
    {
        return u128::from_str_radix(octal, 8).ok();
    }

    text.parse::<u128>().ok()
}
