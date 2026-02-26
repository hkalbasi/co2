use rustc_public::ty::{AdtDef, FnDef, Span};

use crate::{DefId, HirTy};

#[derive(Debug, Clone)]
pub struct HirStructure {
    pub root: HirModule,
}

#[derive(Debug, Clone)]
pub struct HirModule {
    pub span: Span,
    pub items: Vec<HirModuleItem>,
}

#[derive(Debug, Clone)]
pub enum HirModuleItem {
    Function {
        name: String,
        id: FnDef,
        sig: FunctionSignature,
        no_mangle: bool,
        span: Span,
    },
    Adt {
        name: String,
        id: AdtDef,
        kind: HirAdtKind,
        span: Span,
    },
    TypeDef {
        name: String,
        id: DefId,
        ty: HirTy,
        span: Span,
    },
    Static {
        name: String,
        id: DefId,
        ty: HirTy,
        mutable: bool,
        span: Span,
    },
    Impl {
        id: DefId,
        self_ty: HirTy,
        trait_def: Option<DefId>,
        items: Vec<HirImplItem>,
        span: Span,
    },
    ForeignMod {
        id: DefId,
        items: Vec<ForeignModItem>,
    },
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
    RefImm(DefId),
    RefMut(DefId),
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
    ForeignFunction {
        name: String,
        id: FnDef,
        sig: FunctionSignature,
        span: Span,
    },
}

#[derive(Debug, Clone)]
pub struct FunctionSignature {
    pub lifetimes: Vec<DefId>,
    pub inputs: Vec<HirTy>,
    pub output: HirTy,
    pub abi: FunctionAbi,
    pub is_unsafe: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FunctionAbi {
    Rust,
    C,
}
