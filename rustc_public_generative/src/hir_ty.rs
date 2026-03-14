use rustc_public::{
    DefId,
    mir::Mutability,
    ty::{FloatTy, IntTy, Span, UintTy},
};

use crate::FunctionSignature;

#[derive(Clone, Debug)]
pub enum HirTyKind {
    Bool,
    Char,
    Int(IntTy),
    Uint(UintTy),
    Float(FloatTy),
    Adt(DefId, Vec<HirGenericArg>),
    Tuple(Vec<HirTy>),
    RawPtr(Mutability, Box<HirTy>),
    Array(usize, Box<HirTy>),
    Ref(Mutability, HirLifetime, Box<HirTy>),
    FnPtr(Box<FunctionSignature>),
}

#[derive(Clone, Debug)]
pub struct HirTy {
    pub kind: HirTyKind,
    pub span: Span,
}
impl HirTy {
    pub fn usize_ty(span: Span) -> Self {
        HirTy {
            kind: HirTyKind::Uint(UintTy::Usize),
            span,
        }
    }

    pub fn signed_ty(u: IntTy, span: Span) -> Self {
        HirTy {
            kind: HirTyKind::Int(u),
            span,
        }
    }

    pub fn unsigned_ty(u: UintTy, span: Span) -> Self {
        HirTy {
            kind: HirTyKind::Uint(u),
            span,
        }
    }

    pub fn float_ty(f: FloatTy, span: Span) -> Self {
        HirTy {
            kind: HirTyKind::Float(f),
            span,
        }
    }

    pub fn new_tuple(inner: Vec<HirTy>, span: Span) -> Self {
        HirTy {
            kind: HirTyKind::Tuple(inner),
            span,
        }
    }

    pub fn new_array(inner: HirTy, len: usize, span: Span) -> Self {
        HirTy {
            kind: HirTyKind::Array(len, Box::new(inner)),
            span,
        }
    }

    pub fn new_ptr(inner: HirTy, mutbl: Mutability, span: Span) -> Self {
        HirTy {
            kind: HirTyKind::RawPtr(mutbl, Box::new(inner)),
            span,
        }
    }

    pub fn new_ref(inner: HirTy, mutbl: Mutability, lifetime: HirLifetime, span: Span) -> Self {
        HirTy {
            kind: HirTyKind::Ref(mutbl, lifetime, Box::new(inner)),
            span,
        }
    }

    pub fn adt(adt: DefId, args: Vec<HirGenericArg>, span: Span) -> Self {
        HirTy {
            kind: HirTyKind::Adt(adt, args),
            span,
        }
    }
}

#[derive(Clone, Debug)]
pub enum HirGenericArg {
    Ty(HirTy),
    Lifetime(HirLifetime),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HirLifetime {
    Static,
    Param(DefId),
}
