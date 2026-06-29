use rustc_public::ty::{AdtDef, FnDef, Span};

use crate::{DefId, HirLifetime, HirTy};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Public,
    Crate,
    Restricted,
    Private,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineHint {
    Hint,
    Always,
    Never,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GeneratedAttr {
    Word { path: Vec<String> },
    DocComment { comment: String, inner: bool },
    InlineHint(InlineHint),
}

#[derive(Debug, Clone)]
pub struct HirStructure {
    pub no_main: bool,
    pub root: HirModule,
}

#[derive(Debug, Clone)]
pub struct HirModule {
    pub span: Span,
    pub attrs: Vec<GeneratedAttr>,
    pub items: Vec<HirModuleItem>,
}

#[derive(Debug, Clone)]
pub enum HirModuleItem {
    Function {
        name: String,
        id: FnDef,
        sig: FunctionSignature,
        attrs: Vec<GeneratedAttr>,
        no_mangle: bool,
        visibility: Visibility,
        span: Span,
        ident_span: Span,
    },
    Adt {
        name: String,
        id: AdtDef,
        repr: AdtRepr,
        kind: HirAdtKind,
        visibility: Visibility,
        span: Span,
    },
    TypeDef {
        name: String,
        id: DefId,
        ty: HirTy,
        attrs: Vec<GeneratedAttr>,
        visibility: Visibility,
        span: Span,
    },
    Static {
        name: String,
        id: DefId,
        ty: HirTy,
        mutable: bool,
        no_mangle: bool,
        attrs: Vec<GeneratedAttr>,
        visibility: Visibility,
        span: Span,
    },
    Const {
        name: String,
        id: DefId,
        ty: HirTy,
        rhs: DefId,
        attrs: Vec<GeneratedAttr>,
        visibility: Visibility,
        span: Span,
    },
    Impl {
        id: DefId,
        self_ty: HirTy,
        trait_def: Option<DefId>,
        items: Vec<HirImplItem>,
        span: Span,
    },
    Module {
        name: String,
        id: DefId,
        module: HirModule,
        attrs: Vec<GeneratedAttr>,
        visibility: Visibility,
        span: Span,
    },
    ForeignMod {
        id: DefId,
        items: Vec<ForeignModItem>,
    },
}

impl HirModuleItem {
    pub fn name(&self) -> Option<&str> {
        match self {
            HirModuleItem::Function { name, .. }
            | HirModuleItem::Adt { name, .. }
            | HirModuleItem::TypeDef { name, .. }
            | HirModuleItem::Const { name, .. }
            | HirModuleItem::Static { name, .. }
            | HirModuleItem::Module { name, .. } => Some(name),
            HirModuleItem::Impl { .. } | HirModuleItem::ForeignMod { .. } => None,
        }
    }

    pub fn span(&self) -> Option<Span> {
        match self {
            HirModuleItem::Function { span, .. }
            | HirModuleItem::Adt { span, .. }
            | HirModuleItem::TypeDef { span, .. }
            | HirModuleItem::Const { span, .. }
            | HirModuleItem::Static { span, .. }
            | HirModuleItem::Impl { span, .. }
            | HirModuleItem::Module { span, .. } => Some(*span),
            HirModuleItem::ForeignMod { .. } => None,
        }
    }

    pub fn ident_span(&self) -> Option<Span> {
        match self {
            HirModuleItem::Function { ident_span, .. } => Some(*ident_span),
            _ => self.span(),
        }
    }

    pub fn visibility(&self) -> Visibility {
        match self {
            HirModuleItem::Function { visibility, .. }
            | HirModuleItem::Adt { visibility, .. }
            | HirModuleItem::TypeDef { visibility, .. }
            | HirModuleItem::Static { visibility, .. }
            | HirModuleItem::Const { visibility, .. }
            | HirModuleItem::Module { visibility, .. } => *visibility,
            HirModuleItem::Impl { .. } | HirModuleItem::ForeignMod { .. } => Visibility::Public,
        }
    }

    pub fn attrs(&self) -> &[GeneratedAttr] {
        match self {
            HirModuleItem::Function { attrs, .. }
            | HirModuleItem::TypeDef { attrs, .. }
            | HirModuleItem::Const { attrs, .. }
            | HirModuleItem::Static { attrs, .. }
            | HirModuleItem::Module { attrs, .. } => attrs,
            HirModuleItem::Adt { .. }
            | HirModuleItem::Impl { .. }
            | HirModuleItem::ForeignMod { .. } => &[],
        }
    }
}

#[derive(Debug, Clone)]
pub enum HirAdtKind {
    Struct { fields: Vec<StructField> },
    Union { fields: Vec<StructField> },
}

#[derive(Debug, Clone)]
pub struct StructField {
    pub id: DefId,
    pub name: String,
    pub ty: HirTy,
    pub span: Span,
    pub visibility: Visibility,
}

#[derive(Debug, Clone)]
pub struct HirImplItem {
    pub name: String,
    pub id: DefId,
    pub kind: HirImplItemKind,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HirSelfKind {
    Imm,
    Mut,
    RefImm(HirLifetime),
    RefMut(HirLifetime),
    None,
}

#[derive(Debug, Clone)]
pub enum HirImplItemKind {
    Fn {
        sig: FunctionSignature,
        self_kind: HirSelfKind,
        trait_item_def_id: Option<DefId>,
    },
}

#[derive(Debug, Clone)]
pub enum ForeignModItem {
    ForeignType {
        name: String,
        id: DefId,
        span: Span,
    },
    ForeignFunction {
        name: String,
        id: FnDef,
        sig: FunctionSignature,
        span: Span,
    },
    ForeignStatic {
        name: String,
        id: DefId,
        ty: HirTy,
        mutable: bool,
        span: Span,
    },
}

#[derive(Debug, Clone)]
pub struct FunctionInput {
    pub name: Option<String>,
    pub ty: HirTy,
}

#[derive(Debug, Clone)]
pub struct FunctionSignature {
    pub lifetimes: Vec<DefId>,
    pub inputs: Vec<FunctionInput>,
    pub output: HirTy,
    pub abi: FunctionAbi,
    pub is_unsafe: bool,
    pub c_variadic: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FunctionAbi {
    Rust,
    C,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdtRepr {
    Rust,
    C,
    /// `#[repr(C, packed(n))]` — C layout with maximum field alignment of `n` bytes.
    CPacked(u32),
}
