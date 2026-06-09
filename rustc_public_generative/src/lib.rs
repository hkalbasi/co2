#![feature(rustc_private)]

//! rustc_public_generative
//!
//! This crate provides a `generate` entrypoint that runs `rustc_driver`
//! but injects a synthetic crate produced by user code.

use std::{any::Any, path::PathBuf};

use rustc_middle::ty::TyCtxt;
use rustc_public::DefId;
use rustc_public::ty::{FnSig, GenericArgs, Ty};

extern crate rustc_abi;
extern crate rustc_ast;
extern crate rustc_data_structures;
extern crate rustc_driver;
extern crate rustc_hashes;
extern crate rustc_hir;
extern crate rustc_hir_analysis;
extern crate rustc_index;
extern crate rustc_infer;
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
    AdtRepr, ForeignModItem, FunctionAbi, FunctionInput, FunctionSignature, GeneratedAttr,
    HirAdtKind, HirImplItem, HirImplItemKind, HirModule, HirModuleItem, HirSelfKind, HirStructure,
    StructField,
};
pub use hir_ty::{HirGenericArg, HirLifetime, HirTy, HirTyConst, HirTyKind};

/// Information about a dependency crate.
#[derive(Debug, Clone)]
pub struct DependencyCrate {
    pub name: String,
    pub disambiguator: String,
}

/// A child item within a module or crate, returned by `DependencyInfo::children()`.
#[derive(Debug, Clone)]
pub struct DependencyChild {
    pub def_id: DefId,
    pub name: String,
    pub kind: DependencyChildKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyChildKind {
    Module,
    Function,
    Struct,
    Enum,
    Union,
    Trait,
    Const,
    Static,
    Other,
}

/// An inherent impl function for an ADT, returned by `DependencyInfo::impls()`.
#[derive(Debug, Clone)]
pub struct ImplFunction {
    pub def_id: DefId,
    pub name: String,
}

/// Lazily queried dependency information.
///
/// Instead of eagerly collecting all dependency items into flat vectors,
/// this structure queries rustc on demand via `roots()`, `children()`, and `impls()`.
pub struct DependencyInfo<'tcx> {
    pub tcx: TyCtxt<'tcx>,
}

impl std::fmt::Debug for DependencyInfo<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DependencyInfo").finish()
    }
}

impl DependencyInfo<'_> {
    pub fn roots(&self) -> Vec<(DependencyCrate, DefId)> {
        internal::dependency_roots(self.tcx)
    }

    pub fn children(&self, def_id: DefId) -> Vec<DependencyChild> {
        internal::dependency_children(self.tcx, def_id)
    }

    pub fn impls(&self, def_id: DefId) -> Vec<ImplFunction> {
        internal::dependency_impls(self.tcx, def_id)
    }

    pub fn incoherent_impls(&self, receiver_ty: rustc_public::ty::Ty) -> Vec<ImplFunction> {
        internal::dependency_incoherent_impls(self.tcx, receiver_ty)
    }

    pub fn is_trait(&self, def_id: DefId) -> bool {
        internal::dependency_is_trait(self.tcx, def_id)
    }

    pub fn fn_once_output_params(&self, fn_def_id: DefId) -> Vec<(u32, u32)> {
        internal::fn_once_output_params(self.tcx, fn_def_id)
    }

    pub fn check_fn_predicates(
        &self,
        fn_def_id: DefId,
        fn_generic_args: &GenericArgs,
        owner: DefId,
    ) -> Result<(), String> {
        internal::check_fn_predicates(self.tcx, fn_def_id, fn_generic_args, owner)
    }

    pub fn resolve_inherent_method(
        &self,
        owner: DefId,
        receiver_ty: Ty,
        method: &str,
    ) -> Result<Option<ResolvedMethod>, String> {
        internal::resolve_inherent_method(self.tcx, owner, receiver_ty, method)
    }

    pub fn resolve_method(
        &self,
        owner: DefId,
        receiver_ty: Ty,
        method: &str,
        traits_in_scope: &[DefId],
    ) -> Result<Option<ResolvedMethod>, String> {
        internal::resolve_method(self.tcx, owner, receiver_ty, method, traits_in_scope)
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedMethod {
    pub def_id: DefId,
    pub generic_args: GenericArgs,
    pub sig: FnSig,
    pub receiver_adjustment: ReceiverAdjustment,
}

#[derive(Debug, Clone, Default)]
pub struct ReceiverAdjustment {
    pub autoderefs: usize,
    pub steps: Vec<ReceiverAdjustmentStep>,
    pub autoref: Option<rustc_public::mir::Mutability>,
    pub mut_ptr_to_const_ptr: bool,
}

#[derive(Debug, Clone)]
pub enum ReceiverAdjustmentStep {
    BuiltinDeref {
        source: Ty,
        target: Ty,
    },
    OverloadedDeref {
        source: Ty,
        target: Ty,
        target_ref: Ty,
        method_def_id: DefId,
        generic_args: GenericArgs,
        sig: FnSig,
    },
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileId(u32);

pub struct HirStructureCtx<'tcx> {
    pub tcx: TyCtxt<'tcx>,
    inner: internal::Context,
}

impl std::fmt::Debug for HirStructureCtx<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HirStructureCtx").finish()
    }
}

impl<'tcx> HirStructureCtx<'tcx> {
    pub fn dependencies(&self) -> DependencyInfo<'tcx> {
        DependencyInfo { tcx: self.tcx }
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

    pub fn allocate_def_id(&self, parent: DefId, data: &DefData) -> DefId {
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

    pub fn normalize_ty_defaults(&self, ty: rustc_public::ty::Ty) -> rustc_public::ty::Ty {
        internal::normalize_ty_defaults(self.tcx, ty)
    }

    pub fn normalize_ty_for_owner(
        &self,
        owner: DefId,
        ty: rustc_public::ty::Ty,
    ) -> rustc_public::ty::Ty {
        internal::normalize_ty_for_owner(self.tcx, owner, ty)
    }

    pub fn normalize_ty_for_owner_with_self(
        &self,
        owner: DefId,
        ty: rustc_public::ty::Ty,
        self_ty: rustc_public::ty::Ty,
    ) -> rustc_public::ty::Ty {
        internal::normalize_ty_for_owner_with_self(self.tcx, owner, ty, self_ty)
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
    use rustc_public::ty::{
        GenericArgKind, Region, RegionKind, RigidTy, TyConst, TyConstKind, TyKind,
    };

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
                    GenericArgKind::Const(c) => GenericArgKind::Const(erase_const(&c)),
                })
                .collect(),
        )
    }

    fn erase_const(c: &TyConst) -> TyConst {
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
                    RigidTy::Array(erase_bound_regions_in_ty(elem), erase_const(&len))
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
                RigidTy::FnPtr(sig) => RigidTy::FnPtr(sig.map_bound(erase_bound_regions_in_fn_sig)),
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
                                                    rustc_public::ty::TermKind::Const(erase_const(
                                                        &c,
                                                    ))
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
    fn force_no_main_attr() -> bool {
        false
    }

    fn hir_structure(ctx: HirStructureCtx) -> (Self, HirStructure);
    fn emit_mir(&mut self, ctx: HirStructureCtx, def: DefId) -> rustc_public::mir::Body;
}

pub struct InterfaceCallbacks<S: CrateGeneratorState> {
    inner: internal::InterfaceCallbacks<S>,
}

impl<S: CrateGeneratorState> Default for InterfaceCallbacks<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S: CrateGeneratorState> InterfaceCallbacks<S> {
    pub fn new() -> Self {
        Self {
            inner: internal::InterfaceCallbacks::new(),
        }
    }

    pub fn new_without_original_owners() -> Self {
        Self {
            inner: internal::InterfaceCallbacks::new_without_original_owners(),
        }
    }

    pub fn config(&mut self, config: &mut rustc_interface::Config) {
        self.inner.config(config);
    }

    pub fn after_crate_root_parsing(&mut self, krate: &mut rustc_ast::Crate) {
        self.inner.after_crate_root_parsing(krate);
    }

    pub fn after_expansion(&mut self, tcx: TyCtxt<'_>) {
        self.inner.after_expansion(tcx);
    }
}

pub fn generate_with_args<S: CrateGeneratorState>(args: Vec<String>) {
    internal::generate_with_args::<S>(args);
}

pub fn generate_with_args_and_after_analysis<S: CrateGeneratorState>(
    args: Vec<String>,
    after_analysis: Box<dyn for<'tcx> FnOnce(TyCtxt<'tcx>) -> rustc_driver::Compilation + Send>,
) {
    internal::generate_with_args_and_after_analysis::<S>(args, after_analysis);
}

#[derive(Debug)]
pub enum DefData {
    ForeignMod,
    Module(String),
    ValueNs(String),
    TypeNs(String),
    LifetimeNs(String),
    Impl,
    AnonConst,
}
