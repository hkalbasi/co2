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
extern crate rustc_middle;
pub extern crate rustc_public;
extern crate rustc_public_bridge;
extern crate rustc_session;
extern crate rustc_span;
extern crate rustc_target;

mod hir_structure;
mod hir_ty;
mod internal;

pub use hir_structure::{
    ForeignModItem, FunctionAbi, FunctionSignature, HirAdtKind, HirImplItem, HirImplItemKind,
    HirModule, HirModuleItem, HirSelfKind, HirStructure, StructField,
};
pub use hir_ty::{HirGenericArg, HirLifetime, HirTy, HirTyConst, HirTyKind};

/// Summary of crates loaded as dependencies by rustc.
#[derive(Debug, Clone, Default)]
pub struct DependencyInfo {
    pub crates: Vec<DependencyCrate>,
    pub functions: Vec<DependencyFunction>,
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
