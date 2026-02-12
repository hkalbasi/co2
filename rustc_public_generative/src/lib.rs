#![feature(rustc_private)]

//! rustc_public_generative
//!
//! This crate provides a `generate` entrypoint that runs `rustc_driver`
//! but injects a synthetic crate produced by user code.

extern crate rustc_abi;
extern crate rustc_ast;
extern crate rustc_data_structures;
extern crate rustc_driver;
extern crate rustc_hashes;
extern crate rustc_hir;
extern crate rustc_index;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_public;
extern crate rustc_public_bridge;
extern crate rustc_session;
extern crate rustc_span;
extern crate rustc_target;

use std::hash::Hash;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::sync::{Condvar, OnceLock};

use rustc_abi::ExternAbi;
use rustc_ast::{IntTy, UintTy};
use rustc_data_structures::fingerprint::Fingerprint;
use rustc_data_structures::fx::FxHashMap;
use rustc_data_structures::steal::Steal;
use rustc_hir as hir;
use rustc_hir::def::{DefKind, Res};
use rustc_hir::def_id::{DefId, LocalDefId, LocalDefIdMap, CRATE_DEF_ID};
use rustc_hir::definitions::{DefPathData, Definitions, DisambiguatorState};
use rustc_hir::{HirId, ItemLocalId, ItemLocalMap, OwnerId};
use rustc_index::{Idx, IndexVec};
use rustc_middle::middle::codegen_fn_attrs::CodegenFnAttrFlags;
use rustc_middle::mir::BorrowKind;
use rustc_middle::query::Providers as QueryProviders;
use rustc_middle::ty::{self, TyCtxt};
use rustc_middle::util::Providers as UtilProviders;
use rustc_session::config::EntryFnType;
use rustc_span::symbol::{Ident, Symbol};
use rustc_span::{BytePos, Span as RustcSpan, SyntaxContext, DUMMY_SP};

pub use rustc_public::mir::{
    AggregateKind as MirAggregateKind, BasicBlock as MirBasicBlock, Body as MirBody,
    BorrowKind as MirBorrowKind, CastKind as MirCastKind, ConstOperand as MirConst,
    LocalDecl as MirLocalDecl, MutBorrowKind as MirMutBorrowKind, Mutability as MirMutability,
    Operand as MirOperand, Place as MirPlace, ProjectionElem as MirProjection,
    RawPtrKind as MirRawPtrKind, Rvalue as MirRvalue, Statement as MirStatement,
    StatementKind as MirStatementKind, Terminator as MirTerminator,
    TerminatorKind as MirTerminatorKind, UnwindAction as MirUnwindAction,
};
pub use rustc_public::ty::{
    AdtDef, FnDef, GenericArgKind, GenericArgs, IntTy as PublicIntTy, MirConst as PublicMirConst,
    Region, RegionKind, RigidTy, Span as PublicSpan, Ty as MirTy, UintTy as PublicUintTy,
};
pub use rustc_public::CrateDef;

/// Context passed to the user callback. Used for allocating new IDs and
/// registering custom source files.
#[derive(Debug, Clone, Default)]
pub struct Context(Arc<ContextInner>);

#[derive(Debug, Default)]
struct ContextInner {
    next_file_id: std::sync::atomic::AtomicU32,
    custom_files: Mutex<Vec<CustomFile>>,
    registered_files: Mutex<FxHashMap<FileId, RegisteredFile>>,
}

#[derive(Debug, Clone)]
pub struct CustomFile {
    pub id: FileId,
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

impl From<PublicSpan> for Span {
    fn from(value: PublicSpan) -> Self {
        let _ = value;
        Self {
            file: FileId(0),
            lo: 0,
            hi: 0,
        }
    }
}

impl Context {
    pub fn new() -> Self {
        Self(Arc::new(ContextInner {
            next_file_id: std::sync::atomic::AtomicU32::new(1),
            custom_files: Mutex::new(Vec::new()),
            registered_files: Mutex::new(FxHashMap::default()),
        }))
    }

    pub fn add_custom_file(&self, path: impl Into<PathBuf>, contents: impl Into<String>) -> FileId {
        let id = self
            .0
            .next_file_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut guard = self.0.custom_files.lock().unwrap();
        guard.push(CustomFile {
            id: FileId(id),
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

    pub(crate) fn register_with_source_map(&self, tcx: TyCtxt<'_>) {
        let files = self.take_custom_files();
        if files.is_empty() {
            return;
        }
        let source_map = tcx.sess.source_map();
        let mut reg_guard = self.0.registered_files.lock().unwrap();
        for file in files {
            if reg_guard.contains_key(&file.id) {
                continue;
            }
            let source_file = if file.path.exists() {
                source_map
                    .load_file(&file.path)
                    .unwrap_or_else(|_| {
                        let real = source_map.path_mapping().to_real_filename(
                            source_map.working_dir(),
                            file.path.as_path(),
                        );
                        source_map.new_source_file(
                            rustc_span::FileName::Real(real),
                            file.contents.clone(),
                        )
                    })
            } else {
                source_map.new_source_file(
                    rustc_span::FileName::Custom(file.path.display().to_string()),
                    file.contents.clone(),
                )
            };
            reg_guard.insert(
                file.id,
                RegisteredFile {
                    start: source_file.start_pos,
                    end: source_file.end_position(),
                },
            );
        }
    }

    pub fn span_in_file(&self, file: FileId, lo: u32, hi: u32) -> rustc_public::ty::Span {
        let guard = self.0.registered_files.lock().unwrap();
        let file = guard
            .get(&file)
            .unwrap_or_else(|| panic!("file id {file:?} not registered"));
        let lo = file.start + BytePos(lo);
        let mut hi = file.start + BytePos(hi);
        if hi > file.end {
            hi = file.end;
        }
        let span = RustcSpan::new(lo, hi, SyntaxContext::root(), None);
        rustc_public::rustc_internal::stable(span)
    }
}

#[derive(Debug, Clone, Copy)]
struct RegisteredFile {
    start: BytePos,
    end: BytePos,
}

/// Summary of crates loaded as dependencies by rustc.
#[derive(Debug, Clone, Default)]
pub struct DependencyInfo {
    pub crates: Vec<DependencyCrate>,
    pub functions: Vec<DependencyFunction>,
    pub types: Vec<DependencyType>,
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

#[derive(Debug, Clone)]
pub struct DependencyFunction {
    pub id: FunctionId,
    pub path: String,
    pub def_path_hash_hi: u64,
    pub def_path_hash_lo: u64,
    pub fn_def: Option<rustc_public::ty::FnDef>,
}

#[derive(Debug, Clone)]
pub struct DependencyType {
    pub adt: AdtDef,
    pub path: String,
    pub def_path_hash_hi: u64,
    pub def_path_hash_lo: u64,
}

/// User-provided description of the current crate to be generated.
#[derive(Debug, Clone, Default)]
pub struct CurrentCrateInfo {
    pub crate_name: String,
    pub items: Vec<ItemInfo>,
    pub entry: Option<ItemId>,
    pub no_main: bool,
}

#[derive(Debug, Clone)]
pub struct ItemInfo {
    pub id: ItemId,
    pub name: String,
    pub parent: Option<ItemId>,
    pub kind: ItemKind,
    pub no_mangle: bool,
}

#[derive(Debug, Clone)]
pub enum ItemKind {
    Module,
    Function(FunctionSignature),
    ForeignFunction(FunctionSignature),
    Struct,
    Enum,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ItemId(u64);

impl ItemId {
    pub fn new(value: u64) -> Self {
        Self(value)
    }
}

#[derive(Debug, Clone)]
pub struct FunctionSignature {
    pub inputs: Vec<MirTy>,
    pub output: MirTy,
    pub abi: FunctionAbi,
    pub is_unsafe: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FunctionAbi {
    Rust,
    C,
}

#[derive(Debug, Clone)]
pub struct DefinedCrateInfo {
    pub crate_name: String,
    pub items: Vec<DefinedItemInfo>,
    pub entry: Option<ItemId>,
}

#[derive(Debug, Clone)]
pub struct DefinedItemInfo {
    pub id: ItemId,
    pub name: String,
    pub kind: DefinedItemKind,
}

impl DefinedItemInfo {
    pub fn fn_def(&self) -> Option<FnDef> {
        match self.kind {
            DefinedItemKind::Function(fn_def) => Some(fn_def),
            DefinedItemKind::ForeignFunction(fn_def) => Some(fn_def),
        }
    }
}

#[derive(Debug, Clone)]
pub enum DefinedItemKind {
    Function(FnDef),
    ForeignFunction(FnDef),
}

#[derive(Debug, Clone)]
pub struct ItemMirInfo {
    pub id: ItemId,
    pub body: MirBody,
}

#[derive(Debug, Clone)]
struct ForeignFunctionInfo {
    id: ItemId,
    name: String,
    inputs: Vec<MirTy>,
    output: MirTy,
}

#[derive(Debug, Clone)]
struct FunctionInfo {
    id: ItemId,
    name: String,
    signature: FunctionSignature,
    no_mangle: bool,
}

/// Run rustc_driver but emit a synthetic crate described by two callbacks.
///
/// Phase 1 (`define_items`) declares all items and their signatures.
/// Phase 2 (`emit_mir`) receives the allocated rustc_public IDs and returns MIR bodies.
pub fn generate<D, M>(define_items: D, emit_mir: M)
where
    D: FnOnce(Context, DependencyInfo) -> CurrentCrateInfo + Send + 'static,
    M: FnOnce(Context, DependencyInfo, DefinedCrateInfo) -> Vec<ItemMirInfo> + Send + 'static,
{
    generate_with_args(std::env::args().collect(), define_items, emit_mir)
}

pub fn generate_with_args<D, M>(mut args: Vec<String>, define_items: D, emit_mir: M)
where
    D: FnOnce(Context, DependencyInfo) -> CurrentCrateInfo + Send + 'static,
    M: FnOnce(Context, DependencyInfo, DefinedCrateInfo) -> Vec<ItemMirInfo> + Send + 'static,
{
    if args.len() == 1 {
        // Provide a dummy crate name if invoked programmatically without args.
        args.push(String::from("rustc"));
        args.push(String::from("--crate-name"));
        args.push(String::from("synthetic"));
        args.push(String::from("--crate-type=bin"));
        args.push(String::from("/dev/null"));
    }
    let mut callbacks = GenerateCallbacks::new(define_items, emit_mir);
    rustc_driver::run_compiler(&args, &mut callbacks);
}

struct GenerateCallbacks<D, M> {
    define_items: Option<D>,
    emit_mir: Option<M>,
    context: Context,
    gate: Arc<GenerateGate>,
}

#[derive(Default)]
struct GenerateState {
    generated: Option<GeneratedCrate>,
    original: Option<OriginalProviders>,
    define_items: Option<Box<dyn FnOnce(Context, DependencyInfo) -> CurrentCrateInfo + Send>>,
    emit_mir: Option<
        Box<dyn FnOnce(Context, DependencyInfo, DefinedCrateInfo) -> Vec<ItemMirInfo> + Send>,
    >,
    context: Option<Context>,
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
    codegen_fn_attrs: for<'tcx> fn(
        TyCtxt<'tcx>,
        LocalDefId,
    ) -> rustc_middle::middle::codegen_fn_attrs::CodegenFnAttrs,
    mir_built: for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> &'tcx Steal<rustc_middle::mir::Body<'tcx>>,
    mir_for_ctfe: for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> &'tcx rustc_middle::mir::Body<'tcx>,
    mir_drops_elaborated_and_const_checked:
        for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> &'tcx Steal<rustc_middle::mir::Body<'tcx>>,
    optimized_mir: for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> &'tcx rustc_middle::mir::Body<'tcx>,
}

static GENERATE_STATE: OnceLock<Arc<GenerateGate>> = OnceLock::new();

// TODO: these are very wrong
unsafe impl Sync for GenerateGate {}
unsafe impl Send for GenerateGate {}

fn run_with_public_context<'tcx, T>(tcx: TyCtxt<'tcx>, f: impl FnOnce() -> T) -> T {
    let mut f = Some(f);
    match rustc_public::rustc_internal::run(tcx, || (f.take().expect("closure missing"))()) {
        Ok(value) => value,
        Err(_) => (f.take().expect("closure missing"))(),
    }
}

fn with_generated_and_original<'tcx, R>(
    tcx: TyCtxt<'tcx>,
    f: impl FnOnce(Option<&GeneratedCrate>, OriginalProviders) -> R,
) -> R {
    let state = GENERATE_STATE
        .get()
        .cloned()
        .expect("generate state missing");
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
            if generated_ptr.is_some() {
                "some"
            } else {
                "none"
            }
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
            if guard.define_items.is_some() {
                "some"
            } else {
                "none"
            }
        );
    }
    let define_items = guard
        .define_items
        .take()
        .expect("define_items callback missing");
    let emit_mir = guard.emit_mir.take().expect("emit_mir callback missing");
    let context = guard.context.clone().expect("context missing");
    drop(guard);

    rustc_public::rustc_internal::run(tcx, || {
        let dependency_info = collect_dependency_info(tcx);
        let crate_info = define_items(context.clone(), dependency_info.clone());
        let generated = GeneratedCrate::build(tcx, &context, dependency_info.clone(), crate_info);

        {
            let mut guard = gate.state.lock().unwrap();
            guard.generated = Some(generated);
        }

        let generated_ptr = {
            let guard = gate.state.lock().unwrap();
            guard
                .generated
                .as_ref()
                .map(|g| g as *const GeneratedCrate)
                .expect("generated crate missing")
        };

        let defined = unsafe { (&*generated_ptr).defined_info(tcx) };
        let item_mir = emit_mir(context.clone(), dependency_info, defined);

        let mut guard = gate.state.lock().unwrap();
        guard
            .generated
            .as_mut()
            .expect("generated crate missing")
            .install_mir(tcx, item_mir);
    })
    .expect("failed to run rustc_public context");

    let mut guard = gate.state.lock().unwrap();
    guard.building = false;
    guard.building_thread = None;
    gate.cvar.notify_all();
}

impl<D, M> GenerateCallbacks<D, M>
where
    D: FnOnce(Context, DependencyInfo) -> CurrentCrateInfo + Send + 'static,
    M: FnOnce(Context, DependencyInfo, DefinedCrateInfo) -> Vec<ItemMirInfo> + Send + 'static,
{
    fn new(define_items: D, emit_mir: M) -> Self {
        Self {
            define_items: Some(define_items),
            emit_mir: Some(emit_mir),
            context: Context::new(),
            gate: Arc::new(GenerateGate {
                state: Mutex::new(GenerateState::default()),
                cvar: Condvar::new(),
            }),
        }
    }
}

impl<D, M> rustc_driver::Callbacks for GenerateCallbacks<D, M>
where
    D: FnOnce(Context, DependencyInfo) -> CurrentCrateInfo + Send + 'static,
    M: FnOnce(Context, DependencyInfo, DefinedCrateInfo) -> Vec<ItemMirInfo> + Send + 'static,
{
    fn config(&mut self, config: &mut rustc_interface::Config) {
        let define_items = self
            .define_items
            .take()
            .expect("define_items callback already used");
        let emit_mir = self
            .emit_mir
            .take()
            .expect("emit_mir callback already used");

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
            guard.define_items = Some(Box::new(define_items));
            guard.emit_mir = Some(Box::new(emit_mir));
            guard.context = Some(self.context.clone());
        }
    }
}

fn collect_dependency_info<'tcx>(tcx: rustc_middle::ty::TyCtxt<'tcx>) -> DependencyInfo {
    let mut info = DependencyInfo::default();

    for &krate in tcx.crates(()).iter() {
        let name = tcx.crate_name(krate).to_string();
        let disambiguator = tcx.crate_hash(krate).to_hex();
        info.crates.push(DependencyCrate {
            name,
            disambiguator,
        });
    }

    let mut next_fn_id = 1u64;
    let mut alloc_fn_id = || {
        let id = FunctionId(next_fn_id);
        next_fn_id += 1;
        id
    };

    for &cnum in tcx.crates(()).iter() {
        let num_defs = tcx.num_extern_def_ids(cnum);
        for idx in 0..num_defs {
            let def_id = DefId {
                krate: cnum,
                index: rustc_span::def_id::DefIndex::from_usize(idx),
            };
            collect_dependency_def(tcx, def_id, &mut info, &mut alloc_fn_id);
        }
    }

    info
}

fn collect_dependency_def<'tcx>(
    tcx: TyCtxt<'tcx>,
    def_id: DefId,
    info: &mut DependencyInfo,
    alloc_fn_id: &mut impl FnMut() -> FunctionId,
) {
    let kind = tcx.def_kind(def_id);

    if matches!(kind, DefKind::Fn | DefKind::AssocFn | DefKind::Ctor(..)) {
        let id = alloc_fn_id();
        let hash = tcx.def_path_hash(def_id);
        let (hi, lo): (u64, u64) = unsafe { std::mem::transmute::<Fingerprint, (u64, u64)>(hash.0) };
        info.functions.push(DependencyFunction {
            id,
            path: tcx.def_path_str(def_id),
            def_path_hash_hi: hi,
            def_path_hash_lo: lo,
            fn_def: stable_fn_from_def_id(tcx, def_id),
        });
    }

    if matches!(kind, DefKind::Struct | DefKind::Enum | DefKind::Union) {
        if let Some(adt) = stable_adt_from_def_id(tcx, def_id) {
            let hash = tcx.def_path_hash(def_id);
            let (hi, lo): (u64, u64) =
                unsafe { std::mem::transmute::<Fingerprint, (u64, u64)>(hash.0) };
            info.types.push(DependencyType {
                adt,
                path: tcx.def_path_str(def_id),
                def_path_hash_hi: hi,
                def_path_hash_lo: lo,
            });
        }
    }
}

fn stable_adt_from_def_id<'tcx>(tcx: TyCtxt<'tcx>, def_id: DefId) -> Option<AdtDef> {
    use rustc_public::rustc_internal::stable;
    use rustc_public::ty::{RigidTy, TyKind};

    let ty = tcx.type_of(def_id).instantiate_identity();
    let stable_ty = stable(ty);
    match stable_ty.kind() {
        TyKind::RigidTy(RigidTy::Adt(adt, _)) => Some(adt),
        _ => None,
    }
}

fn stable_fn_from_def_id<'tcx>(
    tcx: TyCtxt<'tcx>,
    def_id: DefId,
) -> Option<rustc_public::ty::FnDef> {
    use rustc_public::rustc_internal::stable;
    use rustc_public::ty::TyKind;

    let args = ty::GenericArgs::identity_for_item(tcx, def_id);
    let fn_ty = ty::Ty::new_fn_def(tcx, def_id, args);
    let stable_ty = stable(fn_ty);
    match stable_ty.kind() {
        TyKind::RigidTy(RigidTy::FnDef(def, _)) => Some(def),
        _ => None,
    }
}

fn function_def_stable<'tcx>(tcx: TyCtxt<'tcx>, def_id: LocalDefId) -> rustc_public::DefId {
    use rustc_public::rustc_internal::stable;
    use rustc_public::ty::TyKind;

    let args = ty::GenericArgs::identity_for_item(tcx, def_id);
    let fn_ty = ty::Ty::new_fn_def(tcx, def_id.to_def_id(), args);
    match stable(fn_ty).kind() {
        TyKind::RigidTy(RigidTy::FnDef(def, _)) => def.0,
        _ => panic!("Failed to generate defId"),
    }
}

fn def_path_hash_from_parts(hi: u64, lo: u64) -> rustc_span::def_id::DefPathHash {
    rustc_span::def_id::DefPathHash(Fingerprint::new(hi, lo))
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
            mir_drops_elaborated_and_const_checked: providers
                .mir_drops_elaborated_and_const_checked,
            optimized_mir: providers.optimized_mir,
        });
    }
    drop(guard);

    providers.hir_crate = generated_hir_crate;
    // Leave hir_crate_items/hir_module_items to the original providers.
    providers.local_def_id_to_hir_id = generated_local_def_id_to_hir_id;
    providers.opt_hir_owner_nodes = generated_opt_hir_owner_nodes;
    providers.hir_owner_parent_q = generated_hir_owner_parent_q;
    providers.hir_attr_map = generated_hir_attr_map;
    providers.opt_ast_lowering_delayed_lints = generated_opt_ast_lowering_delayed_lints;
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
    providers.mir_drops_elaborated_and_const_checked =
        generated_mir_drops_elaborated_and_const_checked;
    providers.optimized_mir = generated_optimized_mir;
}

fn generated_hir_crate<'tcx>(tcx: TyCtxt<'tcx>, key: ()) -> hir::Crate<'tcx> {
    if std::env::var("GEN_DEBUG").is_ok() {
        eprintln!("generated_hir_crate");
    }
    with_generated_and_original(tcx, |generated, original| {
        if let Some(generated_crate) = generated {
            return generated_crate.hir_crate(tcx, key);
        }
        (original.hir_crate)(tcx, key)
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
        if let Some(generated_crate) = generated {
            if let Some(nodes) = generated_crate.opt_hir_owner_nodes(tcx, key) {
                return Some(nodes);
            }
            if std::env::var("GEN_DEBUG").is_ok() {
                eprintln!(
                    "generated_opt_hir_owner_nodes: fallback to original {:?}",
                    key
                );
            }
        }
        let original_crate = (original.hir_crate)(tcx, ());
        original_crate
            .owners
            .get(key)
            .and_then(|owner| owner.as_owner().map(|o| &o.nodes))
    })
}

fn generated_local_def_id_to_hir_id<'tcx>(tcx: TyCtxt<'tcx>, key: LocalDefId) -> HirId {
    with_generated_and_original(tcx, |generated, original| {
        if let Some(generated_crate) = generated {
            if generated_crate.def_kinds.contains_key(&key) {
                return HirId::make_owner(key);
            }
        }
        match (original.hir_crate)(tcx, ()).owners[key] {
            hir::MaybeOwner::Owner(_) => HirId::make_owner(key),
            hir::MaybeOwner::NonOwner(hir_id) => hir_id,
            hir::MaybeOwner::Phantom => panic!("No HirId for {:?}", key),
        }
    })
}

fn generated_hir_owner_parent_q<'tcx>(tcx: TyCtxt<'tcx>, key: OwnerId) -> HirId {
    with_generated_and_original(tcx, |generated, original| {
        if let Some(generated_crate) = generated {
            return generated_crate.hir_owner_parent_q(tcx, key);
        }
        (original.hir_owner_parent_q)(tcx, key)
    })
}

fn generated_hir_attr_map<'tcx>(
    tcx: TyCtxt<'tcx>,
    key: OwnerId,
) -> &'tcx hir::AttributeMap<'tcx> {
    with_generated_and_original(tcx, |generated, original| {
        if let Some(generated_crate) = generated {
            if generated_crate.def_kinds.contains_key(&key.def_id) {
                return hir::AttributeMap::EMPTY;
            }
        }
        (original.hir_crate)(tcx, ()).owners[key.def_id]
            .as_owner()
            .map_or(hir::AttributeMap::EMPTY, |o| &o.attrs)
    })
}

fn generated_opt_ast_lowering_delayed_lints<'tcx>(
    tcx: TyCtxt<'tcx>,
    key: OwnerId,
) -> Option<&'tcx hir::lints::DelayedLints> {
    with_generated_and_original(tcx, |generated, original| {
        if let Some(generated_crate) = generated {
            if generated_crate.def_kinds.contains_key(&key.def_id) {
                return None;
            }
        }
        (original.hir_crate)(tcx, ()).owners[key.def_id]
            .as_owner()
            .map(|o| &o.delayed_lints)
    })
}

fn generated_entry_fn<'tcx>(tcx: TyCtxt<'tcx>, key: ()) -> Option<(DefId, EntryFnType)> {
    with_generated_and_original(tcx, |generated, original| {
        if let Some(generated_crate) = generated {
            return generated_crate.entry_fn(tcx, key);
        }
        (original.entry_fn)(tcx, key)
    })
}

fn generated_def_kind<'tcx>(tcx: TyCtxt<'tcx>, key: LocalDefId) -> DefKind {
    if std::env::var("GEN_DEBUG").is_ok() {
        eprintln!("generated_def_kind {:?}", key);
    }
    with_generated_and_original(tcx, |generated, original| {
        if let Some(generated_crate) = generated {
            if let Some(kind) = generated_crate.def_kind(key) {
                return kind;
            }
        }
        (original.def_kind)(tcx, key)
    })
}

fn generated_def_span<'tcx>(tcx: TyCtxt<'tcx>, key: LocalDefId) -> RustcSpan {
    with_generated_and_original(tcx, |generated, original| {
        if let Some(generated_crate) = generated {
            if let Some(span) = generated_crate.def_span(key) {
                return span;
            }
        }
        (original.def_span)(tcx, key)
    })
}

fn generated_def_ident_span<'tcx>(tcx: TyCtxt<'tcx>, key: LocalDefId) -> Option<RustcSpan> {
    with_generated_and_original(tcx, |generated, original| {
        if let Some(generated_crate) = generated {
            if let Some(span) = generated_crate.def_span(key) {
                return Some(span);
            }
        }
        (original.def_ident_span)(tcx, key)
    })
}

fn generated_visibility<'tcx>(tcx: TyCtxt<'tcx>, key: LocalDefId) -> ty::Visibility<DefId> {
    with_generated_and_original(tcx, |generated, original| {
        if let Some(generated_crate) = generated {
            if generated_crate.def_kinds.contains_key(&key) {
                return ty::Visibility::Public;
            }
        }
        (original.visibility)(tcx, key)
    })
}

fn generated_generics_of<'tcx>(tcx: TyCtxt<'tcx>, key: LocalDefId) -> ty::Generics {
    with_generated_and_original(tcx, |generated, original| {
        if let Some(generated_crate) = generated {
            if let Some(generics) = generated_crate.generics_of(tcx, key) {
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
        if let Some(generated_crate) = generated {
            if let Some(ty) = generated_crate.type_of(tcx, key) {
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
        if let Some(generated_crate) = generated {
            if let Some(sig) = generated_crate.fn_sig(tcx, key) {
                return sig;
            }
        }
        (original.fn_sig)(tcx, key)
    })
}

fn generated_predicates_of<'tcx>(tcx: TyCtxt<'tcx>, key: DefId) -> ty::GenericPredicates<'tcx> {
    with_generated_and_original(tcx, |generated, original| {
        if let Some(generated_crate) = generated {
            if let Some(preds) = generated_crate.predicates_of(tcx, key) {
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
        if let Some(generated_crate) = generated {
            if let Some(preds) = generated_crate.explicit_predicates_of(tcx, key) {
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
        if let Some(generated_crate) = generated {
            if let Some(attrs) = generated_crate.codegen_fn_attrs(tcx, key) {
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
        if let Some(generated_crate) = generated {
            if let Some(body) = generated_crate.mir_built(tcx, key) {
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
        if let Some(generated_crate) = generated {
            if let Some(body) = generated_crate.mir_for_ctfe(tcx, key) {
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
        if let Some(generated_crate) = generated {
            if let Some(body) = generated_crate.mir_drops_elaborated_and_const_checked(tcx, key) {
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
        if let Some(generated_crate) = generated {
            if let Some(body) = generated_crate.optimized_mir(tcx, key) {
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
) -> (LocalDefId, LocalDefId, FxHashMap<ItemId, LocalDefId>) {
    let defs_guard = tcx.definitions_untracked();
    let defs_mut = unsafe { &mut *(&*defs_guard as *const Definitions as *mut Definitions) };
    let mut disamb = DisambiguatorState::with(
        CRATE_DEF_ID,
        DefPathData::ValueNs(Symbol::intern("main")),
        1,
    );
    let foreign_mod = defs_mut.create_def(CRATE_DEF_ID, DefPathData::ForeignMod, &mut disamb);
    let main_def = defs_mut.create_def(
        CRATE_DEF_ID,
        DefPathData::ValueNs(Symbol::intern("main")),
        &mut disamb,
    );
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
    foreign_function_ids: FxHashMap<ItemId, LocalDefId>,
    function_ids: FxHashMap<ItemId, LocalDefId>,
    foreign_function_infos: LocalDefIdMap<ForeignFunctionInfo>,
    foreign_function_sigs: LocalDefIdMap<(Vec<ty::Ty<'static>>, ty::Ty<'static>)>,
    foreign_function_symbols: LocalDefIdMap<Symbol>,
    function_sigs: LocalDefIdMap<(Vec<ty::Ty<'static>>, ty::Ty<'static>, hir::Safety, ExternAbi)>,
    function_symbols: LocalDefIdMap<Symbol>,
    owners: IndexVec<LocalDefId, Option<&'static hir::OwnerInfo<'static>>>,
    owner_parents: LocalDefIdMap<HirId>,
    def_kinds: LocalDefIdMap<DefKind>,
    def_spans: LocalDefIdMap<RustcSpan>,
    function_infos: LocalDefIdMap<FunctionInfo>,
    function_bodies: LocalDefIdMap<rustc_middle::mir::Body<'static>>,
    function_mir: Mutex<LocalDefIdMap<&'static Steal<rustc_middle::mir::Body<'static>>>>,
    entry_fn: Option<LocalDefId>,
    no_main: bool,
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
        context.register_with_source_map(tcx);
        let crate_name = if info.crate_name.is_empty() {
            Symbol::intern("synthetic")
        } else {
            Symbol::intern(&info.crate_name)
        };

        let mut foreign_functions = Vec::new();
        let mut functions = Vec::new();
        for item in &info.items {
            match &item.kind {
                ItemKind::ForeignFunction(sig) => {
                    foreign_functions.push(ForeignFunctionInfo {
                        id: item.id,
                        name: item.name.clone(),
                        inputs: sig.inputs.clone(),
                        output: sig.output,
                    });
                }
                ItemKind::Function(sig) => {
                    functions.push(FunctionInfo {
                        id: item.id,
                        name: item.name.clone(),
                        signature: sig.clone(),
                        no_mangle: item.no_mangle,
                    });
                }
                ItemKind::Module => {}
                ItemKind::Struct | ItemKind::Enum => {
                    todo!("item kind {:?} is not implemented yet", item.kind)
                }
            }
        }

        let mut dep_functions = FxHashMap::default();
        for func in &dependency_info.functions {
            let hash = def_path_hash_from_parts(func.def_path_hash_hi, func.def_path_hash_lo);
            if let Some(def_id) = tcx.def_path_hash_to_def_id(hash) {
                dep_functions.insert(func.id, def_id);
            }
        }

        let mut owners: IndexVec<LocalDefId, Option<&'static hir::OwnerInfo<'static>>> =
            IndexVec::new();
        let mut owner_parents: LocalDefIdMap<HirId> = LocalDefIdMap::default();
        let mut def_kinds: LocalDefIdMap<DefKind> = LocalDefIdMap::default();
        let mut def_spans: LocalDefIdMap<RustcSpan> = LocalDefIdMap::default();
        let mut function_infos: LocalDefIdMap<FunctionInfo> = LocalDefIdMap::default();
        let mut foreign_function_infos: LocalDefIdMap<ForeignFunctionInfo> =
            LocalDefIdMap::default();
        let mut foreign_function_sigs: LocalDefIdMap<(Vec<ty::Ty<'static>>, ty::Ty<'static>)> =
            LocalDefIdMap::default();
        let mut foreign_function_symbols: LocalDefIdMap<Symbol> = LocalDefIdMap::default();
        let mut function_sigs: LocalDefIdMap<(
            Vec<ty::Ty<'static>>,
            ty::Ty<'static>,
            hir::Safety,
            ExternAbi,
        )> = LocalDefIdMap::default();
        let mut function_symbols: LocalDefIdMap<Symbol> = LocalDefIdMap::default();

        let crate_def = CRATE_DEF_ID;
        let (foreign_mod_def, main_def, foreign_function_ids) =
            allocate_def_ids(tcx, &foreign_functions);

        def_kinds.insert(crate_def, DefKind::Mod);
        def_kinds.insert(foreign_mod_def, DefKind::ForeignMod);
        def_kinds.insert(main_def, DefKind::Fn);

        def_spans.insert(crate_def, DUMMY_SP);
        def_spans.insert(foreign_mod_def, DUMMY_SP);
        def_spans.insert(main_def, DUMMY_SP);

        let mut foreign_item_ids = Vec::new();
        let mut foreign_items_hir = Vec::new();

        for foreign in &foreign_functions {
            let def_id = *foreign_function_ids
                .get(&foreign.id)
                .expect("foreign function id missing");
            def_kinds.insert(def_id, DefKind::Fn);
            def_spans.insert(def_id, DUMMY_SP);
            foreign_function_infos.insert(def_id, foreign.clone());
            foreign_function_symbols.insert(def_id, Symbol::intern(&foreign.name));

            let inputs_tcx = foreign
                .inputs
                .iter()
                .map(|ty| mir_ty_to_rustc(tcx, ty))
                .collect::<Vec<_>>();
            let output_tcx = mir_ty_to_rustc(tcx, &foreign.output);
            let inputs_static = unsafe {
                std::mem::transmute::<Vec<ty::Ty<'tcx>>, Vec<ty::Ty<'static>>>(inputs_tcx)
            };
            let output_static =
                unsafe { std::mem::transmute::<ty::Ty<'tcx>, ty::Ty<'static>>(output_tcx) };
            foreign_function_sigs.insert(def_id, (inputs_static, output_static));

            let foreign_item_id = hir::ForeignItemId {
                owner_id: OwnerId { def_id },
            };
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
            owner_id: OwnerId {
                def_id: foreign_mod_def,
            },
            kind: hir::ItemKind::ForeignMod {
                abi: ExternAbi::C { unwind: false },
                items: foreign_items,
            },
            span: DUMMY_SP,
            vis_span: DUMMY_SP,
            has_delayed_lints: false,
            eii: false,
        };
        let foreign_mod_item = leak(foreign_mod_item);

        let first_function = functions.first().cloned();
        let entry_name = first_function
            .as_ref()
            .map(|f| f.name.as_str())
            .unwrap_or("main");
        let main_ident = Ident::from_str(entry_name);
        let main_inputs: Vec<hir::Ty<'static>> = Vec::new();
        let main_fn_decl = leak(hir::FnDecl {
            inputs: leak(main_inputs.into_boxed_slice()),
            output: hir::FnRetTy::DefaultReturn(DUMMY_SP),
            c_variadic: false,
            implicit_self: hir::ImplicitSelfKind::None,
            lifetime_elision_allowed: true,
        });
        let main_fn_sig = hir::FnSig {
            header: hir::FnHeader {
                safety: hir::HeaderSafety::Normal(
                    if info.no_main
                        && first_function
                        .as_ref()
                        .map(|f| f.signature.is_unsafe)
                        .unwrap_or(false)
                    {
                        hir::Safety::Unsafe
                    } else {
                        hir::Safety::Safe
                    },
                ),
                constness: hir::Constness::NotConst,
                asyncness: hir::IsAsync::NotAsync,
                abi: if info.no_main {
                    match first_function
                        .as_ref()
                        .map(|f| f.signature.abi)
                        .unwrap_or(FunctionAbi::Rust)
                    {
                        FunctionAbi::Rust => ExternAbi::Rust,
                        FunctionAbi::C => ExternAbi::C { unwind: false },
                    }
                } else {
                    ExternAbi::Rust
                },
            },
            decl: main_fn_decl,
            span: DUMMY_SP,
        };

        let main_body_expr = leak(hir::Expr {
            hir_id: HirId {
                owner: OwnerId { def_id: main_def },
                local_id: ItemLocalId::new(1),
            },
            kind: hir::ExprKind::Tup(&[]),
            span: DUMMY_SP,
        });
        let main_body = leak(hir::Body {
            params: &[],
            value: main_body_expr,
        });
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
            spans: hir::ModSpans {
                inner_span: DUMMY_SP,
                inject_use_span: DUMMY_SP,
            },
            item_ids: leak(
                vec![
                    hir::ItemId {
                        owner_id: OwnerId {
                            def_id: foreign_mod_def,
                        },
                    },
                    hir::ItemId {
                        owner_id: OwnerId { def_id: main_def },
                    },
                ]
                .into_boxed_slice(),
            ),
        });

        let crate_owner_nodes = build_owner_nodes_for_crate(root_mod);
        insert_owner(
            &mut owners,
            crate_def,
            leak(make_owner_info(crate_owner_nodes)),
        );
        owner_parents.insert(crate_def, HirId::make_owner(crate_def));

        let foreign_mod_nodes = build_owner_nodes_for_item(foreign_mod_item);
        insert_owner(
            &mut owners,
            foreign_mod_def,
            leak(make_owner_info(foreign_mod_nodes)),
        );
        owner_parents.insert(foreign_mod_def, HirId::make_owner(crate_def));

        for (def_id, foreign_item) in foreign_items_hir {
            let foreign_nodes = build_owner_nodes_for_foreign_item(foreign_item);
            insert_owner(&mut owners, def_id, leak(make_owner_info(foreign_nodes)));
            owner_parents.insert(def_id, HirId::make_owner(foreign_mod_def));
        }

        let main_nodes = build_owner_nodes_for_fn(main_item, main_body, main_body_expr);
        insert_owner(&mut owners, main_def, leak(make_owner_info(main_nodes)));
        owner_parents.insert(main_def, HirId::make_owner(crate_def));

        if let Some(first) = functions.first() {
            function_infos.insert(main_def, first.clone());
            if first.no_mangle {
                function_symbols.insert(main_def, Symbol::intern(&first.name));
            }
            if info.no_main {
                let inputs_tcx = first
                    .signature
                    .inputs
                    .iter()
                    .map(|ty| mir_ty_to_rustc(tcx, ty))
                    .collect::<Vec<_>>();
                let output_tcx = mir_ty_to_rustc(tcx, &first.signature.output);
                let inputs_static = unsafe {
                    std::mem::transmute::<Vec<ty::Ty<'tcx>>, Vec<ty::Ty<'static>>>(inputs_tcx)
                };
                let output_static =
                    unsafe { std::mem::transmute::<ty::Ty<'tcx>, ty::Ty<'static>>(output_tcx) };
                let safety = if first.signature.is_unsafe {
                    hir::Safety::Unsafe
                } else {
                    hir::Safety::Safe
                };
                let abi = match first.signature.abi {
                    FunctionAbi::Rust => ExternAbi::Rust,
                    FunctionAbi::C => ExternAbi::C { unwind: false },
                };
                function_sigs.insert(main_def, (inputs_static, output_static, safety, abi));
            }
        }

        if std::env::var("GEN_DEBUG").is_ok() {
            eprintln!("GeneratedCrate::build: owners len {}", owners.len());
        }

        let mut function_ids = FxHashMap::default();
        if let Some(first) = functions.first() {
            function_ids.insert(first.id, main_def);
        }
        let entry_fn = if info.no_main {
            None
        } else {
            info.entry
                .and_then(|id| function_ids.get(&id).copied())
                .or(Some(main_def))
        };

        Self {
            crate_name,
            context: context.clone(),
            foreign_function_ids,
            function_ids,
            foreign_function_infos,
            foreign_function_sigs,
            foreign_function_symbols,
            function_sigs,
            function_symbols,
            owners,
            owner_parents,
            def_kinds,
            def_spans,
            function_infos,
            function_bodies: LocalDefIdMap::default(),
            function_mir: Mutex::new(LocalDefIdMap::default()),
            entry_fn,
            no_main: info.no_main,
        }
    }

    fn defined_info<'tcx>(&self, tcx: TyCtxt<'tcx>) -> DefinedCrateInfo {
        let mut items = Vec::new();

        for (id, def_id) in &self.function_ids {
            let fn_def = function_def_stable(tcx, *def_id);
            items.push(DefinedItemInfo {
                id: *id,
                name: self
                    .function_infos
                    .get(def_id)
                    .map(|f| f.name.clone())
                    .unwrap_or_else(|| "function".to_string()),
                kind: DefinedItemKind::Function(rustc_public::ty::FnDef(fn_def)),
            });
        }

        for (id, def_id) in &self.foreign_function_ids {
            let fn_def = function_def_stable(tcx, *def_id);
            items.push(DefinedItemInfo {
                id: *id,
                name: self
                    .foreign_function_infos
                    .get(def_id)
                    .map(|f| f.name.clone())
                    .unwrap_or_else(|| "foreign".to_string()),
                kind: DefinedItemKind::ForeignFunction(rustc_public::ty::FnDef(fn_def)),
            });
        }

        DefinedCrateInfo {
            crate_name: self.crate_name.as_str().to_string(),
            items,
            entry: self.entry_fn.and_then(|entry| {
                self.function_ids
                    .iter()
                    .find(|(_, d)| **d == entry)
                    .map(|(id, _)| *id)
            }),
        }
    }

    fn install_mir<'tcx>(&mut self, tcx: TyCtxt<'tcx>, item_mir: Vec<ItemMirInfo>) {
        for mir in item_mir {
            let Some(def_id) = self.function_ids.get(&mir.id).copied() else {
                continue;
            };
            let body = build_mir_body(tcx, &self.context, &mir.body, def_id);
            self.function_bodies.insert(def_id, body);
        }
    }

    fn hir_crate<'tcx>(&self, _tcx: TyCtxt<'tcx>, _key: ()) -> hir::Crate<'tcx> {
        let owners: IndexVec<LocalDefId, hir::MaybeOwner<'tcx>> =
            IndexVec::from_iter(self.owners.iter().map(|opt| match opt {
                Some(info) => {
                    let info = unsafe {
                        std::mem::transmute::<
                            &'static hir::OwnerInfo<'static>,
                            &'tcx hir::OwnerInfo<'tcx>,
                        >(*info)
                    };
                    hir::MaybeOwner::Owner(info)
                }
                None => hir::MaybeOwner::Phantom,
            }));
        hir::Crate {
            owners,
            opt_hir_hash: None,
        }
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
                    eprintln!(
                        "generated_crate.opt_hir_owner_nodes {:?} => {:?}",
                        key,
                        info.nodes.node()
                    );
                }
                std::mem::transmute::<&hir::OwnerNodes<'static>, &'tcx hir::OwnerNodes<'tcx>>(
                    &info.nodes,
                )
            })
            .or_else(|| {
                if std::env::var("GEN_DEBUG").is_ok() {
                    eprintln!(
                        "generated_crate.opt_hir_owner_nodes {:?} => None (len {})",
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
        if self.function_infos.contains_key(&def_id)
            || self.foreign_function_infos.contains_key(&def_id)
        {
            let args = ty::GenericArgs::identity_for_item(tcx, def_id);
            let ty = ty::Ty::new_fn_def(tcx, def_id.to_def_id(), args);
            return Some(ty::EarlyBinder::bind(ty));
        }
        None
    }

    fn fn_sig<'tcx>(
        &self,
        tcx: TyCtxt<'tcx>,
        def_id: LocalDefId,
    ) -> Option<ty::EarlyBinder<'tcx, ty::PolyFnSig<'tcx>>> {
        if let Some(_info) = self.function_infos.get(&def_id) {
            if !self.no_main {
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
            let Some((inputs_static, output_static, safety, abi)) = self.function_sigs.get(&def_id)
            else {
                return None;
            };
            let inputs = unsafe {
                std::mem::transmute::<Vec<ty::Ty<'static>>, Vec<ty::Ty<'tcx>>>(
                    inputs_static.clone(),
                )
            };
            let output =
                unsafe { std::mem::transmute::<ty::Ty<'static>, ty::Ty<'tcx>>(*output_static) };
            let sig = tcx.mk_fn_sig(
                inputs,
                output,
                false,
                *safety,
                *abi,
            );
            let poly = ty::Binder::dummy(sig);
            return Some(ty::EarlyBinder::bind(poly));
        }
        if let Some((inputs_static, output_static)) = self.foreign_function_sigs.get(&def_id) {
            let inputs = unsafe {
                std::mem::transmute::<Vec<ty::Ty<'static>>, Vec<ty::Ty<'tcx>>>(
                    inputs_static.clone(),
                )
            };
            let output =
                unsafe { std::mem::transmute::<ty::Ty<'static>, ty::Ty<'tcx>>(*output_static) };
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

    fn predicates_of<'tcx>(
        &self,
        tcx: TyCtxt<'tcx>,
        def_id: DefId,
    ) -> Option<ty::GenericPredicates<'tcx>> {
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
            if let Some(symbol) = self.function_symbols.get(&def_id) {
                attrs.flags.insert(CodegenFnAttrFlags::NO_MANGLE);
                attrs.symbol_name = Some(*symbol);
            }
            return Some(attrs);
        }
        None
    }

    fn mir_built<'tcx>(
        &self,
        _tcx: TyCtxt<'tcx>,
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

        let body = self.function_bodies.get(&def_id)?;
        let steal = leak(Steal::new(body.clone()));

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
        let mut body = self.function_bodies.get(&def_id)?.clone();
        body.set_required_consts(Vec::new());
        body.set_mentioned_items(Vec::new());
        let owned = unsafe {
            std::mem::transmute::<rustc_middle::mir::Body<'static>, rustc_middle::mir::Body<'tcx>>(
                body,
            )
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

    hir::OwnerNodes {
        opt_hash_including_bodies: None,
        nodes,
        bodies,
    }
}

fn build_mir_body<'tcx>(
    tcx: TyCtxt<'tcx>,
    _context: &Context,
    body: &MirBody,
    owner: LocalDefId,
) -> rustc_middle::mir::Body<'static> {
    let span = run_with_public_context(tcx, || {
        rustc_public::rustc_internal::internal(tcx, body.span)
    });
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

    let locals: Vec<rustc_middle::mir::LocalDecl<'tcx>> = body
        .locals()
        .iter()
        .map(|local| rustc_middle::mir::LocalDecl {
            mutability: match local.mutability {
                MirMutability::Not => rustc_middle::mir::Mutability::Not,
                MirMutability::Mut => rustc_middle::mir::Mutability::Mut,
            },
            local_info: rustc_middle::mir::ClearCrossCrate::Set(Box::new(
                rustc_middle::mir::LocalInfo::Boring,
            )),
            ty: mir_ty_to_rustc(tcx, &local.ty),
            user_ty: None,
            source_info: rustc_middle::mir::SourceInfo {
                span,
                scope: source_scope,
            },
        })
        .collect();

    let mut blocks = Vec::new();
    for block in &body.blocks {
        let mut statements = Vec::new();
        for stmt in &block.statements {
            match &stmt.kind {
                MirStatementKind::Assign(place, rvalue) => {
                    let place = mir_place_to_rustc(tcx, place);
                    let rvalue = mir_rvalue_to_rustc(tcx, rvalue);
                    statements.push(rustc_middle::mir::Statement::new(
                        rustc_middle::mir::SourceInfo {
                            span,
                            scope: source_scope,
                        },
                        rustc_middle::mir::StatementKind::Assign(Box::new((place, rvalue))),
                    ));
                }
                _ => todo!(),
            }
        }

        let terminator = match &block.terminator.kind {
            MirTerminatorKind::Return => rustc_middle::mir::Terminator {
                source_info: rustc_middle::mir::SourceInfo {
                    span,
                    scope: source_scope,
                },
                kind: rustc_middle::mir::TerminatorKind::Return,
            },
            MirTerminatorKind::Call {
                func,
                args,
                destination,
                target,
                unwind: _,
            } => {
                let func = mir_operand_to_rustc(tcx, func);
                let args: Box<[rustc_span::source_map::Spanned<rustc_middle::mir::Operand<'tcx>>]> =
                    args.iter()
                        .map(|arg| rustc_span::source_map::Spanned {
                            node: mir_operand_to_rustc(tcx, arg),
                            span,
                        })
                        .collect::<Vec<_>>()
                        .into_boxed_slice();
                let destination = mir_place_to_rustc(tcx, destination);
                let target = target.map(rustc_middle::mir::BasicBlock::from_usize);
                rustc_middle::mir::Terminator {
                    source_info: rustc_middle::mir::SourceInfo {
                        span,
                        scope: source_scope,
                    },
                    kind: rustc_middle::mir::TerminatorKind::Call {
                        func,
                        args,
                        destination,
                        target,
                        unwind: rustc_middle::mir::UnwindAction::Continue,
                        call_source: rustc_middle::mir::CallSource::Normal,
                        fn_span: span,
                    },
                }
            }
            _ => todo!(),
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
        body.arg_locals().len(),
        Vec::new(),
        span,
        None,
        None,
    );

    unsafe {
        std::mem::transmute::<rustc_middle::mir::Body<'tcx>, rustc_middle::mir::Body<'static>>(body)
    }
}

fn mir_ty_to_rustc<'tcx>(tcx: TyCtxt<'tcx>, ty: &MirTy) -> ty::Ty<'tcx> {
    use rustc_public::rustc_internal::internal;
    run_with_public_context(tcx, || internal(tcx, ty))
}

fn mir_region_to_rustc<'tcx>(tcx: TyCtxt<'tcx>, ty: &rustc_public::ty::Region) -> ty::Region<'tcx> {
    use rustc_public::rustc_internal::internal;
    run_with_public_context(tcx, || internal(tcx, ty))
}

fn mir_place_to_rustc<'tcx>(tcx: TyCtxt<'tcx>, place: &MirPlace) -> rustc_middle::mir::Place<'tcx> {
    let mut proj = Vec::new();
    for elem in &place.projection {
        match elem {
            MirProjection::Deref => proj.push(rustc_middle::mir::PlaceElem::Deref),
            _ => todo!(),
        }
    }
    rustc_middle::mir::Place {
        local: rustc_middle::mir::Local::from_usize(place.local),
        projection: tcx.mk_place_elems(&proj),
    }
}

fn mir_rvalue_to_rustc<'tcx>(
    tcx: TyCtxt<'tcx>,
    rvalue: &MirRvalue,
) -> rustc_middle::mir::Rvalue<'tcx> {
    match rvalue {
        MirRvalue::Use(op) => rustc_middle::mir::Rvalue::Use(mir_operand_to_rustc(tcx, op)),
        MirRvalue::AddressOf(raw_ptr_kind, place) => rustc_middle::mir::Rvalue::RawPtr(
            match raw_ptr_kind {
                rustc_public::mir::RawPtrKind::Mut => rustc_middle::mir::RawPtrKind::Mut,
                rustc_public::mir::RawPtrKind::Const => rustc_middle::mir::RawPtrKind::Const,
                rustc_public::mir::RawPtrKind::FakeForPtrMetadata => {
                    rustc_middle::mir::RawPtrKind::FakeForPtrMetadata
                }
            },
            mir_place_to_rustc(tcx, place),
        ),
        MirRvalue::Aggregate(kind, ops) => {
            let kind = match kind {
                rustc_public::mir::AggregateKind::Array(ty) => {
                    rustc_middle::mir::AggregateKind::Array(mir_ty_to_rustc(tcx, ty))
                }
                rustc_public::mir::AggregateKind::Tuple => {
                    rustc_middle::mir::AggregateKind::Tuple
                }
                rustc_public::mir::AggregateKind::RawPtr(ty, mutability) => {
                    rustc_middle::mir::AggregateKind::RawPtr(
                        mir_ty_to_rustc(tcx, ty),
                        match mutability {
                            rustc_public::mir::Mutability::Mut => rustc_middle::mir::Mutability::Mut,
                            rustc_public::mir::Mutability::Not => rustc_middle::mir::Mutability::Not,
                        },
                    )
                }
                _ => todo!("aggregate kind not supported yet"),
            };
            rustc_middle::mir::Rvalue::Aggregate(
                Box::new(kind),
                ops.iter().map(|op| mir_operand_to_rustc(tcx, op)).collect(),
            )
        }
        MirRvalue::Cast(kind, operand, ty) => {
            let kind = match kind {
                rustc_public::mir::CastKind::PointerExposeAddress => {
                    rustc_middle::mir::CastKind::PointerExposeProvenance
                }
                rustc_public::mir::CastKind::PointerWithExposedProvenance => {
                    rustc_middle::mir::CastKind::PointerWithExposedProvenance
                }
                rustc_public::mir::CastKind::PointerCoercion(coercion) => {
                    rustc_middle::mir::CastKind::PointerCoercion(
                        match coercion {
                        rustc_public::mir::PointerCoercion::ReifyFnPointer(safety) => {
                            rustc_middle::ty::adjustment::PointerCoercion::ReifyFnPointer(
                                match safety {
                                    rustc_public::mir::Safety::Safe => rustc_hir::Safety::Safe,
                                    rustc_public::mir::Safety::Unsafe => rustc_hir::Safety::Unsafe,
                                },
                            )
                        }
                        rustc_public::mir::PointerCoercion::UnsafeFnPointer => {
                            rustc_middle::ty::adjustment::PointerCoercion::UnsafeFnPointer
                        }
                        rustc_public::mir::PointerCoercion::ClosureFnPointer(safety) => {
                            rustc_middle::ty::adjustment::PointerCoercion::ClosureFnPointer(
                                match safety {
                                    rustc_public::mir::Safety::Safe => rustc_hir::Safety::Safe,
                                    rustc_public::mir::Safety::Unsafe => rustc_hir::Safety::Unsafe,
                                },
                            )
                        }
                        rustc_public::mir::PointerCoercion::MutToConstPointer => {
                            rustc_middle::ty::adjustment::PointerCoercion::MutToConstPointer
                        }
                        rustc_public::mir::PointerCoercion::ArrayToPointer => {
                            rustc_middle::ty::adjustment::PointerCoercion::ArrayToPointer
                        }
                        rustc_public::mir::PointerCoercion::Unsize => {
                            rustc_middle::ty::adjustment::PointerCoercion::Unsize
                        }
                    },
                        rustc_middle::mir::CoercionSource::AsCast,
                    )
                }
                rustc_public::mir::CastKind::IntToInt => rustc_middle::mir::CastKind::IntToInt,
                rustc_public::mir::CastKind::FloatToInt => rustc_middle::mir::CastKind::FloatToInt,
                rustc_public::mir::CastKind::FloatToFloat => {
                    rustc_middle::mir::CastKind::FloatToFloat
                }
                rustc_public::mir::CastKind::IntToFloat => rustc_middle::mir::CastKind::IntToFloat,
                rustc_public::mir::CastKind::PtrToPtr => rustc_middle::mir::CastKind::PtrToPtr,
                rustc_public::mir::CastKind::FnPtrToPtr => rustc_middle::mir::CastKind::FnPtrToPtr,
                rustc_public::mir::CastKind::Transmute => rustc_middle::mir::CastKind::Transmute,
                rustc_public::mir::CastKind::Subtype => rustc_middle::mir::CastKind::Subtype,
            };
            rustc_middle::mir::Rvalue::Cast(
                kind,
                mir_operand_to_rustc(tcx, operand),
                mir_ty_to_rustc(tcx, ty),
            )
        }
        MirRvalue::Ref(region, borrow_kind, place) => rustc_middle::mir::Rvalue::Ref(
            mir_region_to_rustc(tcx, region),
            match borrow_kind {
                rustc_public::mir::BorrowKind::Shared => BorrowKind::Shared,
                rustc_public::mir::BorrowKind::Fake(rustc_public::mir::FakeBorrowKind::Deep) => {
                    BorrowKind::Fake(rustc_middle::mir::FakeBorrowKind::Deep)
                }
                rustc_public::mir::BorrowKind::Fake(rustc_public::mir::FakeBorrowKind::Shallow) => {
                    BorrowKind::Fake(rustc_middle::mir::FakeBorrowKind::Shallow)
                }
                rustc_public::mir::BorrowKind::Mut {
                    kind: rustc_public::mir::MutBorrowKind::Default,
                } => BorrowKind::Mut {
                    kind: rustc_middle::mir::MutBorrowKind::Default,
                },
                rustc_public::mir::BorrowKind::Mut {
                    kind: rustc_public::mir::MutBorrowKind::ClosureCapture,
                } => BorrowKind::Mut {
                    kind: rustc_middle::mir::MutBorrowKind::ClosureCapture,
                },
                rustc_public::mir::BorrowKind::Mut {
                    kind: rustc_public::mir::MutBorrowKind::TwoPhaseBorrow,
                } => BorrowKind::Mut {
                    kind: rustc_middle::mir::MutBorrowKind::TwoPhaseBorrow,
                },
            },
            mir_place_to_rustc(tcx, place),
        ),
        _ => todo!(),
    }
}

fn mir_operand_to_rustc<'tcx>(
    tcx: TyCtxt<'tcx>,
    operand: &MirOperand,
) -> rustc_middle::mir::Operand<'tcx> {
    match operand {
        MirOperand::Copy(place) => rustc_middle::mir::Operand::Copy(mir_place_to_rustc(tcx, place)),
        MirOperand::Move(place) => rustc_middle::mir::Operand::Move(mir_place_to_rustc(tcx, place)),
        MirOperand::Constant(c) => {
            rustc_middle::mir::Operand::Constant(Box::new(mir_const_to_rustc(tcx, c)))
        }
        MirOperand::RuntimeChecks(_) => todo!(),
    }
}

fn mir_const_to_rustc<'tcx>(
    tcx: TyCtxt<'tcx>,
    konst: &MirConst,
) -> rustc_middle::mir::ConstOperand<'tcx> {
    use rustc_public::rustc_internal::internal;
    rustc_middle::mir::ConstOperand {
        span: run_with_public_context(tcx, || internal(tcx, konst.span)),
        user_ty: None,
        const_: run_with_public_context(tcx, || internal(tcx, konst.const_.clone())),
    }
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
        kind: hir::TyKind::Ptr(hir::MutTy {
            ty: pointee,
            mutbl: mutability,
        }),
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
    use rustc_public::ty::{RigidTy, TyKind};

    match ty.kind() {
        TyKind::RigidTy(RigidTy::Int(int_ty)) => {
            let int_ty = match int_ty {
                rustc_public::ty::IntTy::Isize => IntTy::Isize,
                rustc_public::ty::IntTy::I8 => IntTy::I8,
                rustc_public::ty::IntTy::I16 => IntTy::I16,
                rustc_public::ty::IntTy::I32 => IntTy::I32,
                rustc_public::ty::IntTy::I64 => IntTy::I64,
                rustc_public::ty::IntTy::I128 => IntTy::I128,
            };
            make_prim_ty(owner, hir::PrimTy::Int(int_ty))
        }
        TyKind::RigidTy(RigidTy::Uint(int_ty)) => {
            let int_ty = match int_ty {
                rustc_public::ty::UintTy::Usize => UintTy::Usize,
                rustc_public::ty::UintTy::U8 => UintTy::U8,
                rustc_public::ty::UintTy::U16 => UintTy::U16,
                rustc_public::ty::UintTy::U32 => UintTy::U32,
                rustc_public::ty::UintTy::U64 => UintTy::U64,
                rustc_public::ty::UintTy::U128 => UintTy::U128,
            };
            make_prim_ty(owner, hir::PrimTy::Uint(int_ty))
        }
        TyKind::RigidTy(RigidTy::RawPtr(to, mutability)) => {
            let pointee = leak(mir_ty_to_hir(owner, &to));
            make_ptr_ty(
                owner,
                pointee,
                match mutability {
                    rustc_public::mir::Mutability::Not => hir::Mutability::Not,
                    rustc_public::mir::Mutability::Mut => hir::Mutability::Mut,
                },
            )
        }
        TyKind::RigidTy(RigidTy::Tuple(elems)) if elems.is_empty() => make_unit_ty(owner),
        _ => todo!("hir type support for {:?}", ty),
    }
}

fn leak<T>(value: T) -> &'static T {
    Box::leak(Box::new(value))
}
