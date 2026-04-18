#![feature(rustc_private)]

//! rustc_public_generative
//!
//! This crate provides a `generate` entrypoint that runs `rustc_driver`
//! but injects a synthetic crate produced by user code.

use std::{any::Any, path::PathBuf};

use rustc_middle::ty::TyCtxt;
use rustc_public::{
    DefId,
    ty::{AdtDef, FnDef},
};

extern crate rustc_abi;
extern crate rustc_ast;
extern crate rustc_data_structures;
extern crate rustc_driver;
extern crate rustc_hashes;
extern crate rustc_hir;
extern crate rustc_index;
extern crate rustc_interface;
extern crate rustc_lint;
extern crate rustc_middle;
pub extern crate rustc_public;
extern crate rustc_public_bridge;
extern crate rustc_session;
extern crate rustc_span;
extern crate rustc_target;
extern crate rustc_trait_selection;

mod hir_structure;
mod hir_ty;
mod internal;

pub use hir_structure::{
    ForeignModItem, FunctionAbi, FunctionSignature, HirAdtKind, HirImplItem, HirImplItemKind,
    HirModule, HirModuleItem, HirSelfKind, HirStructure, StructField, AdtRepr,
};
pub use hir_ty::{HirGenericArg, HirLifetime, HirTy, HirTyConst, HirTyKind};

/// Summary of crates loaded as dependencies by rustc.
#[derive(Debug, Clone, Default)]
pub struct DependencyInfo {
    pub crates: Vec<DependencyCrate>,
    pub functions: Vec<DependencyFunction>,
    pub values: Vec<DependencyValue>,
    pub types: Vec<DependencyType>,
    pub traits: Vec<DependencyTrait>,
}

#[derive(Debug, Clone)]
pub struct DependencyCrate {
    pub name: String,
    pub disambiguator: String,
}

#[derive(Debug, Clone)]
pub struct DependencyFunction {
    pub path: String,
    pub def_path_hash_hi: u64,
    pub def_path_hash_lo: u64,
    pub fn_def: Option<FnDef>,
}

#[derive(Debug, Clone)]
pub enum DependencyConstValue {
    Bool(bool),
    Char(char),
    I8(i8),
    I16(i16),
    I32(i32),
    I64(i64),
    I128(i128),
    Isize(i64),
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    U128(u128),
    Usize(u64),
    F32(f32),
    F64(f64),
}

#[derive(Debug, Clone)]
pub enum DependencyValueKind {
    Def(DefId),
    ConstDef(DefId),
}

#[derive(Debug, Clone)]
pub struct DependencyValue {
    pub kind: DependencyValueKind,
    pub path: String,
    pub def_path_hash_hi: u64,
    pub def_path_hash_lo: u64,
}

#[derive(Debug, Clone)]
pub struct DependencyType {
    pub adt: AdtDef,
    pub path: String,
    pub def_path_hash_hi: u64,
    pub def_path_hash_lo: u64,
}

#[derive(Debug, Clone)]
pub struct DependencyTrait {
    pub def_id: DefId,
    pub path: String,
    pub def_path_hash_hi: u64,
    pub def_path_hash_lo: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileId(u32);

pub struct HirStructureCtx<'tcx> {
    tcx: TyCtxt<'tcx>,
    inner: internal::Context,
}

impl<'tcx> std::fmt::Debug for HirStructureCtx<'tcx> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HirStructureCtx").finish()
    }
}

impl HirStructureCtx<'_> {
    pub fn dependencies(&self) -> DependencyInfo {
        internal::collect_dependency_info(self.tcx)
    }

    pub fn add_custom_file(&self, path: impl Into<PathBuf>, contents: impl Into<String>) -> FileId {
        let result = self.inner.add_custom_file(path, contents);
        self.inner.register_with_source_map(self.tcx);
        result
    }

    pub fn span_in_file(&self, file: FileId, lo: u32, hi: u32) -> rustc_public::ty::Span {
        self.inner.span_in_file(file, lo, hi)
    }

    pub fn root_crate_def_id(&self) -> DefId {
        internal::root_crate_def_id(self.tcx)
    }

    pub fn allocate_def_id(&self, parent: DefId, data: DefData) -> DefId {
        internal::allocate_def_id(self.tcx, parent, data)
    }

    pub fn dependency_const_value(&self, def_id: DefId) -> Option<DependencyConstValue> {
        internal::dependency_const_value_for_def_id(self.tcx, def_id)
    }

    pub fn type_implements_trait(
        &self,
        owner: DefId,
        ty: rustc_public::ty::Ty,
        trait_def_id: DefId,
    ) -> bool {
        internal::type_implements_trait(self.tcx, owner, ty, trait_def_id)
    }

    pub fn type_is_copy(&self, owner: DefId, ty: rustc_public::ty::Ty) -> bool {
        internal::type_is_copy(self.tcx, owner, ty)
    }

    pub fn erase_late_bound_regions_in_fn_sig(
        &self,
        sig: rustc_public::ty::Binder<rustc_public::ty::FnSig>,
    ) -> rustc_public::ty::FnSig {
        erase_late_bound_regions_in_fn_sig(sig)
    }
}

pub fn erase_late_bound_regions_in_fn_sig(
    sig: rustc_public::ty::Binder<rustc_public::ty::FnSig>,
) -> rustc_public::ty::FnSig {
    erase_bound_regions_in_fn_sig(sig.skip_binder())
}

fn erase_bound_regions_in_fn_sig(mut sig: rustc_public::ty::FnSig) -> rustc_public::ty::FnSig {
    sig.inputs_and_output = sig
        .inputs_and_output
        .into_iter()
        .map(erase_bound_regions_in_ty)
        .collect();
    sig
}

fn erase_bound_regions_in_ty(ty: rustc_public::ty::Ty) -> rustc_public::ty::Ty {
    use rustc_public::ty::{GenericArgKind, Region, RegionKind, RigidTy, TyConst, TyConstKind, TyKind};

    fn erase_region(region: Region) -> Region {
        let kind = match region.kind {
            RegionKind::ReBound(_, _) => RegionKind::ReErased,
            other => other,
        };
        Region { kind }
    }

    fn erase_args(args: rustc_public::ty::GenericArgs) -> rustc_public::ty::GenericArgs {
        rustc_public::ty::GenericArgs(
            args.0
                .into_iter()
                .map(|arg| match arg {
                    GenericArgKind::Lifetime(region) => {
                        GenericArgKind::Lifetime(erase_region(region))
                    }
                    GenericArgKind::Type(ty) => GenericArgKind::Type(erase_bound_regions_in_ty(ty)),
                    GenericArgKind::Const(c) => GenericArgKind::Const(erase_const(c)),
                })
                .collect(),
        )
    }

    fn erase_const(c: TyConst) -> TyConst {
        let kind = match c.kind().clone() {
            TyConstKind::Param(param) => TyConstKind::Param(param),
            TyConstKind::Bound(db, var) => TyConstKind::Bound(db, var),
            TyConstKind::Unevaluated(def, args) => TyConstKind::Unevaluated(def, erase_args(args)),
            TyConstKind::Value(ty, alloc) => {
                TyConstKind::Value(erase_bound_regions_in_ty(ty), alloc)
            }
            TyConstKind::ZSTValue(ty) => TyConstKind::ZSTValue(erase_bound_regions_in_ty(ty)),
        };
        TyConst::new(kind, c.id)
    }

    match ty.kind() {
        TyKind::RigidTy(rigid) => {
            let rigid = match rigid {
                RigidTy::Bool => RigidTy::Bool,
                RigidTy::Char => RigidTy::Char,
                RigidTy::Int(int) => RigidTy::Int(int),
                RigidTy::Uint(uint) => RigidTy::Uint(uint),
                RigidTy::Float(float) => RigidTy::Float(float),
                RigidTy::Adt(def, args) => RigidTy::Adt(def, erase_args(args)),
                RigidTy::Foreign(def) => RigidTy::Foreign(def),
                RigidTy::Str => RigidTy::Str,
                RigidTy::Array(elem, len) => {
                    RigidTy::Array(erase_bound_regions_in_ty(elem), erase_const(len))
                }
                RigidTy::Pat(inner, pattern) => {
                    RigidTy::Pat(erase_bound_regions_in_ty(inner), pattern)
                }
                RigidTy::Slice(elem) => RigidTy::Slice(erase_bound_regions_in_ty(elem)),
                RigidTy::RawPtr(pointee, mutability) => {
                    RigidTy::RawPtr(erase_bound_regions_in_ty(pointee), mutability)
                }
                RigidTy::Ref(region, pointee, mutability) => RigidTy::Ref(
                    erase_region(region),
                    erase_bound_regions_in_ty(pointee),
                    mutability,
                ),
                RigidTy::FnDef(def, args) => RigidTy::FnDef(def, erase_args(args)),
                RigidTy::FnPtr(sig) => {
                    RigidTy::FnPtr(sig.map_bound(erase_bound_regions_in_fn_sig))
                }
                RigidTy::Closure(def, args) => RigidTy::Closure(def, erase_args(args)),
                RigidTy::Coroutine(def, args) => RigidTy::Coroutine(def, erase_args(args)),
                RigidTy::CoroutineClosure(def, args) => {
                    RigidTy::CoroutineClosure(def, erase_args(args))
                }
                RigidTy::Dynamic(predicates, region) => RigidTy::Dynamic(
                    predicates
                        .into_iter()
                        .map(|predicate| {
                            predicate.map_bound(|predicate| match predicate {
                                rustc_public::ty::ExistentialPredicate::Trait(trait_ref) => {
                                    rustc_public::ty::ExistentialPredicate::Trait(
                                        rustc_public::ty::ExistentialTraitRef {
                                            def_id: trait_ref.def_id,
                                            generic_args: erase_args(trait_ref.generic_args),
                                        },
                                    )
                                }
                                rustc_public::ty::ExistentialPredicate::Projection(projection) => {
                                    rustc_public::ty::ExistentialPredicate::Projection(
                                        rustc_public::ty::ExistentialProjection {
                                            def_id: projection.def_id,
                                            generic_args: erase_args(projection.generic_args),
                                            term: match projection.term {
                                                rustc_public::ty::TermKind::Type(ty) => {
                                                    rustc_public::ty::TermKind::Type(
                                                        erase_bound_regions_in_ty(ty),
                                                    )
                                                }
                                                rustc_public::ty::TermKind::Const(c) => {
                                                    rustc_public::ty::TermKind::Const(erase_const(c))
                                                }
                                            },
                                        },
                                    )
                                }
                                rustc_public::ty::ExistentialPredicate::AutoTrait(def) => {
                                    rustc_public::ty::ExistentialPredicate::AutoTrait(def)
                                }
                            })
                        })
                        .collect(),
                    erase_region(region),
                ),
                RigidTy::Never => RigidTy::Never,
                RigidTy::Tuple(tys) => {
                    RigidTy::Tuple(tys.into_iter().map(erase_bound_regions_in_ty).collect())
                }
                RigidTy::CoroutineWitness(def, args) => {
                    RigidTy::CoroutineWitness(def, erase_args(args))
                }
            };
            rustc_public::ty::Ty::from_rigid_kind(rigid)
        }
        TyKind::Alias(_, _) | TyKind::Param(_) | TyKind::Bound(_, _) => ty,
    }
}

pub trait CrateGeneratorState: Sync + Send + Any + Sized {
    fn hir_structure(ctx: HirStructureCtx) -> (Self, HirStructure);
    fn emit_mir(&mut self, ctx: HirStructureCtx, def: DefId) -> rustc_public::mir::Body;
}

pub fn generate<S: CrateGeneratorState>() {
    internal::generate::<S>();
}

pub fn generate_with_args<S: CrateGeneratorState>(args: Vec<String>) {
    internal::generate_with_args::<S>(args);
}

#[derive(Debug)]
pub enum DefData {
    ForeignMod,
    ValueNs(String),
    TypeNs(String),
    LifetimeNs(String),
    Impl,
    AnonConst,
}
