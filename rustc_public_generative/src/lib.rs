#![feature(rustc_private)]

//! rustc_public_generative
//!
//! This crate provides a `generate` entrypoint that runs `rustc_driver`
//! but injects a synthetic crate produced by user code.

extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_index;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_session;
extern crate rustc_span;
extern crate rustc_target;
extern crate rustc_data_structures;
extern crate rustc_abi;
extern crate rustc_ast;
extern crate rustc_public;
extern crate rustc_public_bridge;
extern crate rustc_hashes;

use std::path::PathBuf;
use std::hash::Hash;
use std::sync::{Arc, Mutex};
use std::sync::{Condvar, OnceLock};

use rustc_hir as hir;
use rustc_hir::def::{DefKind, Res};
use rustc_hir::def_id::{DefId, LocalDefId, LocalDefIdMap, LocalModDefId, CRATE_DEF_ID, StableCrateId};
use rustc_hir::definitions::{DefPathData, DisambiguatorState, Definitions};
use rustc_hir::{HirId, ItemLocalId, ItemLocalMap, OwnerId};
use rustc_index::{Idx, IndexVec};
use rustc_middle::hir::ModuleItems;
use rustc_middle::query::Providers as QueryProviders;
use rustc_middle::util::Providers as UtilProviders;
use rustc_middle::ty::{self, TyCtxt, AssocKind};
use rustc_middle::mir::ConstValue;
use rustc_middle::mir::interpret::{Pointer, Scalar};
use rustc_middle::mir::{BorrowKind, MutBorrowKind};
use rustc_session::config::EntryFnType;
use rustc_abi::{ExternAbi, HasDataLayout};
use rustc_span::symbol::{Ident, Symbol};
use rustc_span::{BytePos, DUMMY_SP, Span as RustcSpan, SyntaxContext};
use rustc_data_structures::fingerprint::Fingerprint;
use rustc_data_structures::fx::FxHashMap;
use rustc_data_structures::stable_hasher::StableHasher;
use rustc_data_structures::steal::Steal;
use rustc_ast::{IntTy, UintTy, Mutability};
use rustc_middle::middle::codegen_fn_attrs::CodegenFnAttrFlags;
use rustc_hashes::Hash64;

/// Context passed to the user callback. Used for allocating new IDs and
/// registering custom source files.
#[derive(Debug, Clone, Default)]
pub struct Context(Arc<ContextInner>);

#[derive(Debug, Default)]
struct ContextInner {
    next_file_id: std::sync::atomic::AtomicU32,
    custom_files: Mutex<Vec<CustomFile>>,
}

#[derive(Debug, Clone)]
pub struct CustomFile {
    pub path: PathBuf,
    pub contents: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileId(u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    pub file: FileId,
    pub lo: u32,
    pub hi: u32,
}

impl Context {
    pub fn new() -> Self {
        Self(Arc::new(ContextInner {
            next_file_id: std::sync::atomic::AtomicU32::new(1),
            custom_files: Mutex::new(Vec::new()),
        }))
    }

    pub fn add_custom_file(&self, path: impl Into<PathBuf>, contents: impl Into<String>) -> FileId {
        let id = self
            .0
            .next_file_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut guard = self.0.custom_files.lock().unwrap();
        guard.push(CustomFile {
            path: path.into(),
            contents: contents.into(),
        });
        FileId(id)
    }

    pub fn span(&self, file: FileId, lo: u32, hi: u32) -> Span {
        Span { file, lo, hi }
    }

    pub fn take_custom_files(&self) -> Vec<CustomFile> {
        let mut guard = self.0.custom_files.lock().unwrap();
        std::mem::take(&mut *guard)
    }
}

/// Summary of crates loaded as dependencies by rustc.
#[derive(Debug, Clone, Default)]
pub struct DependencyInfo {
    pub crates: Vec<DependencyCrate>,
    pub functions: Vec<DependencyFunction>,
    pub types: Vec<DependencyType>,
    pub std_env_args: Option<FunctionId>,
    pub iter_nth: Option<FunctionId>,
    pub option_unwrap: Option<FunctionId>,
    pub result_unwrap: Option<FunctionId>,
    pub cstring_new: Option<FunctionId>,
    pub cstring_into_raw: Option<FunctionId>,
    pub std_env_args_ty: Option<TypeId>,
    pub string_ty: Option<TypeId>,
    pub cstring_ty: Option<TypeId>,
    pub nul_error_ty: Option<TypeId>,
    pub option_ty: Option<TypeId>,
    pub result_ty: Option<TypeId>,
}

#[derive(Debug, Clone)]
pub struct DependencyCrate {
    pub name: String,
    pub disambiguator: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FunctionId(u64);

impl FunctionId {
    pub fn new(value: u64) -> Self {
        Self(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypeId(u64);

#[derive(Debug, Clone)]
pub struct DependencyFunction {
    pub id: FunctionId,
    pub path: String,
    pub def_path_hash_hi: u64,
    pub def_path_hash_lo: u64,
}

#[derive(Debug, Clone)]
pub struct DependencyType {
    pub id: TypeId,
    pub path: String,
    pub def_path_hash_hi: u64,
    pub def_path_hash_lo: u64,
}

/// User-provided description of the current crate to be generated.
#[derive(Debug, Clone, Default)]
pub struct CurrentCrateInfo {
    pub crate_name: String,
    pub foreign_functions: Vec<ForeignFunctionInfo>,
    pub functions: Vec<FunctionInfo>,
}

#[derive(Debug, Clone)]
pub struct ForeignFunctionInfo {
    pub name: String,
    pub inputs: Vec<MirTy>,
    pub output: MirTy,
    pub id: FunctionId,
}

#[derive(Debug, Clone)]
pub struct FunctionInfo {
    pub name: String,
    pub body: MirBody,
}

#[derive(Debug, Clone)]
pub struct MirBody {
    pub locals: Vec<MirLocalDecl>,
    pub arg_count: usize,
    pub blocks: Vec<MirBasicBlock>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct MirLocalDecl {
    pub ty: MirTy,
    pub mutability: MirMutability,
    pub span: Span,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum MirMutability {
    Not,
    Mut,
}

#[derive(Debug, Clone)]
pub enum MirTy {
    Unit,
    I32,
    Isize,
    Usize,
    U8,
    I8,
    Ptr { mutability: MirMutability, to: Box<MirTy> },
    Ref { mutability: MirMutability, to: Box<MirTy> },
    Adt { id: TypeId, args: Vec<MirTy> },
    FnDef { id: FunctionId },
}

#[derive(Debug, Clone)]
pub struct MirBasicBlock {
    pub statements: Vec<MirStatement>,
    pub terminator: MirTerminator,
}

#[derive(Debug, Clone)]
pub enum MirStatement {
    Assign(MirPlace, MirRvalue),
}

#[derive(Debug, Clone)]
pub enum MirRvalue {
    Use(MirOperand),
    AddressOf { mutability: MirMutability, place: MirPlace },
    Ref { mutability: MirMutability, place: MirPlace },
}

#[derive(Debug, Clone)]
pub enum MirOperand {
    Copy(MirPlace),
    Move(MirPlace),
    Const(MirConst),
}

#[derive(Debug, Clone)]
pub enum MirConst {
    Unit,
    I32(i32),
    Isize(i64),
    Usize(u64),
    ByteStr(Vec<u8>),
    Fn { id: FunctionId, args: Vec<MirTy> },
}

#[derive(Debug, Clone)]
pub struct MirPlace {
    pub local: usize,
    pub projection: Vec<MirProjection>,
}

#[derive(Debug, Clone)]
pub enum MirProjection {
    Deref,
}

#[derive(Debug, Clone)]
pub enum MirTerminator {
    Return,
    Call {
        func: MirOperand,
        args: Vec<MirOperand>,
        destination: Option<(MirPlace, usize)>,
    },
}

/// Run rustc_driver but emit a synthetic crate described by the callback.
pub fn generate<F>(callback: F)
where
    F: FnOnce(Context, DependencyInfo) -> CurrentCrateInfo + Send + 'static,
{
    let mut args: Vec<String> = std::env::args().collect();
    if args.len() == 1 {
        // Provide a dummy crate name if invoked programmatically without args.
        args.push(String::from("rustc"));
        args.push(String::from("--crate-name"));
        args.push(String::from("synthetic"));
        args.push(String::from("--crate-type=bin"));
        args.push(String::from("-"));
    }

    let mut callbacks = GenerateCallbacks::new(callback);
    rustc_driver::run_compiler(&args, &mut callbacks);
}

struct GenerateCallbacks<F> {
    callback: Option<F>,
    context: Context,
    gate: Arc<GenerateGate>,
}

#[derive(Default)]
struct GenerateState {
    generated: Option<GeneratedCrate>,
    original: Option<OriginalProviders>,
    callback: Option<Box<dyn FnOnce(Context, DependencyInfo) -> CurrentCrateInfo + Send>>,
    context: Option<Context>,
    dependency_info: Option<DependencyInfo>,
    building: bool,
    building_thread: Option<std::thread::ThreadId>,
}

struct GenerateGate {
    state: Mutex<GenerateState>,
    cvar: Condvar,
}

#[derive(Copy, Clone)]
struct OriginalProviders {
    hir_crate: for<'tcx> fn(TyCtxt<'tcx>, ()) -> hir::Crate<'tcx>,
    hir_crate_items: for<'tcx> fn(TyCtxt<'tcx>, ()) -> ModuleItems,
    hir_module_items: for<'tcx> fn(TyCtxt<'tcx>, LocalModDefId) -> ModuleItems,
    opt_hir_owner_nodes:
        for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> Option<&'tcx hir::OwnerNodes<'tcx>>,
    hir_owner_parent_q: for<'tcx> fn(TyCtxt<'tcx>, OwnerId) -> HirId,
    entry_fn: for<'tcx> fn(TyCtxt<'tcx>, ()) -> Option<(DefId, EntryFnType)>,
    def_kind: for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> DefKind,
    def_span: for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> RustcSpan,
    def_ident_span: for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> Option<RustcSpan>,
    visibility: for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> ty::Visibility<DefId>,
    generics_of: for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> ty::Generics,
    type_of: for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> ty::EarlyBinder<'tcx, ty::Ty<'tcx>>,
    fn_sig: for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> ty::EarlyBinder<'tcx, ty::PolyFnSig<'tcx>>,
    predicates_of: for<'tcx> fn(TyCtxt<'tcx>, DefId) -> ty::GenericPredicates<'tcx>,
    explicit_predicates_of: for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> ty::GenericPredicates<'tcx>,
    codegen_fn_attrs:
        for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> rustc_middle::middle::codegen_fn_attrs::CodegenFnAttrs,
    mir_built: for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> &'tcx Steal<rustc_middle::mir::Body<'tcx>>,
    mir_for_ctfe: for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> &'tcx rustc_middle::mir::Body<'tcx>,
    mir_drops_elaborated_and_const_checked:
        for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> &'tcx Steal<rustc_middle::mir::Body<'tcx>>,
    optimized_mir: for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> &'tcx rustc_middle::mir::Body<'tcx>,
}

static GENERATE_STATE: OnceLock<Arc<GenerateGate>> = OnceLock::new();

fn with_generated_and_original<'tcx, R>(
    tcx: TyCtxt<'tcx>,
    f: impl FnOnce(Option<&GeneratedCrate>, OriginalProviders) -> R,
) -> R {
    let state = GENERATE_STATE.get().cloned().expect("generate state missing");
    ensure_generated(tcx, &state);
    let (generated_ptr, original) = {
        let guard = state.state.lock().unwrap();
        let original = guard.original.expect("original providers missing");
        let generated_ptr = guard.generated.as_ref().map(|g| g as *const GeneratedCrate);
        (generated_ptr, original)
    };
    if std::env::var("GEN_DEBUG").is_ok() {
        eprintln!(
            "with_generated_and_original: generated={}",
            if generated_ptr.is_some() { "some" } else { "none" }
        );
    }
    let generated: Option<&GeneratedCrate> = generated_ptr.map(|ptr| unsafe { &*ptr });
    f(generated, original)
}

fn ensure_generated<'tcx>(tcx: TyCtxt<'tcx>, gate: &Arc<GenerateGate>) {
    let mut guard = gate.state.lock().unwrap();
    if guard.generated.is_some() {
        return;
    }
    if guard.building {
        if guard.building_thread == Some(std::thread::current().id()) {
            return;
        }
        while guard.generated.is_none() {
            guard = gate.cvar.wait(guard).unwrap();
        }
        return;
    }
    guard.building = true;
    guard.building_thread = Some(std::thread::current().id());
    if std::env::var("GEN_DEBUG").is_ok() {
        eprintln!(
            "ensure_generated: callback is {}",
            if guard.callback.is_some() { "some" } else { "none" }
        );
    }
    let callback = guard.callback.take().expect("callback missing");
    let context = guard.context.clone().expect("context missing");
    drop(guard);

    let dependency_info = collect_dependency_info(tcx);
    let crate_info = callback(context.clone(), dependency_info.clone());
    let generated = GeneratedCrate::build(tcx, &context, dependency_info, crate_info);

    let mut guard = gate.state.lock().unwrap();
    guard.generated = Some(generated);
    guard.building = false;
    guard.building_thread = None;
    gate.cvar.notify_all();
}

impl<F> GenerateCallbacks<F>
where
    F: FnOnce(Context, DependencyInfo) -> CurrentCrateInfo + Send + 'static,
{
    fn new(callback: F) -> Self {
        Self {
            callback: Some(callback),
            context: Context::new(),
            gate: Arc::new(GenerateGate {
                state: Mutex::new(GenerateState::default()),
                cvar: Condvar::new(),
            }),
        }
    }
}

impl<F> rustc_driver::Callbacks for GenerateCallbacks<F>
where
    F: FnOnce(Context, DependencyInfo) -> CurrentCrateInfo + Send + 'static,
{
    fn config(&mut self, config: &mut rustc_interface::Config) {
        let callback = self.callback.take().expect("callback already used");

        if std::env::var("GEN_DEBUG").is_ok() {
            eprintln!("callbacks.config");
        }
        let _ = GENERATE_STATE.set(self.gate.clone());

        config.override_queries = Some(override_queries);

        if let Some(gate) = GENERATE_STATE.get() {
            let mut guard = gate.state.lock().unwrap();
            if std::env::var("GEN_DEBUG").is_ok() {
                eprintln!("callbacks.config: storing callback");
            }
            guard.callback = Some(Box::new(callback));
            guard.context = Some(self.context.clone());
        }
    }
}

fn collect_dependency_info<'tcx>(tcx: rustc_middle::ty::TyCtxt<'tcx>) -> DependencyInfo {
    let mut info = DependencyInfo::default();

    for &krate in tcx.crates(()).iter() {
        let name = tcx.crate_name(krate).to_string();
        let disambiguator = tcx.crate_hash(krate).to_hex();
        info.crates.push(DependencyCrate { name, disambiguator });
    }

    let mut next_fn_id = 1u64;
    let mut alloc_fn_id = || {
        let id = FunctionId(next_fn_id);
        next_fn_id += 1;
        id
    };

    let mut next_ty_id = 1u64;
    let mut alloc_ty_id = || {
        let id = TypeId(next_ty_id);
        next_ty_id += 1;
        id
    };

    let mut register = |id: FunctionId, path: &str, hash: rustc_span::def_id::DefPathHash| {
        let (hi, lo): (u64, u64) = unsafe { std::mem::transmute::<Fingerprint, (u64, u64)>(hash.0) };
        info.functions.push(DependencyFunction {
            id,
            path: path.to_string(),
            def_path_hash_hi: hi,
            def_path_hash_lo: lo,
        });
    };

    let mut register_type = |id: TypeId, path: &str, hash: rustc_span::def_id::DefPathHash| {
        let (hi, lo): (u64, u64) = unsafe { std::mem::transmute::<Fingerprint, (u64, u64)>(hash.0) };
        info.types.push(DependencyType {
            id,
            path: path.to_string(),
            def_path_hash_hi: hi,
            def_path_hash_lo: lo,
        });
    };


    let crates = tcx
        .crates(())
        .iter()
        .map(|&cnum| (tcx.crate_name(cnum), cnum, tcx.stable_crate_id(cnum)))
        .collect::<Vec<_>>();

    let std_crate = crates.iter().find(|(name, ..)| *name == Symbol::intern("std"));
    let core_crate = crates.iter().find(|(name, ..)| *name == Symbol::intern("core"));
    let alloc_crate = crates.iter().find(|(name, ..)| *name == Symbol::intern("alloc"));

    if let Some((_, _cnum, stable_id)) = std_crate {
        if let Some(hash) = resolve_def_path_hash(
            *stable_id,
            &[
                (DefPathData::TypeNs(Symbol::intern("env")), 0),
                (DefPathData::ValueNs(Symbol::intern("args")), 0),
            ],
        ) {
            if tcx.def_path_hash_to_def_id(hash).is_some() {
                let id = alloc_fn_id();
                info.std_env_args = Some(id);
                register(id, "std::env::args", hash);
            }
        }

        if let Some(hash) = resolve_def_path_hash(
            *stable_id,
            &[
                (DefPathData::TypeNs(Symbol::intern("env")), 0),
                (DefPathData::TypeNs(Symbol::intern("Args")), 0),
            ],
        ) {
            if tcx.def_path_hash_to_def_id(hash).is_some() {
                let id = alloc_ty_id();
                info.std_env_args_ty = Some(id);
                register_type(id, "std::env::Args", hash);
            }
        }
    }

    if let Some((_, _cnum, stable_id)) = alloc_crate {
        if let Some(hash) = resolve_def_path_hash(
            *stable_id,
            &[
                (DefPathData::TypeNs(Symbol::intern("ffi")), 0),
                (DefPathData::TypeNs(Symbol::intern("c_str")), 0),
                (DefPathData::TypeNs(Symbol::intern("CString")), 0),
            ],
        ) {
            if let Some(def_id) = tcx.def_path_hash_to_def_id(hash) {
                let id = alloc_ty_id();
                info.cstring_ty = Some(id);
                register_type(id, "alloc::ffi::c_str::CString", hash);

                if let Some(method) = find_inherent_method(tcx, def_id, Symbol::intern("new")) {
                    let fn_id = alloc_fn_id();
                    info.cstring_new = Some(fn_id);
                    register(fn_id, "alloc::ffi::c_str::CString::new", tcx.def_path_hash(method));
                } else if let Some(hash) = resolve_def_path_hash_with_impl(
                    tcx,
                    *stable_id,
                    &[
                        (DefPathData::TypeNs(Symbol::intern("ffi")), 0),
                        (DefPathData::TypeNs(Symbol::intern("c_str")), 0),
                        (DefPathData::TypeNs(Symbol::intern("CString")), 0),
                    ],
                    "new",
                ) {
                    let fn_id = alloc_fn_id();
                    info.cstring_new = Some(fn_id);
                    register(fn_id, "alloc::ffi::c_str::CString::new", hash);
                }

                if let Some(method) = find_inherent_method(tcx, def_id, Symbol::intern("into_raw")) {
                    let fn_id = alloc_fn_id();
                    info.cstring_into_raw = Some(fn_id);
                    register(fn_id, "alloc::ffi::c_str::CString::into_raw", tcx.def_path_hash(method));
                } else if let Some(hash) = resolve_def_path_hash_with_impl(
                    tcx,
                    *stable_id,
                    &[
                        (DefPathData::TypeNs(Symbol::intern("ffi")), 0),
                        (DefPathData::TypeNs(Symbol::intern("c_str")), 0),
                        (DefPathData::TypeNs(Symbol::intern("CString")), 0),
                    ],
                    "into_raw",
                ) {
                    let fn_id = alloc_fn_id();
                    info.cstring_into_raw = Some(fn_id);
                    register(fn_id, "alloc::ffi::c_str::CString::into_raw", hash);
                }
            }
        }

        if let Some(hash) = resolve_def_path_hash(
            *stable_id,
            &[
                (DefPathData::TypeNs(Symbol::intern("ffi")), 0),
                (DefPathData::TypeNs(Symbol::intern("c_str")), 0),
                (DefPathData::TypeNs(Symbol::intern("NulError")), 0),
            ],
        ) {
            if tcx.def_path_hash_to_def_id(hash).is_some() {
                let id = alloc_ty_id();
                info.nul_error_ty = Some(id);
                register_type(id, "alloc::ffi::c_str::NulError", hash);
            }
        }

        if let Some(hash) = resolve_def_path_hash(
            *stable_id,
            &[
                (DefPathData::TypeNs(Symbol::intern("string")), 0),
                (DefPathData::TypeNs(Symbol::intern("String")), 0),
            ],
        ) {
            if tcx.def_path_hash_to_def_id(hash).is_some() {
                let id = alloc_ty_id();
                info.string_ty = Some(id);
                register_type(id, "alloc::string::String", hash);
            }
        }
    }

    if let Some((_, _cnum, stable_id)) = core_crate {
        if let Some(hash) = resolve_def_path_hash(
            *stable_id,
            &[
                (DefPathData::TypeNs(Symbol::intern("iter")), 0),
                (DefPathData::TypeNs(Symbol::intern("traits")), 0),
                (DefPathData::TypeNs(Symbol::intern("iterator")), 0),
                (DefPathData::TypeNs(Symbol::intern("Iterator")), 0),
                (DefPathData::ValueNs(Symbol::intern("nth")), 0),
            ],
        ) {
            if tcx.def_path_hash_to_def_id(hash).is_some() {
                let id = alloc_fn_id();
                info.iter_nth = Some(id);
                register(id, "core::iter::Iterator::nth", hash);
            }
        }
    }

    if let Some(option_def_id) = tcx.lang_items().option_type() {
        let id = alloc_ty_id();
        info.option_ty = Some(id);
        register_type(id, "core::option::Option", tcx.def_path_hash(option_def_id));

        if let Some(method) = find_inherent_method(tcx, option_def_id, Symbol::intern("unwrap")) {
            let fn_id = alloc_fn_id();
            info.option_unwrap = Some(fn_id);
            register(fn_id, "core::option::Option::unwrap", tcx.def_path_hash(method));
        }
    }

    if let Some((_, _cnum, stable_id)) = core_crate {
        if let Some(hash) = resolve_def_path_hash(
            *stable_id,
            &[
                (DefPathData::TypeNs(Symbol::intern("result")), 0),
                (DefPathData::TypeNs(Symbol::intern("Result")), 0),
            ],
        ) {
            if let Some(result_def_id) = tcx.def_path_hash_to_def_id(hash) {
                let id = alloc_ty_id();
                info.result_ty = Some(id);
                register_type(id, "core::result::Result", tcx.def_path_hash(result_def_id));

                if let Some(method) =
                    find_inherent_method(tcx, result_def_id, Symbol::intern("unwrap"))
                {
                    let fn_id = alloc_fn_id();
                    info.result_unwrap = Some(fn_id);
                    register(fn_id, "core::result::Result::unwrap", tcx.def_path_hash(method));
                }
            }
        }
    }

    info
}

fn resolve_def_path_hash(
    stable_crate_id: StableCrateId,
    path: &[(DefPathData, u32)],
) -> Option<rustc_span::def_id::DefPathHash> {
    let mut hash = rustc_span::def_id::DefPathHash::new(
        stable_crate_id,
        Hash64::new(stable_crate_id.as_u64()),
    );

    for (data, disambiguator) in path {
        let mut hasher = StableHasher::new();
        hash.local_hash().hash(&mut hasher);
        std::mem::discriminant(data).hash(&mut hasher);
        if let Some(name) = data.get_opt_name() {
            name.as_str().hash(&mut hasher);
        }
        disambiguator.hash(&mut hasher);
        let local_hash = hasher.finish();
        hash = rustc_span::def_id::DefPathHash::new(hash.stable_crate_id(), local_hash);
    }

    Some(hash)
}

fn resolve_def_path_hash_with_impl<'tcx>(
    tcx: TyCtxt<'tcx>,
    stable_crate_id: StableCrateId,
    base_path: &[(DefPathData, u32)],
    method: &str,
) -> Option<rustc_span::def_id::DefPathHash> {
    for disamb in 0u32..64 {
        let mut path = base_path.to_vec();
        path.push((DefPathData::Impl, disamb));
        path.push((DefPathData::ValueNs(Symbol::intern(method)), 0));
        let hash = resolve_def_path_hash(stable_crate_id, &path)?;
        if tcx.def_path_hash_to_def_id(hash).is_some() {
            return Some(hash);
        }
    }
    None
}

fn def_path_hash_from_parts(hi: u64, lo: u64) -> rustc_span::def_id::DefPathHash {
    rustc_span::def_id::DefPathHash(Fingerprint::new(hi, lo))
}

fn find_inherent_method<'tcx>(
    tcx: TyCtxt<'tcx>,
    def_id: DefId,
    name: Symbol,
) -> Option<DefId> {
    for impl_def in tcx.inherent_impls(def_id).iter().copied() {
        for item in tcx.associated_items(impl_def).in_definition_order() {
            if matches!(item.kind, AssocKind::Fn { .. }) && item.ident(tcx).name == name {
                return Some(item.def_id);
            }
        }
    }
    None
}

fn override_queries(_sess: &rustc_session::Session, providers: &mut UtilProviders) {
    if std::env::var("GEN_DEBUG").is_ok() {
        eprintln!("override_queries");
    }
    if let Some(gate) = GENERATE_STATE.get() {
        override_providers(&mut providers.queries, gate.clone());
    } else if std::env::var("GEN_DEBUG").is_ok() {
        eprintln!("override_queries: no state");
    }
}

fn override_providers(providers: &mut QueryProviders, gate: Arc<GenerateGate>) {
    let mut guard = gate.state.lock().unwrap();
    if guard.original.is_none() {
        guard.original = Some(OriginalProviders {
            hir_crate: providers.hir_crate,
            hir_crate_items: providers.hir_crate_items,
            hir_module_items: providers.hir_module_items,
            opt_hir_owner_nodes: providers.opt_hir_owner_nodes,
            hir_owner_parent_q: providers.hir_owner_parent_q,
            entry_fn: providers.entry_fn,
            def_kind: providers.def_kind,
            def_span: providers.def_span,
            def_ident_span: providers.def_ident_span,
            visibility: providers.visibility,
            generics_of: providers.generics_of,
            type_of: providers.type_of,
            fn_sig: providers.fn_sig,
            predicates_of: providers.predicates_of,
            explicit_predicates_of: providers.explicit_predicates_of,
            codegen_fn_attrs: providers.codegen_fn_attrs,
            mir_built: providers.mir_built,
            mir_for_ctfe: providers.mir_for_ctfe,
            mir_drops_elaborated_and_const_checked: providers.mir_drops_elaborated_and_const_checked,
            optimized_mir: providers.optimized_mir,
        });
    }
    drop(guard);

    providers.hir_crate = generated_hir_crate;
    // Leave hir_crate_items/hir_module_items to the original providers.
    providers.opt_hir_owner_nodes = generated_opt_hir_owner_nodes;
    providers.hir_owner_parent_q = generated_hir_owner_parent_q;
    providers.entry_fn = generated_entry_fn;
    providers.def_kind = generated_def_kind;
    providers.def_span = generated_def_span;
    providers.def_ident_span = generated_def_ident_span;
    providers.visibility = generated_visibility;
    providers.generics_of = generated_generics_of;
    providers.type_of = generated_type_of;
    providers.fn_sig = generated_fn_sig;
    providers.predicates_of = generated_predicates_of;
    providers.explicit_predicates_of = generated_explicit_predicates_of;
    providers.codegen_fn_attrs = generated_codegen_fn_attrs;
    providers.mir_built = generated_mir_built;
    providers.mir_for_ctfe = generated_mir_for_ctfe;
    providers.mir_drops_elaborated_and_const_checked = generated_mir_drops_elaborated_and_const_checked;
    providers.optimized_mir = generated_optimized_mir;
}

fn generated_hir_crate<'tcx>(tcx: TyCtxt<'tcx>, key: ()) -> hir::Crate<'tcx> {
    if std::env::var("GEN_DEBUG").is_ok() {
        eprintln!("generated_hir_crate");
    }
    with_generated_and_original(tcx, |generated, original| {
        if let Some(gen) = generated {
            return gen.hir_crate(tcx, key);
        }
        (original.hir_crate)(tcx, key)
    })
}

fn generated_hir_crate_items<'tcx>(tcx: TyCtxt<'tcx>, key: ()) -> ModuleItems {
    if std::env::var("GEN_DEBUG").is_ok() {
        eprintln!("generated_hir_crate_items");
    }
    with_generated_and_original(tcx, |generated, original| {
        if let Some(gen) = generated {
            return gen.hir_crate_items(tcx, key);
        }
        (original.hir_crate_items)(tcx, key)
    })
}

fn generated_hir_module_items<'tcx>(tcx: TyCtxt<'tcx>, key: LocalModDefId) -> ModuleItems {
    with_generated_and_original(tcx, |generated, original| {
        if let Some(gen) = generated {
            return gen.hir_module_items(tcx, key);
        }
        (original.hir_module_items)(tcx, key)
    })
}

fn generated_opt_hir_owner_nodes<'tcx>(
    tcx: TyCtxt<'tcx>,
    key: LocalDefId,
) -> Option<&'tcx hir::OwnerNodes<'tcx>> {
    if std::env::var("GEN_DEBUG").is_ok() {
        eprintln!("generated_opt_hir_owner_nodes {:?}", key);
    }
    with_generated_and_original(tcx, |generated, original| {
        if let Some(gen) = generated {
            if let Some(nodes) = gen.opt_hir_owner_nodes(tcx, key) {
                return Some(nodes);
            }
            if std::env::var("GEN_DEBUG").is_ok() {
                eprintln!("generated_opt_hir_owner_nodes: fallback to original {:?}", key);
            }
        }
        (original.opt_hir_owner_nodes)(tcx, key)
    })
}

fn generated_hir_owner_parent_q<'tcx>(tcx: TyCtxt<'tcx>, key: OwnerId) -> HirId {
    with_generated_and_original(tcx, |generated, original| {
        if let Some(gen) = generated {
            return gen.hir_owner_parent_q(tcx, key);
        }
        (original.hir_owner_parent_q)(tcx, key)
    })
}

fn generated_entry_fn<'tcx>(tcx: TyCtxt<'tcx>, key: ()) -> Option<(DefId, EntryFnType)> {
    with_generated_and_original(tcx, |generated, original| {
        if let Some(gen) = generated {
            return gen.entry_fn(tcx, key);
        }
        (original.entry_fn)(tcx, key)
    })
}

fn generated_def_kind<'tcx>(tcx: TyCtxt<'tcx>, key: LocalDefId) -> DefKind {
    if std::env::var("GEN_DEBUG").is_ok() {
        eprintln!("generated_def_kind {:?}", key);
    }
    with_generated_and_original(tcx, |generated, original| {
        if let Some(gen) = generated {
            if let Some(kind) = gen.def_kind(key) {
                return kind;
            }
        }
        (original.def_kind)(tcx, key)
    })
}

fn generated_def_span<'tcx>(tcx: TyCtxt<'tcx>, key: LocalDefId) -> RustcSpan {
    with_generated_and_original(tcx, |generated, original| {
        if let Some(gen) = generated {
            if let Some(span) = gen.def_span(key) {
                return span;
            }
        }
        (original.def_span)(tcx, key)
    })
}

fn generated_def_ident_span<'tcx>(tcx: TyCtxt<'tcx>, key: LocalDefId) -> Option<RustcSpan> {
    with_generated_and_original(tcx, |generated, original| {
        if let Some(gen) = generated {
            if let Some(span) = gen.def_span(key) {
                return Some(span);
            }
        }
        (original.def_ident_span)(tcx, key)
    })
}

fn generated_visibility<'tcx>(tcx: TyCtxt<'tcx>, key: LocalDefId) -> ty::Visibility<DefId> {
    with_generated_and_original(tcx, |generated, original| {
        if let Some(gen) = generated {
            if gen.def_kinds.contains_key(&key) {
                return ty::Visibility::Public;
            }
        }
        (original.visibility)(tcx, key)
    })
}

fn generated_generics_of<'tcx>(tcx: TyCtxt<'tcx>, key: LocalDefId) -> ty::Generics {
    with_generated_and_original(tcx, |generated, original| {
        if let Some(gen) = generated {
            if let Some(generics) = gen.generics_of(tcx, key) {
                return generics;
            }
        }
        (original.generics_of)(tcx, key)
    })
}

fn generated_type_of<'tcx>(
    tcx: TyCtxt<'tcx>,
    key: LocalDefId,
) -> ty::EarlyBinder<'tcx, ty::Ty<'tcx>> {
    with_generated_and_original(tcx, |generated, original| {
        if let Some(gen) = generated {
            if let Some(ty) = gen.type_of(tcx, key) {
                return ty;
            }
        }
        (original.type_of)(tcx, key)
    })
}

fn generated_fn_sig<'tcx>(
    tcx: TyCtxt<'tcx>,
    key: LocalDefId,
) -> ty::EarlyBinder<'tcx, ty::PolyFnSig<'tcx>> {
    with_generated_and_original(tcx, |generated, original| {
        if let Some(gen) = generated {
            if let Some(sig) = gen.fn_sig(tcx, key) {
                return sig;
            }
        }
        (original.fn_sig)(tcx, key)
    })
}

fn generated_predicates_of<'tcx>(
    tcx: TyCtxt<'tcx>,
    key: DefId,
) -> ty::GenericPredicates<'tcx> {
    with_generated_and_original(tcx, |generated, original| {
        if let Some(gen) = generated {
            if let Some(preds) = gen.predicates_of(tcx, key) {
                return preds;
            }
        }
        (original.predicates_of)(tcx, key)
    })
}

fn generated_explicit_predicates_of<'tcx>(
    tcx: TyCtxt<'tcx>,
    key: LocalDefId,
) -> ty::GenericPredicates<'tcx> {
    with_generated_and_original(tcx, |generated, original| {
        if let Some(gen) = generated {
            if let Some(preds) = gen.explicit_predicates_of(tcx, key) {
                return preds;
            }
        }
        (original.explicit_predicates_of)(tcx, key)
    })
}

fn generated_codegen_fn_attrs<'tcx>(
    tcx: TyCtxt<'tcx>,
    key: LocalDefId,
) -> rustc_middle::middle::codegen_fn_attrs::CodegenFnAttrs {
    with_generated_and_original(tcx, |generated, original| {
        if let Some(gen) = generated {
            if let Some(attrs) = gen.codegen_fn_attrs(tcx, key) {
                return attrs;
            }
        }
        (original.codegen_fn_attrs)(tcx, key)
    })
}

fn generated_mir_built<'tcx>(
    tcx: TyCtxt<'tcx>,
    key: LocalDefId,
) -> &'tcx Steal<rustc_middle::mir::Body<'tcx>> {
    with_generated_and_original(tcx, |generated, original| {
        if let Some(gen) = generated {
            if let Some(body) = gen.mir_built(tcx, key) {
                return body;
            }
        }
        (original.mir_built)(tcx, key)
    })
}

fn generated_mir_for_ctfe<'tcx>(
    tcx: TyCtxt<'tcx>,
    key: LocalDefId,
) -> &'tcx rustc_middle::mir::Body<'tcx> {
    with_generated_and_original(tcx, |generated, original| {
        if let Some(gen) = generated {
            if let Some(body) = gen.mir_for_ctfe(tcx, key) {
                return body;
            }
        }
        (original.mir_for_ctfe)(tcx, key)
    })
}

fn generated_mir_drops_elaborated_and_const_checked<'tcx>(
    tcx: TyCtxt<'tcx>,
    key: LocalDefId,
) -> &'tcx Steal<rustc_middle::mir::Body<'tcx>> {
    with_generated_and_original(tcx, |generated, original| {
        if let Some(gen) = generated {
            if let Some(body) = gen.mir_drops_elaborated_and_const_checked(tcx, key) {
                return body;
            }
        }
        (original.mir_drops_elaborated_and_const_checked)(tcx, key)
    })
}

fn generated_optimized_mir<'tcx>(
    tcx: TyCtxt<'tcx>,
    key: LocalDefId,
) -> &'tcx rustc_middle::mir::Body<'tcx> {
    with_generated_and_original(tcx, |generated, original| {
        if let Some(gen) = generated {
            if let Some(body) = gen.optimized_mir(tcx, key) {
                return body;
            }
        }
        (original.optimized_mir)(tcx, key)
    })
}

#[allow(invalid_reference_casting)]
fn allocate_def_ids<'tcx>(
    tcx: TyCtxt<'tcx>,
    foreign_functions: &[ForeignFunctionInfo],
) -> (LocalDefId, LocalDefId, FxHashMap<FunctionId, LocalDefId>) {
    let defs_guard = tcx.definitions_untracked();
    let defs_mut = unsafe { &mut *(&*defs_guard as *const Definitions as *mut Definitions) };
    let mut disamb = DisambiguatorState::with(
        CRATE_DEF_ID,
        DefPathData::ValueNs(Symbol::intern("main")),
        1,
    );
    let foreign_mod = defs_mut.create_def(CRATE_DEF_ID, DefPathData::ForeignMod, &mut disamb);
    let main_def =
        defs_mut.create_def(CRATE_DEF_ID, DefPathData::ValueNs(Symbol::intern("main")), &mut disamb);
    let mut foreign_function_ids = FxHashMap::default();
    for foreign in foreign_functions {
        let def_id = defs_mut.create_def(
            foreign_mod,
            DefPathData::ValueNs(Symbol::intern(&foreign.name)),
            &mut disamb,
        );
        foreign_function_ids.insert(foreign.id, def_id);
    }
    (foreign_mod, main_def, foreign_function_ids)
}

struct GeneratedCrate {
    #[allow(dead_code)]
    crate_name: Symbol,
    context: Context,
    dep_functions: FxHashMap<FunctionId, DefId>,
    dep_types: FxHashMap<TypeId, DefId>,
    foreign_function_ids: FxHashMap<FunctionId, LocalDefId>,
    foreign_function_infos: LocalDefIdMap<ForeignFunctionInfo>,
    foreign_function_symbols: LocalDefIdMap<Symbol>,
    owners: IndexVec<LocalDefId, Option<&'static hir::OwnerInfo<'static>>>,
    crate_items: ModuleItemsSpec,
    module_items: LocalDefIdMap<ModuleItemsSpec>,
    owner_parents: LocalDefIdMap<HirId>,
    def_kinds: LocalDefIdMap<DefKind>,
    def_spans: LocalDefIdMap<RustcSpan>,
    function_infos: LocalDefIdMap<FunctionInfo>,
    function_mir: Mutex<LocalDefIdMap<&'static Steal<rustc_middle::mir::Body<'static>>>>,
    entry_fn: Option<LocalDefId>,
}

impl GeneratedCrate {
    fn build<'tcx>(
        tcx: TyCtxt<'tcx>,
        context: &Context,
        dependency_info: DependencyInfo,
        info: CurrentCrateInfo,
    ) -> Self {
        if std::env::var("GEN_DEBUG").is_ok() {
            eprintln!("GeneratedCrate::build");
        }
        let crate_name = if info.crate_name.is_empty() {
            Symbol::intern("synthetic")
        } else {
            Symbol::intern(&info.crate_name)
        };

        let mut dep_functions = FxHashMap::default();
        for func in &dependency_info.functions {
            let hash = def_path_hash_from_parts(func.def_path_hash_hi, func.def_path_hash_lo);
            if let Some(def_id) = tcx.def_path_hash_to_def_id(hash) {
                dep_functions.insert(func.id, def_id);
            }
        }

        let mut dep_types = FxHashMap::default();
        for ty in &dependency_info.types {
            let hash = def_path_hash_from_parts(ty.def_path_hash_hi, ty.def_path_hash_lo);
            if let Some(def_id) = tcx.def_path_hash_to_def_id(hash) {
                dep_types.insert(ty.id, def_id);
            }
        }

        let mut owners: IndexVec<LocalDefId, Option<&'static hir::OwnerInfo<'static>>> = IndexVec::new();
        let mut owner_parents: LocalDefIdMap<HirId> = LocalDefIdMap::default();
        let mut def_kinds: LocalDefIdMap<DefKind> = LocalDefIdMap::default();
        let mut def_spans: LocalDefIdMap<RustcSpan> = LocalDefIdMap::default();
        let mut function_infos: LocalDefIdMap<FunctionInfo> = LocalDefIdMap::default();
        let mut foreign_function_infos: LocalDefIdMap<ForeignFunctionInfo> = LocalDefIdMap::default();
        let mut foreign_function_symbols: LocalDefIdMap<Symbol> = LocalDefIdMap::default();

        let crate_def = CRATE_DEF_ID;
        let (foreign_mod_def, main_def, foreign_function_ids) =
            allocate_def_ids(tcx, &info.foreign_functions);

        def_kinds.insert(crate_def, DefKind::Mod);
        def_kinds.insert(foreign_mod_def, DefKind::ForeignMod);
        def_kinds.insert(main_def, DefKind::Fn);

        def_spans.insert(crate_def, DUMMY_SP);
        def_spans.insert(foreign_mod_def, DUMMY_SP);
        def_spans.insert(main_def, DUMMY_SP);

        let mut foreign_item_ids = Vec::new();
        let mut foreign_items_hir = Vec::new();

        for foreign in &info.foreign_functions {
            let def_id = *foreign_function_ids
                .get(&foreign.id)
                .expect("foreign function id missing");
            def_kinds.insert(def_id, DefKind::Fn);
            def_spans.insert(def_id, DUMMY_SP);
            foreign_function_infos.insert(def_id, foreign.clone());
            foreign_function_symbols.insert(def_id, Symbol::intern(&foreign.name));

            let foreign_item_id = hir::ForeignItemId { owner_id: OwnerId { def_id } };
            foreign_item_ids.push(foreign_item_id);

            let inputs: Vec<hir::Ty<'static>> = foreign
                .inputs
                .iter()
                .map(|ty| mir_ty_to_hir(def_id, ty))
                .collect();
            let output = leak(mir_ty_to_hir(def_id, &foreign.output));
            let fn_decl = leak(hir::FnDecl {
                inputs: leak(inputs.into_boxed_slice()),
                output: hir::FnRetTy::Return(output),
                c_variadic: false,
                implicit_self: hir::ImplicitSelfKind::None,
                lifetime_elision_allowed: true,
            });

            let fn_sig = hir::FnSig {
                header: hir::FnHeader {
                    safety: hir::HeaderSafety::Normal(hir::Safety::Unsafe),
                    constness: hir::Constness::NotConst,
                    asyncness: hir::IsAsync::NotAsync,
                    abi: ExternAbi::C { unwind: false },
                },
                decl: fn_decl,
                span: DUMMY_SP,
            };

            let foreign_item = hir::ForeignItem {
                ident: Ident::from_str(&foreign.name),
                kind: hir::ForeignItemKind::Fn(
                    fn_sig,
                    leak(vec![None; foreign.inputs.len()].into_boxed_slice()),
                    hir::Generics::empty(),
                ),
                owner_id: OwnerId { def_id },
                span: DUMMY_SP,
                vis_span: DUMMY_SP,
                has_delayed_lints: false,
            };
            foreign_items_hir.push((def_id, leak(foreign_item)));
        }

        let foreign_items = leak(foreign_item_ids.clone().into_boxed_slice());

        let foreign_mod_item = hir::Item {
            owner_id: OwnerId { def_id: foreign_mod_def },
            kind: hir::ItemKind::ForeignMod { abi: ExternAbi::C { unwind: false }, items: foreign_items },
            span: DUMMY_SP,
            vis_span: DUMMY_SP,
            has_delayed_lints: false,
            eii: false,
        };
        let foreign_mod_item = leak(foreign_mod_item);

        let main_ident = Ident::from_str("main");
        let main_fn_decl = leak(hir::FnDecl {
            inputs: &[],
            output: hir::FnRetTy::DefaultReturn(DUMMY_SP),
            c_variadic: false,
            implicit_self: hir::ImplicitSelfKind::None,
            lifetime_elision_allowed: true,
        });
        let main_fn_sig = hir::FnSig {
            header: hir::FnHeader {
                safety: hir::HeaderSafety::Normal(hir::Safety::Safe),
                constness: hir::Constness::NotConst,
                asyncness: hir::IsAsync::NotAsync,
                abi: ExternAbi::Rust,
            },
            decl: main_fn_decl,
            span: DUMMY_SP,
        };

        let main_body_expr = leak(hir::Expr {
            hir_id: HirId { owner: OwnerId { def_id: main_def }, local_id: ItemLocalId::new(1) },
            kind: hir::ExprKind::Tup(&[]),
            span: DUMMY_SP,
        });
        let main_body = leak(hir::Body { params: &[], value: main_body_expr });
        let main_body_id = main_body.id();

        let main_item = hir::Item {
            owner_id: OwnerId { def_id: main_def },
            kind: hir::ItemKind::Fn {
                sig: main_fn_sig,
                ident: main_ident,
                generics: hir::Generics::empty(),
                body: main_body_id,
                has_body: true,
            },
            span: DUMMY_SP,
            vis_span: DUMMY_SP,
            has_delayed_lints: false,
            eii: false,
        };
        let main_item = leak(main_item);

        let root_mod = leak(hir::Mod {
            spans: hir::ModSpans { inner_span: DUMMY_SP, inject_use_span: DUMMY_SP },
            item_ids: leak(
                vec![
                    hir::ItemId { owner_id: OwnerId { def_id: foreign_mod_def } },
                    hir::ItemId { owner_id: OwnerId { def_id: main_def } },
                ]
                .into_boxed_slice(),
            ),
        });

        let crate_owner_nodes = build_owner_nodes_for_crate(root_mod);
        insert_owner(&mut owners, crate_def, leak(make_owner_info(crate_owner_nodes)));
        owner_parents.insert(crate_def, HirId::make_owner(crate_def));

        let foreign_mod_nodes = build_owner_nodes_for_item(foreign_mod_item);
        insert_owner(&mut owners, foreign_mod_def, leak(make_owner_info(foreign_mod_nodes)));
        owner_parents.insert(foreign_mod_def, HirId::make_owner(crate_def));

        for (def_id, foreign_item) in foreign_items_hir {
            let foreign_nodes = build_owner_nodes_for_foreign_item(foreign_item);
            insert_owner(&mut owners, def_id, leak(make_owner_info(foreign_nodes)));
            owner_parents.insert(def_id, HirId::make_owner(foreign_mod_def));
        }

        let main_nodes = build_owner_nodes_for_fn(main_item, main_body, main_body_expr);
        insert_owner(&mut owners, main_def, leak(make_owner_info(main_nodes)));
        owner_parents.insert(main_def, HirId::make_owner(crate_def));

        let crate_items = ModuleItemsSpec {
            add_root: true,
            free_items: vec![
                hir::ItemId { owner_id: OwnerId { def_id: foreign_mod_def } },
                hir::ItemId { owner_id: OwnerId { def_id: main_def } },
            ],
            foreign_items: foreign_item_ids.clone(),
            body_owners: vec![main_def],
            delayed_lint_items: Vec::new(),
        };

        let module_items_for_root = ModuleItemsSpec {
            add_root: false,
            free_items: vec![
                hir::ItemId { owner_id: OwnerId { def_id: foreign_mod_def } },
                hir::ItemId { owner_id: OwnerId { def_id: main_def } },
            ],
            foreign_items: foreign_item_ids.clone(),
            body_owners: vec![main_def],
            delayed_lint_items: Vec::new(),
        };

        let mut module_items = LocalDefIdMap::default();
        module_items.insert(crate_def, module_items_for_root.clone());

        if let Some(first) = info.functions.first() {
            function_infos.insert(main_def, first.clone());
        }

        if std::env::var("GEN_DEBUG").is_ok() {
            eprintln!("GeneratedCrate::build: owners len {}", owners.len());
        }

        Self {
            crate_name,
            context: context.clone(),
            dep_functions,
            dep_types,
            foreign_function_ids,
            foreign_function_infos,
            foreign_function_symbols,
            owners,
            crate_items,
            module_items,
            owner_parents,
            def_kinds,
            def_spans,
            function_infos,
            function_mir: Mutex::new(LocalDefIdMap::default()),
            entry_fn: Some(main_def),
        }
    }

    fn hir_crate<'tcx>(&self, _tcx: TyCtxt<'tcx>, _key: ()) -> hir::Crate<'tcx> {
        let owners: IndexVec<LocalDefId, hir::MaybeOwner<'tcx>> = IndexVec::from_iter(
            self.owners.iter().map(|opt| match opt {
                Some(info) => {
                    let info = unsafe {
                        std::mem::transmute::<&'static hir::OwnerInfo<'static>, &'tcx hir::OwnerInfo<'tcx>>(*info)
                    };
                    hir::MaybeOwner::Owner(info)
                }
                None => hir::MaybeOwner::Phantom,
            }),
        );
        hir::Crate { owners, opt_hir_hash: None }
    }

    fn hir_crate_items<'tcx>(&self, _tcx: TyCtxt<'tcx>, _key: ()) -> ModuleItems {
        make_module_items(self.crate_items.clone())
    }

    fn hir_module_items<'tcx>(&self, _tcx: TyCtxt<'tcx>, key: LocalModDefId) -> ModuleItems {
        let local: LocalDefId = key.into();
        if let Some(items) = self.module_items.get(&local) {
            return make_module_items(items.clone());
        }
        make_module_items(ModuleItemsSpec {
            add_root: false,
            free_items: Vec::new(),
            foreign_items: Vec::new(),
            body_owners: Vec::new(),
            delayed_lint_items: Vec::new(),
        })
    }

    fn opt_hir_owner_nodes<'tcx>(
        &self,
        _tcx: TyCtxt<'tcx>,
        key: LocalDefId,
    ) -> Option<&'tcx hir::OwnerNodes<'tcx>> {
        self.owners
            .get(key)
            .and_then(|opt| *opt)
            .map(|info| unsafe {
                if std::env::var("GEN_DEBUG").is_ok() {
                    eprintln!("gen.opt_hir_owner_nodes {:?} => {:?}", key, info.nodes.node());
                }
                std::mem::transmute::<&hir::OwnerNodes<'static>, &'tcx hir::OwnerNodes<'tcx>>(&info.nodes)
            })
            .or_else(|| {
                if std::env::var("GEN_DEBUG").is_ok() {
                    eprintln!(
                        "gen.opt_hir_owner_nodes {:?} => None (len {})",
                        key,
                        self.owners.len()
                    );
                }
                None
            })
    }

    fn hir_owner_parent_q<'tcx>(&self, _tcx: TyCtxt<'tcx>, key: OwnerId) -> HirId {
        self.owner_parents
            .get(&key.def_id)
            .copied()
            .unwrap_or_else(|| HirId::make_owner(key.def_id))
    }

    fn entry_fn<'tcx>(&self, _tcx: TyCtxt<'tcx>, _key: ()) -> Option<(DefId, EntryFnType)> {
        self.entry_fn.map(|def| {
            (
                def.to_def_id(),
                EntryFnType::Main {
                    sigpipe: rustc_session::config::sigpipe::DEFAULT,
                },
            )
        })
    }

    fn def_kind(&self, def_id: LocalDefId) -> Option<DefKind> {
        self.def_kinds.get(&def_id).copied()
    }

    fn def_span(&self, def_id: LocalDefId) -> Option<RustcSpan> {
        self.def_spans.get(&def_id).copied()
    }

    fn generics_of<'tcx>(&self, _tcx: TyCtxt<'tcx>, def_id: LocalDefId) -> Option<ty::Generics> {
        if self.def_kinds.contains_key(&def_id) {
            let generics = ty::Generics {
                parent: None,
                parent_count: 0,
                own_params: Vec::new(),
                param_def_id_to_index: FxHashMap::default(),
                has_self: false,
                has_late_bound_regions: None,
            };
            return Some(generics);
        }
        None
    }

    fn type_of<'tcx>(
        &self,
        tcx: TyCtxt<'tcx>,
        def_id: LocalDefId,
    ) -> Option<ty::EarlyBinder<'tcx, ty::Ty<'tcx>>> {
        if Some(def_id) == self.entry_fn || self.foreign_function_infos.contains_key(&def_id) {
            let args = ty::GenericArgs::identity_for_item(tcx, def_id);
            let ty = ty::Ty::new_fn_def(tcx, def_id.to_def_id(), args);
            return Some(ty::EarlyBinder::bind(ty));
        }
        None
    }

    fn fn_sig<'tcx>(&self, tcx: TyCtxt<'tcx>, def_id: LocalDefId) -> Option<ty::EarlyBinder<'tcx, ty::PolyFnSig<'tcx>>> {
        if Some(def_id) == self.entry_fn {
            let sig = tcx.mk_fn_sig(
                Vec::new(),
                tcx.types.unit,
                false,
                hir::Safety::Safe,
                ExternAbi::Rust,
            );
            let poly = ty::Binder::dummy(sig);
            return Some(ty::EarlyBinder::bind(poly));
        }
        if let Some(info) = self.foreign_function_infos.get(&def_id) {
            let resolver = MirResolver {
                dep_functions: &self.dep_functions,
                foreign_functions: &self.foreign_function_ids,
                dep_types: &self.dep_types,
            };
            let inputs = info
                .inputs
                .iter()
                .map(|ty| mir_ty_to_rustc(tcx, &resolver, ty))
                .collect::<Vec<_>>();
            let output = mir_ty_to_rustc(tcx, &resolver, &info.output);
            let sig = tcx.mk_fn_sig(
                inputs,
                output,
                false,
                hir::Safety::Unsafe,
                ExternAbi::C { unwind: false },
            );
            let poly = ty::Binder::dummy(sig);
            return Some(ty::EarlyBinder::bind(poly));
        }
        None
    }

    fn predicates_of<'tcx>(&self, tcx: TyCtxt<'tcx>, def_id: DefId) -> Option<ty::GenericPredicates<'tcx>> {
        let local = def_id.as_local()?;
        if self.def_kinds.contains_key(&local) {
            return Some(ty::GenericPredicates {
                parent: None,
                predicates: tcx.arena.alloc_from_iter([]),
            });
        }
        None
    }

    fn explicit_predicates_of<'tcx>(
        &self,
        tcx: TyCtxt<'tcx>,
        def_id: LocalDefId,
    ) -> Option<ty::GenericPredicates<'tcx>> {
        self.predicates_of(tcx, def_id.to_def_id())
    }

    fn codegen_fn_attrs<'tcx>(
        &self,
        _tcx: TyCtxt<'tcx>,
        def_id: LocalDefId,
    ) -> Option<rustc_middle::middle::codegen_fn_attrs::CodegenFnAttrs> {
        if self.def_kinds.contains_key(&def_id) {
            let mut attrs = rustc_middle::middle::codegen_fn_attrs::CodegenFnAttrs::new();
            if let Some(symbol) = self.foreign_function_symbols.get(&def_id) {
                attrs.flags.insert(CodegenFnAttrFlags::NO_MANGLE);
                attrs.symbol_name = Some(*symbol);
            }
            return Some(attrs);
        }
        None
    }

    fn mir_built<'tcx>(
        &self,
        tcx: TyCtxt<'tcx>,
        def_id: LocalDefId,
    ) -> Option<&'tcx Steal<rustc_middle::mir::Body<'tcx>>> {
        {
            let guard = self.function_mir.lock().unwrap();
            if let Some(body) = guard.get(&def_id) {
                return Some(unsafe {
                    std::mem::transmute::<
                        &'static Steal<rustc_middle::mir::Body<'static>>,
                        &'tcx Steal<rustc_middle::mir::Body<'tcx>>,
                    >(*body)
                });
            }
        }

        let info = self.function_infos.get(&def_id)?;
        let resolver = MirResolver {
            dep_functions: &self.dep_functions,
            foreign_functions: &self.foreign_function_ids,
            dep_types: &self.dep_types,
        };
        let body = build_mir_body(tcx, &self.context, info, &resolver, def_id);
        let steal = leak(Steal::new(body));

        let mut guard = self.function_mir.lock().unwrap();
        guard.insert(def_id, steal);
        Some(unsafe {
            std::mem::transmute::<
                &'static Steal<rustc_middle::mir::Body<'static>>,
                &'tcx Steal<rustc_middle::mir::Body<'tcx>>,
            >(steal)
        })
    }

    fn mir_for_ctfe<'tcx>(
        &self,
        tcx: TyCtxt<'tcx>,
        def_id: LocalDefId,
    ) -> Option<&'tcx rustc_middle::mir::Body<'tcx>> {
        let info = self.function_infos.get(&def_id)?;
        let resolver = MirResolver {
            dep_functions: &self.dep_functions,
            foreign_functions: &self.foreign_function_ids,
            dep_types: &self.dep_types,
        };
        let mut body = build_mir_body(tcx, &self.context, info, &resolver, def_id);
        body.set_required_consts(Vec::new());
        body.set_mentioned_items(Vec::new());
        let owned = unsafe {
            std::mem::transmute::<rustc_middle::mir::Body<'static>, rustc_middle::mir::Body<'tcx>>(body)
        };
        Some(tcx.arena.alloc(owned))
    }

    fn mir_drops_elaborated_and_const_checked<'tcx>(
        &self,
        tcx: TyCtxt<'tcx>,
        def_id: LocalDefId,
    ) -> Option<&'tcx Steal<rustc_middle::mir::Body<'tcx>>> {
        self.mir_built(tcx, def_id)
    }

    fn optimized_mir<'tcx>(
        &self,
        tcx: TyCtxt<'tcx>,
        def_id: LocalDefId,
    ) -> Option<&'tcx rustc_middle::mir::Body<'tcx>> {
        self.mir_for_ctfe(tcx, def_id)
    }
}

fn build_owner_nodes_for_crate(root: &'static hir::Mod<'static>) -> hir::OwnerNodes<'static> {
    let mut nodes: IndexVec<ItemLocalId, hir::ParentedNode<'static>> = IndexVec::new();
    nodes.push(hir::ParentedNode {
        parent: ItemLocalId::INVALID,
        node: hir::Node::Crate(root),
    });

    hir::OwnerNodes {
        opt_hash_including_bodies: None,
        nodes,
        bodies: rustc_data_structures::sorted_map::SortedMap::new(),
    }
}

fn build_owner_nodes_for_item(item: &'static hir::Item<'static>) -> hir::OwnerNodes<'static> {
    let mut nodes: IndexVec<ItemLocalId, hir::ParentedNode<'static>> = IndexVec::new();
    nodes.push(hir::ParentedNode {
        parent: ItemLocalId::INVALID,
        node: hir::Node::Item(item),
    });

    hir::OwnerNodes {
        opt_hash_including_bodies: None,
        nodes,
        bodies: rustc_data_structures::sorted_map::SortedMap::new(),
    }
}

fn build_owner_nodes_for_foreign_item(
    item: &'static hir::ForeignItem<'static>,
) -> hir::OwnerNodes<'static> {
    let mut nodes: IndexVec<ItemLocalId, hir::ParentedNode<'static>> = IndexVec::new();
    nodes.push(hir::ParentedNode {
        parent: ItemLocalId::INVALID,
        node: hir::Node::ForeignItem(item),
    });

    hir::OwnerNodes {
        opt_hash_including_bodies: None,
        nodes,
        bodies: rustc_data_structures::sorted_map::SortedMap::new(),
    }
}

fn build_owner_nodes_for_fn(
    item: &'static hir::Item<'static>,
    body: &'static hir::Body<'static>,
    expr: &'static hir::Expr<'static>,
) -> hir::OwnerNodes<'static> {
    let mut nodes: IndexVec<ItemLocalId, hir::ParentedNode<'static>> = IndexVec::new();
    nodes.push(hir::ParentedNode {
        parent: ItemLocalId::INVALID,
        node: hir::Node::Item(item),
    });
    nodes.push(hir::ParentedNode {
        parent: ItemLocalId::ZERO,
        node: hir::Node::Expr(expr),
    });

    let mut bodies = rustc_data_structures::sorted_map::SortedMap::new();
    bodies.insert(ItemLocalId::new(1), body);

    hir::OwnerNodes { opt_hash_including_bodies: None, nodes, bodies }
}

struct MirResolver<'a> {
    dep_functions: &'a FxHashMap<FunctionId, DefId>,
    foreign_functions: &'a FxHashMap<FunctionId, LocalDefId>,
    dep_types: &'a FxHashMap<TypeId, DefId>,
}

impl MirResolver<'_> {
    fn resolve_fn(&self, id: FunctionId) -> DefId {
        if let Some(def_id) = self.dep_functions.get(&id) {
            return *def_id;
        }
        if let Some(local) = self.foreign_functions.get(&id) {
            return local.to_def_id();
        }
        panic!("missing function id {:?}", id);
    }

    fn resolve_ty(&self, id: TypeId) -> DefId {
        if let Some(def_id) = self.dep_types.get(&id) {
            return *def_id;
        }
        panic!("missing type id {:?}", id);
    }
}

fn build_mir_body<'tcx>(
    tcx: TyCtxt<'tcx>,
    context: &Context,
    func: &FunctionInfo,
    resolver: &MirResolver<'_>,
    owner: LocalDefId,
) -> rustc_middle::mir::Body<'static> {
    let span = to_rustc_span(context, func.body.span);
    let source_scope = rustc_middle::mir::SourceScope::from_usize(0);
    let source_scopes = IndexVec::from_iter([rustc_middle::mir::SourceScopeData {
        span,
        parent_scope: None,
        inlined: None,
        inlined_parent_scope: None,
        local_data: rustc_middle::mir::ClearCrossCrate::Set(
            rustc_middle::mir::SourceScopeLocalData {
                lint_root: HirId::make_owner(owner),
            },
        ),
    }]);

    let locals: Vec<rustc_middle::mir::LocalDecl<'tcx>> = func
        .body
        .locals
        .iter()
        .map(|local| rustc_middle::mir::LocalDecl {
            mutability: match local.mutability {
                MirMutability::Not => rustc_middle::mir::Mutability::Not,
                MirMutability::Mut => rustc_middle::mir::Mutability::Mut,
            },
            local_info: rustc_middle::mir::ClearCrossCrate::Set(Box::new(
                rustc_middle::mir::LocalInfo::Boring,
            )),
            ty: mir_ty_to_rustc(tcx, resolver, &local.ty),
            user_ty: None,
            source_info: rustc_middle::mir::SourceInfo { span, scope: source_scope },
        })
        .collect();

    let mut blocks = Vec::new();
    for block in &func.body.blocks {
        let mut statements = Vec::new();
        for stmt in &block.statements {
            match stmt {
                MirStatement::Assign(place, rvalue) => {
                    let place = mir_place_to_rustc(tcx, place);
                    let rvalue = mir_rvalue_to_rustc(tcx, resolver, rvalue);
                    statements.push(rustc_middle::mir::Statement::new(
                        rustc_middle::mir::SourceInfo { span, scope: source_scope },
                        rustc_middle::mir::StatementKind::Assign(Box::new((place, rvalue))),
                    ));
                }
            }
        }

        let terminator = match &block.terminator {
            MirTerminator::Return => rustc_middle::mir::Terminator {
                source_info: rustc_middle::mir::SourceInfo { span, scope: source_scope },
                kind: rustc_middle::mir::TerminatorKind::Return,
            },
            MirTerminator::Call { func, args, destination } => {
                let func = mir_operand_to_rustc(tcx, resolver, func);
                let args: Box<[rustc_span::source_map::Spanned<rustc_middle::mir::Operand<'tcx>>]> = args
                    .iter()
                    .map(|arg| rustc_span::source_map::Spanned {
                        node: mir_operand_to_rustc(tcx, resolver, arg),
                        span,
                    })
                    .collect::<Vec<_>>()
                    .into_boxed_slice();
                let dest = destination.as_ref().map(|(place, bb)| {
                    (
                        mir_place_to_rustc(tcx, place),
                        rustc_middle::mir::BasicBlock::from_usize(*bb),
                    )
                });
                let (destination, target) = dest.unwrap_or((
                    rustc_middle::mir::Place::return_place(),
                    rustc_middle::mir::BasicBlock::from_usize(0),
                ));
                rustc_middle::mir::Terminator {
                    source_info: rustc_middle::mir::SourceInfo { span, scope: source_scope },
                    kind: rustc_middle::mir::TerminatorKind::Call {
                        func,
                        args,
                        destination,
                        target: Some(target),
                        unwind: rustc_middle::mir::UnwindAction::Continue,
                        call_source: rustc_middle::mir::CallSource::Normal,
                        fn_span: span,
                    },
                }
            }
        };

        blocks.push(rustc_middle::mir::BasicBlockData::new_stmts(
            statements,
            Some(terminator),
            false,
        ));
    }

    let basic_blocks = IndexVec::from_iter(blocks);
    let local_decls = IndexVec::from_iter(locals);
    let body = rustc_middle::mir::Body::new(
        rustc_middle::mir::MirSource::item(owner.to_def_id()),
        basic_blocks,
        source_scopes,
        local_decls,
        IndexVec::new(),
        func.body.arg_count,
        Vec::new(),
        span,
        None,
        None,
    );

    unsafe { std::mem::transmute::<rustc_middle::mir::Body<'tcx>, rustc_middle::mir::Body<'static>>(body) }
}

fn mir_generic_args<'tcx>(
    tcx: TyCtxt<'tcx>,
    resolver: &MirResolver<'_>,
    args: &[MirTy],
) -> ty::GenericArgsRef<'tcx> {
    let generic_args: Vec<ty::GenericArg<'tcx>> = args
        .iter()
        .map(|ty| ty::GenericArg::from(mir_ty_to_rustc(tcx, resolver, ty)))
        .collect();
    tcx.mk_args(&generic_args)
}

fn mir_ty_to_rustc<'tcx>(tcx: TyCtxt<'tcx>, resolver: &MirResolver<'_>, ty: &MirTy) -> ty::Ty<'tcx> {
    match ty {
        MirTy::Unit => tcx.types.unit,
        MirTy::I32 => tcx.types.i32,
        MirTy::Isize => tcx.types.isize,
        MirTy::Usize => tcx.types.usize,
        MirTy::U8 => tcx.types.u8,
        MirTy::I8 => tcx.types.i8,
        MirTy::Ptr { mutability, to } => match mutability {
            MirMutability::Not => ty::Ty::new_ptr(tcx, mir_ty_to_rustc(tcx, resolver, to), hir::Mutability::Not),
            MirMutability::Mut => ty::Ty::new_ptr(tcx, mir_ty_to_rustc(tcx, resolver, to), hir::Mutability::Mut),
        },
        MirTy::Ref { mutability, to } => {
            let mutbl = match mutability {
                MirMutability::Not => Mutability::Not,
                MirMutability::Mut => Mutability::Mut,
            };
            ty::Ty::new_ref(
                tcx,
                tcx.lifetimes.re_erased,
                mir_ty_to_rustc(tcx, resolver, to),
                mutbl,
            )
        }
        MirTy::Adt { id, args } => {
            let def_id = resolver.resolve_ty(*id);
            let adt = tcx.adt_def(def_id);
            let args = mir_generic_args(tcx, resolver, args);
            ty::Ty::new_adt(tcx, adt, args)
        }
        MirTy::FnDef { id } => {
            let def_id = resolver.resolve_fn(*id);
            let args = ty::GenericArgs::identity_for_item(tcx, def_id);
            ty::Ty::new_fn_def(tcx, def_id, args)
        }
    }
}

fn mir_place_to_rustc<'tcx>(tcx: TyCtxt<'tcx>, place: &MirPlace) -> rustc_middle::mir::Place<'tcx> {
    let mut proj = Vec::new();
    for elem in &place.projection {
        match elem {
            MirProjection::Deref => proj.push(rustc_middle::mir::PlaceElem::Deref),
        }
    }
    rustc_middle::mir::Place {
        local: rustc_middle::mir::Local::from_usize(place.local),
        projection: tcx.mk_place_elems(&proj),
    }
}

fn mir_rvalue_to_rustc<'tcx>(
    tcx: TyCtxt<'tcx>,
    resolver: &MirResolver<'_>,
    rvalue: &MirRvalue,
) -> rustc_middle::mir::Rvalue<'tcx> {
    match rvalue {
        MirRvalue::Use(op) => rustc_middle::mir::Rvalue::Use(mir_operand_to_rustc(tcx, resolver, op)),
        MirRvalue::AddressOf { mutability, place } => rustc_middle::mir::Rvalue::RawPtr(
            match mutability {
                MirMutability::Not => rustc_middle::mir::RawPtrKind::Const,
                MirMutability::Mut => rustc_middle::mir::RawPtrKind::Mut,
            },
            mir_place_to_rustc(tcx, place),
        ),
        MirRvalue::Ref { mutability, place } => rustc_middle::mir::Rvalue::Ref(
            tcx.lifetimes.re_erased,
            match mutability {
                MirMutability::Not => BorrowKind::Shared,
                MirMutability::Mut => BorrowKind::Mut { kind: MutBorrowKind::Default },
            },
            mir_place_to_rustc(tcx, place),
        ),
    }
}

fn mir_operand_to_rustc<'tcx>(
    tcx: TyCtxt<'tcx>,
    resolver: &MirResolver<'_>,
    operand: &MirOperand,
) -> rustc_middle::mir::Operand<'tcx> {
    match operand {
        MirOperand::Copy(place) => rustc_middle::mir::Operand::Copy(mir_place_to_rustc(tcx, place)),
        MirOperand::Move(place) => rustc_middle::mir::Operand::Move(mir_place_to_rustc(tcx, place)),
        MirOperand::Const(c) => {
            rustc_middle::mir::Operand::Constant(Box::new(mir_const_to_rustc(tcx, resolver, c)))
        }
    }
}

fn mir_const_to_rustc<'tcx>(
    tcx: TyCtxt<'tcx>,
    resolver: &MirResolver<'_>,
    konst: &MirConst,
) -> rustc_middle::mir::ConstOperand<'tcx> {
    let span = DUMMY_SP;
    let (_ty, constant) = match konst {
        MirConst::Unit => (
            tcx.types.unit,
            rustc_middle::mir::Const::Val(ConstValue::ZeroSized, tcx.types.unit),
        ),
        MirConst::I32(v) => (
            tcx.types.i32,
            rustc_middle::mir::Const::Val(ConstValue::Scalar(Scalar::from_int(*v, rustc_abi::Size::from_bits(32))), tcx.types.i32),
        ),
        MirConst::Isize(v) => (
            tcx.types.isize,
            rustc_middle::mir::Const::Val(
                ConstValue::Scalar(Scalar::from_int(*v, tcx.data_layout().pointer_size())),
                tcx.types.isize,
            ),
        ),
        MirConst::Usize(v) => (
            tcx.types.usize,
            rustc_middle::mir::Const::Val(
                ConstValue::Scalar(Scalar::from_uint(*v, tcx.data_layout().pointer_size())),
                tcx.types.usize,
            ),
        ),
        MirConst::ByteStr(bytes) => {
            let alloc_id = tcx.allocate_bytes_dedup(bytes.clone(), 0);
            let ptr = Pointer::from(alloc_id);
            let scalar = Scalar::Ptr(ptr, tcx.data_layout().pointer_size().bytes() as u8);
            let ty = ty::Ty::new_ptr(tcx, tcx.types.i8, hir::Mutability::Not);
            (ty, rustc_middle::mir::Const::Val(ConstValue::Scalar(scalar), ty))
        }
        MirConst::Fn { id, args } => {
            let def_id = resolver.resolve_fn(*id);
            let generic_args = mir_generic_args(tcx, resolver, args);
            let ty = ty::Ty::new_fn_def(tcx, def_id, generic_args);
            (ty, rustc_middle::mir::Const::Val(ConstValue::ZeroSized, ty))
        }
    };

    rustc_middle::mir::ConstOperand { span, user_ty: None, const_: constant }
}

fn to_rustc_span(context: &Context, span: Span) -> RustcSpan {
    let files = context.take_custom_files();
    let _ = files; // TODO: Use SourceMap to register files and map spans.
    let lo = BytePos(span.lo);
    let hi = BytePos(span.hi);
    RustcSpan::new(lo, hi, SyntaxContext::root(), None)
}

fn make_owner_info(nodes: hir::OwnerNodes<'static>) -> hir::OwnerInfo<'static> {
    hir::OwnerInfo {
        nodes,
        parenting: LocalDefIdMap::default(),
        attrs: hir::AttributeMap {
            map: rustc_data_structures::sorted_map::SortedMap::new(),
            define_opaque: None,
            opt_hash: Some(Fingerprint::ZERO),
        },
        trait_map: ItemLocalMap::default(),
        delayed_lints: hir::lints::DelayedLints {
            lints: Vec::new().into_boxed_slice(),
            opt_hash: Some(Fingerprint::ZERO),
        },
    }
}

#[repr(C)]
struct ModuleItemsRepr {
    add_root: bool,
    submodules: Box<[OwnerId]>,
    free_items: Box<[hir::ItemId]>,
    trait_items: Box<[hir::TraitItemId]>,
    impl_items: Box<[hir::ImplItemId]>,
    foreign_items: Box<[hir::ForeignItemId]>,
    opaques: Box<[LocalDefId]>,
    body_owners: Box<[LocalDefId]>,
    nested_bodies: Box<[LocalDefId]>,
    delayed_lint_items: Box<[OwnerId]>,
    eiis: Box<[LocalDefId]>,
}

#[derive(Clone)]
struct ModuleItemsSpec {
    add_root: bool,
    free_items: Vec<hir::ItemId>,
    foreign_items: Vec<hir::ForeignItemId>,
    body_owners: Vec<LocalDefId>,
    delayed_lint_items: Vec<OwnerId>,
}

fn make_module_items(spec: ModuleItemsSpec) -> ModuleItems {
    let repr = ModuleItemsRepr {
        add_root: spec.add_root,
        submodules: Vec::new().into_boxed_slice(),
        free_items: spec.free_items.into_boxed_slice(),
        trait_items: Vec::new().into_boxed_slice(),
        impl_items: Vec::new().into_boxed_slice(),
        foreign_items: spec.foreign_items.into_boxed_slice(),
        opaques: Vec::new().into_boxed_slice(),
        body_owners: spec.body_owners.into_boxed_slice(),
        nested_bodies: Vec::new().into_boxed_slice(),
        delayed_lint_items: spec.delayed_lint_items.into_boxed_slice(),
        eiis: Vec::new().into_boxed_slice(),
    };

    unsafe { std::mem::transmute::<ModuleItemsRepr, ModuleItems>(repr) }
}

fn insert_owner(
    owners: &mut IndexVec<LocalDefId, Option<&'static hir::OwnerInfo<'static>>>,
    def_id: LocalDefId,
    info: &'static hir::OwnerInfo<'static>,
) {
    if def_id.index() >= owners.len() {
        owners.resize(def_id.index() + 1, None);
    }
    owners[def_id] = Some(info);
}

fn make_prim_ty(owner: LocalDefId, prim: hir::PrimTy) -> hir::Ty<'static> {
    let ident = Ident::from_str(prim.name_str());
    let segment = hir::PathSegment::new(ident, HirId::make_owner(owner), Res::PrimTy(prim));
    let segments = leak(vec![segment].into_boxed_slice());
    let path = leak(hir::Path {
        span: DUMMY_SP,
        res: Res::PrimTy(prim),
        segments,
    });
    hir::Ty {
        hir_id: HirId::make_owner(owner),
        span: DUMMY_SP,
        kind: hir::TyKind::Path(hir::QPath::Resolved(None, path)),
    }
}

fn make_ptr_ty(
    owner: LocalDefId,
    pointee: &'static hir::Ty<'static>,
    mutability: hir::Mutability,
) -> hir::Ty<'static> {
    hir::Ty {
        hir_id: HirId::make_owner(owner),
        span: DUMMY_SP,
        kind: hir::TyKind::Ptr(hir::MutTy { ty: pointee, mutbl: mutability }),
    }
}

fn make_unit_ty(owner: LocalDefId) -> hir::Ty<'static> {
    let empty: &'static [hir::Ty<'static>] = leak(Vec::new().into_boxed_slice());
    hir::Ty {
        hir_id: HirId::make_owner(owner),
        span: DUMMY_SP,
        kind: hir::TyKind::Tup(empty),
    }
}

fn mir_ty_to_hir(owner: LocalDefId, ty: &MirTy) -> hir::Ty<'static> {
    match ty {
        MirTy::Unit => make_unit_ty(owner),
        MirTy::I32 => make_prim_ty(owner, hir::PrimTy::Int(IntTy::I32)),
        MirTy::Isize => make_prim_ty(owner, hir::PrimTy::Int(IntTy::Isize)),
        MirTy::Usize => make_prim_ty(owner, hir::PrimTy::Uint(UintTy::Usize)),
        MirTy::U8 => make_prim_ty(owner, hir::PrimTy::Uint(UintTy::U8)),
        MirTy::I8 => make_prim_ty(owner, hir::PrimTy::Int(IntTy::I8)),
        MirTy::Ptr { mutability, to } => {
            let pointee = leak(mir_ty_to_hir(owner, to));
            make_ptr_ty(
                owner,
                pointee,
                match mutability {
                    MirMutability::Not => hir::Mutability::Not,
                    MirMutability::Mut => hir::Mutability::Mut,
                },
            )
        }
        _ => todo!("hir type support for {:?}", ty),
    }
}

fn leak<T>(value: T) -> &'static T {
    Box::leak(Box::new(value))
}
