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
use rustc_hir::def::{CtorKind, DefKind, Res};
use rustc_hir::def_id::{CRATE_DEF_ID, DefId as RustcDefId, LocalDefId, LocalDefIdMap};
use rustc_hir::definitions::{DefPathData, Definitions, DisambiguatorState};
use rustc_hir::{HirId, ItemLocalId, ItemLocalMap, OwnerId};
use rustc_index::{Idx, IndexVec};
use rustc_middle::mir::BorrowKind;
use rustc_middle::query::Providers as QueryProviders;
use rustc_middle::ty::{self, TyCtxt};
use rustc_middle::util::Providers as UtilProviders;
use rustc_session::config::EntryFnType;
use rustc_span::symbol::{Ident, Symbol};
use rustc_span::{BytePos, DUMMY_SP, Span as RustcSpan, SyntaxContext};

pub use hir_ty::{HirGenericArg, HirTy, HirTyKind};
pub use rustc_public::mir::{
    AggregateKind as MirAggregateKind, BasicBlock as MirBasicBlock, BinOp as MirBinOp,
    Body as MirBody, BorrowKind as MirBorrowKind, CastKind as MirCastKind,
    ConstOperand as MirConst, LocalDecl as MirLocalDecl, MutBorrowKind as MirMutBorrowKind,
    Mutability as MirMutability, Operand as MirOperand, Place as MirPlace,
    ProjectionElem as MirProjection, RawPtrKind as MirRawPtrKind, Rvalue as MirRvalue,
    Statement as MirStatement, StatementKind as MirStatementKind, Terminator as MirTerminator,
    TerminatorKind as MirTerminatorKind, UnwindAction as MirUnwindAction,
};
pub use rustc_public::ty::{
    AdtDef, FnDef, GenericArgKind, GenericArgs, IntTy as PublicIntTy, MirConst as PublicMirConst,
    Region, RegionKind, RigidTy, Span as PublicSpan, Ty as MirTy, UintTy as PublicUintTy,
    VariantIdx,
};
pub use rustc_public::{CrateDef, DefId};

mod hir_ty;

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
                source_map.load_file(&file.path).unwrap_or_else(|_| {
                    let real = source_map
                        .path_mapping()
                        .to_real_filename(source_map.working_dir(), file.path.as_path());
                    source_map
                        .new_source_file(rustc_span::FileName::Real(real), file.contents.clone())
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
    pub no_main: bool,
}

#[derive(Debug, Clone)]
pub struct ItemInfo {
    pub name: String,
    pub kind: ItemKind,
    pub no_mangle: bool,
}

#[derive(Debug, Clone)]
pub enum ItemKind {
    Module,
    Function,
    ForeignFunction,
    Struct(Vec<String>),
    Enum,
}

#[derive(Debug, Clone)]
pub struct StructField {
    pub id: DefId,
    pub ty: HirTy,
}

#[derive(Debug, Clone)]
pub struct FunctionSignature {
    pub inputs: Vec<HirTy>,
    pub output: HirTy,
    pub abi: FunctionAbi,
    pub is_unsafe: bool,
}

#[derive(Debug, Clone)]
pub struct ItemSignatureInfo {
    pub id: rustc_public::DefId,
    pub kind: ItemSignatureKind,
    pub span: PublicSpan,
}

#[derive(Debug, Clone)]
pub enum ItemSignatureKind {
    Function {
        sig: FunctionSignature,
        no_mangle: bool,
    },
    ForeignFunction(FunctionSignature),
    Struct(Vec<StructField>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FunctionAbi {
    Rust,
    C,
}

#[derive(Debug, Clone)]
pub struct DefinedCrateInfo {
    pub items: Vec<DefinedItemInfo>,
}

impl DefinedCrateInfo {
    fn owners<'tcx>(
        &self,
        tcx: TyCtxt<'tcx>,
        signatures: &[ItemSignatureInfo],
        foreign_mod_def: LocalDefId,
    ) -> IndexVec<LocalDefId, hir::MaybeOwner<'static>> {
        static RESULT: OnceLock<IndexVec<LocalDefId, hir::MaybeOwner<'static>>> = OnceLock::new();

        if let Some(r) = RESULT.get() {
            return r.clone();
        }

        let mut owners: IndexVec<LocalDefId, hir::MaybeOwner<'static>> = IndexVec::new();
        let mut owner_parents: LocalDefIdMap<HirId> = LocalDefIdMap::default();
        let mut foreign_function_symbols: LocalDefIdMap<Symbol> = LocalDefIdMap::default();
        let mut function_symbols: LocalDefIdMap<Symbol> = LocalDefIdMap::default();

        let crate_def = CRATE_DEF_ID;

        let mut foreign_item_ids = Vec::new();
        let mut foreign_items_hir = Vec::new();

        for (my_def_id, foreign) in signatures.iter().filter_map(|item| match &item.kind {
            ItemSignatureKind::ForeignFunction(function_signature) => {
                Some((item.id, function_signature))
            }
            _ => None,
        }) {
            let name = &self
                .items
                .iter()
                .find(|item| item.def_id() == my_def_id)
                .unwrap()
                .name;
            let def_id = my_def_id_to_rustc_def_id(tcx, my_def_id).expect_local();
            foreign_function_symbols.insert(def_id, Symbol::intern(name));

            let foreign_item_id = hir::ForeignItemId {
                owner_id: OwnerId { def_id },
            };
            foreign_item_ids.push(foreign_item_id);

            let fn_sig = generate_sig(tcx, def_id, &foreign.inputs, &foreign.output, true);

            let foreign_item = hir::ForeignItem {
                ident: Ident::from_str(name),
                kind: hir::ForeignItemKind::Fn(
                    fn_sig,
                    leak(vec![None; 0].into_boxed_slice()),
                    hir::Generics::empty(),
                ),
                owner_id: OwnerId { def_id },
                span: DUMMY_SP,
                vis_span: DUMMY_SP,
                has_delayed_lints: false,
            };
            foreign_items_hir.push((def_id, leak(unsafe { std::mem::transmute(foreign_item) })));
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

        let mut struct_items_hir = Vec::new();
        for (my_def_id, strukt) in signatures.iter().filter_map(|item| match &item.kind {
            ItemSignatureKind::Struct(fields) => Some((item.id, fields)),
            _ => None,
        }) {
            let name = &self
                .items
                .iter()
                .find(|item| item.def_id() == my_def_id)
                .unwrap()
                .name;
            let def_id = my_def_id_to_rustc_def_id(tcx, my_def_id).expect_local();

            let fields_hir: &'static [hir::FieldDef<'static>] = leak(
                strukt
                    .into_iter()
                    .enumerate()
                    .map(|(idx, field)| {
                        let field_def_id = my_def_id_to_rustc_def_id(tcx, field.id).expect_local();

                        let hir_id = HirId {
                            owner: OwnerId { def_id },
                            local_id: ItemLocalId::new(idx + 1),
                        };
                        let hir_field_def = hir::FieldDef {
                            span: DUMMY_SP,
                            vis_span: DUMMY_SP,
                            ident: Ident::from_str(
                                &self
                                    .items
                                    .iter()
                                    .find(|item| item.def_id() == field.id)
                                    .unwrap()
                                    .name,
                            ),
                            hir_id,
                            def_id: field_def_id,
                            ty: leak(hir_ty_to_rustc(tcx, def_id, &field.ty)),
                            safety: hir::Safety::Safe,
                            default: None,
                        };

                        insert_non_owner(
                            &mut owners,
                            field_def_id,
                            hir::MaybeOwner::NonOwner(hir_id),
                        );

                        hir_field_def
                    })
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            );
            let item = hir::Item {
                owner_id: OwnerId { def_id },
                kind: hir::ItemKind::Struct(
                    Ident::from_str(name),
                    hir::Generics::empty(),
                    hir::VariantData::Struct {
                        fields: fields_hir,
                        recovered: rustc_ast::Recovered::No,
                    },
                ),
                span: DUMMY_SP,
                vis_span: DUMMY_SP,
                has_delayed_lints: false,
                eii: false,
            };
            struct_items_hir.push((def_id, leak(item), fields_hir));
        }

        let mut function_items_hir = Vec::new();
        for (my_def_id, function, no_mangle) in
            signatures.iter().filter_map(|item| match &item.kind {
                ItemSignatureKind::Function { sig, no_mangle } => Some((item.id, sig, no_mangle)),
                _ => None,
            })
        {
            let name = &self
                .items
                .iter()
                .find(|item| item.def_id() == my_def_id)
                .unwrap()
                .name;
            let def_id = my_def_id_to_rustc_def_id(tcx, my_def_id).expect_local();

            if *no_mangle {
                function_symbols.insert(def_id, Symbol::intern(name));
            }

            let fn_sig = generate_sig(tcx, def_id, &function.inputs, &function.output, false);
            let loop_expr = leak(hir::Block {
                    stmts: &[],
                    expr: None,
                    hir_id: HirId {
                        owner: OwnerId { def_id },
                        local_id: ItemLocalId::new(2),
                    },
                    rules: rustc_hir::BlockCheckMode::DefaultBlock,
                    span: DUMMY_SP,
                    targeted_by_break: false,
                });
            let body_kind = hir::ExprKind::Loop(
                loop_expr,
                None,
                rustc_hir::LoopSource::Loop,
                DUMMY_SP,
            );
            let body_expr = leak(hir::Expr {
                hir_id: HirId {
                    owner: OwnerId { def_id },
                    local_id: ItemLocalId::new(1),
                },
                kind: body_kind,
                span: DUMMY_SP,
            });
            let body = leak(hir::Body {
                params: &[],
                value: body_expr,
            });
            let item = hir::Item {
                owner_id: OwnerId { def_id },
                kind: hir::ItemKind::Fn {
                    sig: fn_sig,
                    ident: Ident::from_str(name),
                    generics: hir::Generics::empty(),
                    body: body.id(),
                    has_body: true,
                },
                span: DUMMY_SP,
                vis_span: DUMMY_SP,
                has_delayed_lints: false,
                eii: false,
            };
            function_items_hir.push((def_id, leak(item), body, body_expr, loop_expr));
        }

        let mut root_item_ids =
            Vec::with_capacity(1 + struct_items_hir.len() + function_items_hir.len());
        root_item_ids.push(hir::ItemId {
            owner_id: OwnerId {
                def_id: foreign_mod_def,
            },
        });
        for (def_id, _, _) in &struct_items_hir {
            root_item_ids.push(hir::ItemId {
                owner_id: OwnerId { def_id: *def_id },
            });
        }
        for (def_id, _, _, _, _) in &function_items_hir {
            root_item_ids.push(hir::ItemId {
                owner_id: OwnerId { def_id: *def_id },
            });
        }

        let root_mod = leak(hir::Mod {
            spans: hir::ModSpans {
                inner_span: DUMMY_SP,
                inject_use_span: DUMMY_SP,
            },
            item_ids: leak(root_item_ids.into_boxed_slice()),
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

        for (def_id, item, fields) in struct_items_hir {
            let nodes = build_owner_nodes_for_struct(item, fields);
            insert_owner(&mut owners, def_id, leak(make_owner_info(nodes)));
            owner_parents.insert(def_id, HirId::make_owner(crate_def));
        }

        for (def_id, item, body, body_expr, loop_expr) in function_items_hir {
            let nodes = build_owner_nodes_for_fn(item, body, body_expr, loop_expr);
            insert_owner(&mut owners, def_id, leak(make_owner_info(nodes)));
            owner_parents.insert(def_id, HirId::make_owner(crate_def));
        }

        if std::env::var("GEN_DEBUG").is_ok() {
            eprintln!("GeneratedCrate::build: owners len {}", owners.len());
        }

        RESULT.set(owners.clone()).unwrap();

        owners
    }
}

fn generate_sig<'tcx>(
    tcx: TyCtxt<'tcx>,
    owner: LocalDefId,
    inputs: &[HirTy],
    output: &HirTy,
    is_c: bool,
) -> rustc_hir::FnSig<'static> {
    let fn_decl = leak(hir::FnDecl {
        inputs: leak(
            inputs
                .iter()
                .map(|ty| hir_ty_to_rustc(tcx, owner, ty))
                .collect::<Vec<_>>(),
        ),
        output: hir::FnRetTy::Return(leak(hir_ty_to_rustc(tcx, owner, output))),
        c_variadic: false,
        implicit_self: hir::ImplicitSelfKind::None,
        lifetime_elision_allowed: true,
    });

    let fn_sig = hir::FnSig {
        header: hir::FnHeader {
            safety: if is_c {
                hir::HeaderSafety::Normal(hir::Safety::Unsafe)
            } else {
                hir::HeaderSafety::Normal(hir::Safety::Safe)
            },
            constness: hir::Constness::NotConst,
            asyncness: hir::IsAsync::NotAsync,
            abi: if is_c {
                ExternAbi::C { unwind: false }
            } else {
                ExternAbi::Rust
            },
        },
        decl: fn_decl,
        span: DUMMY_SP,
    };
    fn_sig
}

fn my_def_id_to_rustc_def_id<'tcx>(tcx: TyCtxt<'tcx>, def_id: DefId) -> RustcDefId {
    rustc_public::rustc_internal::internal(tcx, def_id)
}

impl DefinedCrateState {
    fn stage_id(&self) -> i32 {
        match self {
            DefinedCrateState::Stage0 => 0,
            DefinedCrateState::Stage1(..) => 1,
            DefinedCrateState::Stage2(..) => 2,
            DefinedCrateState::Stage3(..) => 3,
        }
    }

    fn hir_crate<'tcx>(&self, tcx: TyCtxt<'tcx>, key: ()) -> rustc_hir::Crate<'tcx> {
        let (DefinedCrateState::Stage2(defined_crate, signatures, foreign_def_id)
        | DefinedCrateState::Stage3(defined_crate, signatures, foreign_def_id, _)) = self
        else {
            panic!("hir_crate query in stage {}", self.stage_id());
        };
        let owners = defined_crate.owners(tcx, signatures, *foreign_def_id);
        // let owners: IndexVec<LocalDefId, hir::MaybeOwner<'tcx>> =
        //     IndexVec::from_iter(owners.iter().map(|opt| match opt {
        //         Some(info) => {
        //             let info = unsafe {
        //                 std::mem::transmute::<
        //                     &'static hir::OwnerInfo<'static>,
        //                     &'tcx hir::OwnerInfo<'tcx>,
        //                 >(*info)
        //             };
        //             hir::MaybeOwner::Owner(info)
        //         }
        //         None => hir::MaybeOwner::Phantom,
        //     }));
        hir::Crate {
            owners,
            opt_hir_hash: Some(random_fingerprint()),
        }
    }

    #[cfg(false)]
    fn opt_hir_owner_nodes<'tcx>(
        &self,
        tcx: TyCtxt<'tcx>,
        key: LocalDefId,
    ) -> Option<&'tcx hir::OwnerNodes<'tcx>> {
        let (DefinedCrateState::Stage2(defined_crate, signatures, foreign_def_id)
        | DefinedCrateState::Stage3(defined_crate, signatures, foreign_def_id, _)) = self
        else {
            panic!("hir_crate query in stage {}", self.stage_id());
        };
        let owners = defined_crate.owners(tcx, signatures, *foreign_def_id);
        let r = owners
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
                        owners.len()
                    );
                }
                None
            });
        r
    }

    fn contains_key<'tcx>(&self, tcx: TyCtxt<'tcx>, key: &LocalDefId) -> bool {
        match self {
            DefinedCrateState::Stage0 => false,
            DefinedCrateState::Stage1(defined_crate_info)
            | DefinedCrateState::Stage2(defined_crate_info, _, _)
            | DefinedCrateState::Stage3(defined_crate_info, _, _, _) => defined_crate_info
                .items
                .iter()
                .any(|item| my_def_id_to_rustc_def_id(tcx, item.def_id()).as_local() == Some(*key)),
        }
    }

    fn hir_owner_parent_q<'tcx>(&self, tcx: TyCtxt<'tcx>, key: OwnerId) -> HirId {
        todo!()
    }

    fn entry_fn<'tcx>(&self, tcx: TyCtxt<'tcx>, key: ()) -> Option<(RustcDefId, EntryFnType)> {
        let entry_fn = match self {
            DefinedCrateState::Stage0 => panic!("Can't eval entry_fn at stage0"),
            DefinedCrateState::Stage1(defined_crate_info)
            | DefinedCrateState::Stage2(defined_crate_info, _, _)
            | DefinedCrateState::Stage3(defined_crate_info, _, _, _) => defined_crate_info
                .items
                .iter()
                .find(|item| item.name == "main")
                .map(|item| item.def_id()),
        };

        entry_fn.map(|def| {
            (
                my_def_id_to_rustc_def_id(tcx, def),
                EntryFnType::Main {
                    sigpipe: rustc_session::config::sigpipe::DEFAULT,
                },
            )
        })
    }

    fn def_kind<'tcx>(&self, tcx: TyCtxt<'tcx>, key: LocalDefId) -> DefKind {
        let (DefinedCrateState::Stage2(items, _, _) | DefinedCrateState::Stage3(items, _, _, _)) =
            self
        else {
            panic!("def_kind query in stage {}", self.stage_id());
        };
        let key = rustc_def_to_my_def(tcx, key.to_def_id());
        let kind = items
            .items
            .iter()
            .find(|item| item.def_id() == key)
            .unwrap()
            .kind;
        match kind {
            DefinedItemKind::CrateRoot(_) => DefKind::Mod,
            DefinedItemKind::ForeignMod(_) => DefKind::ForeignMod,
            DefinedItemKind::Function(_) => DefKind::Fn,
            DefinedItemKind::ForeignFunction(_) => DefKind::Fn,
            DefinedItemKind::Struct(_) => DefKind::Struct,
            DefinedItemKind::Field(_) => DefKind::Field,
        }
    }

    fn def_span<'tcx>(&self, _tcx: TyCtxt<'tcx>, _key: LocalDefId) -> Option<RustcSpan> {
        // let DefinedCrateState::Stage2(_, signatures, _) = self else {
        //     panic!("hir_crate query in stage {}", self.stage_id());
        // };
        // let key = rustc_def_to_my_def(tcx, key.to_def_id());
        // signatures.iter().find(|item| {
        //     item.id == key
        // }).unwrap();
        Some(DUMMY_SP)
    }

    fn generics_of<'tcx>(&self, tcx: TyCtxt<'tcx>, key: LocalDefId) -> Option<ty::Generics> {
        if self.contains_key(tcx, &key) {
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
        todo!()
    }

    fn fn_sig<'tcx>(
        &self,
        tcx: TyCtxt<'tcx>,
        def_id: LocalDefId,
    ) -> Option<ty::EarlyBinder<'tcx, ty::PolyFnSig<'tcx>>> {
        todo!()
    }

    fn predicates_of<'tcx>(
        &self,
        tcx: TyCtxt<'tcx>,
        key: RustcDefId,
    ) -> Option<ty::GenericPredicates<'tcx>> {
        todo!()
    }

    fn explicit_predicates_of<'tcx>(
        &self,
        tcx: TyCtxt<'tcx>,
        key: LocalDefId,
    ) -> Option<ty::GenericPredicates<'tcx>> {
        todo!()
    }

    fn codegen_fn_attrs<'tcx>(
        &self,
        tcx: TyCtxt<'tcx>,
        key: LocalDefId,
    ) -> Option<rustc_middle::middle::codegen_fn_attrs::CodegenFnAttrs> {
        todo!()
    }

    fn mir_built<'tcx>(
        &self,
        tcx: TyCtxt<'tcx>,
        def_id: LocalDefId,
    ) -> Option<&'tcx Steal<rustc_middle::mir::Body<'tcx>>> {
        let DefinedCrateState::Stage3(defined_crate, signatures, _, mirs) = self else {
            panic!("hir_crate query in stage {}", self.stage_id());
        };

        let key = rustc_def_to_my_def(tcx, def_id.to_def_id());
        let mir = mirs.iter().find(|item| item.id == key).unwrap();

        let body = build_mir_body(tcx, &mir.body, def_id);

        Some(unsafe { std::mem::transmute(leak(Steal::new(body))) })
    }

    fn mir_for_ctfe<'tcx>(
        &self,
        tcx: TyCtxt<'tcx>,
        key: LocalDefId,
    ) -> Option<&'tcx rustc_middle::mir::Body<'tcx>> {
        todo!()
    }

    fn mir_drops_elaborated_and_const_checked<'tcx>(
        &self,
        tcx: TyCtxt<'tcx>,
        key: LocalDefId,
    ) -> Option<&'tcx Steal<rustc_middle::mir::Body<'tcx>>> {
        todo!()
    }

    fn to_stage1(&mut self, defined_crate: DefinedCrateInfo) {
        let DefinedCrateState::Stage0 = self else {
            panic!("Moving to stage1 from stage {}", self.stage_id());
        };
        *self = DefinedCrateState::Stage1(defined_crate);
    }

    fn to_stage2(&mut self, sigs: Vec<ItemSignatureInfo>, foreign_mod_def: LocalDefId) {
        let this = std::mem::replace(self, DefinedCrateState::Stage0);
        let DefinedCrateState::Stage1(defined_crate) = this else {
            panic!("Moving to stage1 from stage {}", this.stage_id());
        };
        *self = DefinedCrateState::Stage2(defined_crate, sigs, foreign_mod_def);
    }

    fn to_stage3(&mut self, mirs: Vec<ItemMirInfo>) {
        let this = std::mem::replace(self, DefinedCrateState::Stage0);
        let DefinedCrateState::Stage2(defined_crate, sigs, foreign) = this else {
            panic!("Moving to stage3 from stage {}", this.stage_id());
        };
        *self = DefinedCrateState::Stage3(defined_crate, sigs, foreign, mirs);
    }
}

#[derive(Debug, Clone)]
pub struct DefinedItemInfo {
    pub name: String,
    pub kind: DefinedItemKind,
}

impl DefinedItemInfo {
    pub fn def_id(&self) -> rustc_public::DefId {
        match self.kind {
            DefinedItemKind::Function(fn_def) | DefinedItemKind::ForeignFunction(fn_def) => {
                fn_def.0
            }
            DefinedItemKind::Struct(adt_def) => adt_def.0,
            DefinedItemKind::CrateRoot(def_id)
            | DefinedItemKind::ForeignMod(def_id)
            | DefinedItemKind::Field(def_id) => def_id,
        }
    }

    pub fn fn_def(&self) -> Option<FnDef> {
        match self.kind {
            DefinedItemKind::Function(fn_def) => Some(fn_def),
            DefinedItemKind::ForeignFunction(fn_def) => Some(fn_def),
            _ => None,
        }
    }

    pub fn adt_def(&self) -> Option<AdtDef> {
        match self.kind {
            DefinedItemKind::Struct(adt) => Some(adt),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum DefinedItemKind {
    CrateRoot(DefId),
    ForeignMod(DefId),
    Function(FnDef),
    ForeignFunction(FnDef),
    Struct(AdtDef),
    Field(DefId),
}

#[derive(Debug, Clone)]
pub struct ItemMirInfo {
    pub id: rustc_public::DefId,
    pub body: MirBody,
}

#[derive(Debug, Clone)]
struct ForeignFunctionInfo {
    name: String,
}

#[derive(Debug, Clone)]
struct FunctionInfo {
    name: String,
    no_mangle: bool,
}

#[derive(Debug, Clone)]
struct StructInfo {
    name: String,
}

/// Run rustc_driver but emit a synthetic crate described by three callbacks.
///
/// Phase 1 (`define_items`) declares items and allocates their definitions.
/// Phase 2 (`define_signatures`) defines function signatures using allocated definitions.
/// Phase 3 (`emit_mir`) emits MIR bodies for generated local functions.
pub fn generate<D, S, M>(define_items: D, define_signatures: S, emit_mir: M)
where
    D: FnOnce(Context, DependencyInfo) -> CurrentCrateInfo + Send + 'static,
    S: FnOnce(Context, DependencyInfo, DefinedCrateInfo) -> Vec<ItemSignatureInfo> + Send + 'static,
    M: FnOnce(Context, DependencyInfo, DefinedCrateInfo) -> Vec<ItemMirInfo> + Send + 'static,
{
    generate_with_args(
        std::env::args().collect(),
        define_items,
        define_signatures,
        emit_mir,
    )
}

pub fn generate_with_args<D, S, M>(
    mut args: Vec<String>,
    define_items: D,
    define_signatures: S,
    emit_mir: M,
) where
    D: FnOnce(Context, DependencyInfo) -> CurrentCrateInfo + Send + 'static,
    S: FnOnce(Context, DependencyInfo, DefinedCrateInfo) -> Vec<ItemSignatureInfo> + Send + 'static,
    M: FnOnce(Context, DependencyInfo, DefinedCrateInfo) -> Vec<ItemMirInfo> + Send + 'static,
{
    if args.len() == 1 {
        // Provide a dummy crate name if invoked programmatically without args.
        args.push(String::from("--crate-name"));
        args.push(String::from("synthetic"));
        args.push(String::from("--crate-type=bin"));
        args.push(String::from("/dev/null"));
    }
    let mut callbacks = GenerateCallbacks::new(define_items, define_signatures, emit_mir);
    rustc_driver::run_compiler(&args, &mut callbacks);
}

struct GenerateCallbacks<D, S, M> {
    define_items: Option<D>,
    define_signatures: Option<S>,
    emit_mir: Option<M>,
    context: Context,
    gate: Arc<GenerateGate>,
}

#[derive(Debug, Default, Clone)]
enum DefinedCrateState {
    #[default]
    Stage0,
    Stage1(DefinedCrateInfo),
    Stage2(DefinedCrateInfo, Vec<ItemSignatureInfo>, LocalDefId),
    Stage3(
        DefinedCrateInfo,
        Vec<ItemSignatureInfo>,
        LocalDefId,
        Vec<ItemMirInfo>,
    ),
}

#[derive(Default)]
struct GenerateState {
    defined_crate: DefinedCrateState,
    original: Option<OriginalProviders>,
    define_items: Option<Box<dyn FnOnce(Context, DependencyInfo) -> CurrentCrateInfo + Send>>,
    define_signatures: Option<
        Box<dyn FnOnce(Context, DependencyInfo, DefinedCrateInfo) -> Vec<ItemSignatureInfo> + Send>,
    >,
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
    entry_fn: for<'tcx> fn(TyCtxt<'tcx>, ()) -> Option<(RustcDefId, EntryFnType)>,
    def_kind: for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> DefKind,
    def_span: for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> RustcSpan,
    def_ident_span: for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> Option<RustcSpan>,
    visibility: for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> ty::Visibility<RustcDefId>,
    generics_of: for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> ty::Generics,
    type_of: for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> ty::EarlyBinder<'tcx, ty::Ty<'tcx>>,
    fn_sig: for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> ty::EarlyBinder<'tcx, ty::PolyFnSig<'tcx>>,
    predicates_of: for<'tcx> fn(TyCtxt<'tcx>, RustcDefId) -> ty::GenericPredicates<'tcx>,
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
    f()
}

fn with_generated_and_original<'tcx, R>(
    tcx: TyCtxt<'tcx>,
    f: impl FnOnce(DefinedCrateState, OriginalProviders) -> R,
) -> R {
    let state = GENERATE_STATE
        .get()
        .cloned()
        .expect("generate state missing");
    // ensure_generated(tcx, &state);
    let (generated_ptr, original) = {
        let guard = state.state.lock().unwrap();
        let original = guard.original.expect("original providers missing");
        // let generated_ptr = guard.generated.as_ref().map(|g| g as *const GeneratedCrate);
        (guard.defined_crate.clone(), original)
    };
    // if std::env::var("GEN_DEBUG").is_ok() {
    //     eprintln!(
    //         "with_generated_and_original: generated={}",
    //         if generated_ptr.is_some() {
    //             "some"
    //         } else {
    //             "none"
    //         }
    //     );
    // }
    // let generated: Option<&GeneratedCrate> = generated_ptr.map(|ptr| unsafe { &*ptr });
    f(generated_ptr, original)
}

#[cfg(false)]
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
    let define_signatures = guard
        .define_signatures
        .take()
        .expect("define_signatures callback missing");
    let emit_mir = guard.emit_mir.take().expect("emit_mir callback missing");
    let context = guard.context.clone().expect("context missing");
    drop(guard);

    rustc_public::rustc_internal::run(tcx, || {
        let dependency_info = collect_dependency_info(tcx);
        let crate_info = define_items(context.clone(), dependency_info.clone());

        // let generated = GeneratedCrate::build(tcx, &context, dependency_info.clone(), crate_info);

        // {
        //     let mut guard = gate.state.lock().unwrap();
        //     guard.generated = Some(generated);
        // }

        // let generated_ptr = {
        //     let guard = gate.state.lock().unwrap();
        //     guard
        //         .generated
        //         .as_ref()
        //         .map(|g| g as *const GeneratedCrate)
        //         .expect("generated crate missing")
        // };
        // let generated_mut_ptr = generated_ptr as *mut GeneratedCrate;

        // let defined_for_signatures = unsafe { (&*generated_ptr).defined_info(tcx, false) };

        let defined_crate = allocate_ids_for_crate(crate_info);

        let signatures = define_signatures(
            context.clone(),
            dependency_info.clone(),
            defined_crate.clone(),
        );

        // Avoid holding gate mutex while phase callbacks run: they may re-enter
        // query providers that also try to acquire this mutex.
        // unsafe { (&mut *generated_mut_ptr).install_signatures(tcx, signatures) };

        // let defined_for_mir = unsafe { (&*generated_ptr).defined_info(tcx, true) };
        let item_mir = emit_mir(context.clone(), dependency_info, defined_crate);
        // unsafe { (&mut *generated_mut_ptr).install_mir(tcx, item_mir) };
    })
    .expect("failed to run rustc_public context");

    let mut guard = gate.state.lock().unwrap();
    guard.building = false;
    guard.building_thread = None;
    gate.cvar.notify_all();
}

#[allow(invalid_reference_casting)]
fn allocate_def_ids_for_crate<'tcx>(
    tcx: TyCtxt<'tcx>,
    crate_info: CurrentCrateInfo,
) -> (DefinedCrateInfo, LocalDefId) {
    let defs_guard = tcx.definitions_untracked();
    let defs_mut = unsafe { &mut *(&*defs_guard as *const Definitions as *mut Definitions) };
    let mut disamb =
        DisambiguatorState::with(CRATE_DEF_ID, DefPathData::ValueNs(Symbol::intern("gen")), 1);
    let foreign_mod = defs_mut.create_def(CRATE_DEF_ID, DefPathData::ForeignMod, &mut disamb);

    let mut result = DefinedCrateInfo {
        items: vec![
            DefinedItemInfo {
                name: crate_info.crate_name,
                kind: DefinedItemKind::CrateRoot(rustc_def_to_my_def(
                    tcx,
                    CRATE_DEF_ID.to_def_id(),
                )),
            },
            DefinedItemInfo {
                name: "".to_owned(),
                kind: DefinedItemKind::ForeignMod(rustc_def_to_my_def(
                    tcx,
                    foreign_mod.to_def_id(),
                )),
            },
        ],
    };

    for item in crate_info.items {
        let info = DefinedItemInfo {
            kind: match item.kind {
                ItemKind::Module => todo!(),
                ItemKind::ForeignFunction => {
                    let def_id = defs_mut.create_def(
                        foreign_mod,
                        DefPathData::ValueNs(Symbol::intern(&item.name)),
                        &mut disamb,
                    );
                    DefinedItemKind::ForeignFunction(FnDef(rustc_def_to_my_def(
                        tcx,
                        def_id.to_def_id(),
                    )))
                }
                ItemKind::Function => {
                    let def_id = defs_mut.create_def(
                        CRATE_DEF_ID,
                        DefPathData::ValueNs(Symbol::intern(&item.name)),
                        &mut disamb,
                    );
                    DefinedItemKind::Function(FnDef(rustc_def_to_my_def(tcx, def_id.to_def_id())))
                }
                ItemKind::Struct(fields) => {
                    let def_id = defs_mut.create_def(
                        CRATE_DEF_ID,
                        DefPathData::TypeNs(Symbol::intern(&item.name)),
                        &mut disamb,
                    );
                    for field in fields {
                        let field_id = defs_mut.create_def(
                            def_id,
                            DefPathData::TypeNs(Symbol::intern(&field)),
                            &mut disamb,
                        );
                        result.items.push(DefinedItemInfo {
                            name: field,
                            kind: DefinedItemKind::Field(rustc_def_to_my_def(
                                tcx,
                                field_id.to_def_id(),
                            )),
                        });
                    }
                    DefinedItemKind::Struct(AdtDef(rustc_def_to_my_def(tcx, def_id.to_def_id())))
                }
                ItemKind::Enum => todo!(),
            },
            name: item.name,
        };
        result.items.push(info)
    }

    // for strukt in structs {
    //     let def_id = defs_mut.create_def(
    //         CRATE_DEF_ID,
    //         DefPathData::TypeNs(Symbol::intern(&strukt.name)),
    //         &mut disamb,
    //     );
    //     struct_ids.insert(strukt.id, def_id);
    // }
    // let mut function_ids = FxHashMap::default();
    // for function in functions {
    //     let def_id = defs_mut.create_def(
    //         CRATE_DEF_ID,
    //         DefPathData::ValueNs(Symbol::intern(&function.name)),
    //         &mut disamb,
    //     );
    //     function_ids.insert(function.id, def_id);
    // }
    // let mut foreign_function_ids = FxHashMap::default();
    // for foreign in foreign_functions {
    //     let def_id = defs_mut.create_def(
    //         foreign_mod,
    //         DefPathData::ValueNs(Symbol::intern(&foreign.name)),
    //         &mut disamb,
    //     );
    //     foreign_function_ids.insert(foreign.id, def_id);
    // }
    (result, foreign_mod)
}

fn rustc_def_to_my_def<'tcx>(tcx: TyCtxt<'tcx>, def_id: RustcDefId) -> DefId {
    rustc_public::compiler_interface::with(|cx| unsafe {
        (*cx.tables.as_ptr()).def_ids.create_or_fetch(def_id)
    })
}

impl<D, S, M> GenerateCallbacks<D, S, M>
where
    D: FnOnce(Context, DependencyInfo) -> CurrentCrateInfo + Send + 'static,
    S: FnOnce(Context, DependencyInfo, DefinedCrateInfo) -> Vec<ItemSignatureInfo> + Send + 'static,
    M: FnOnce(Context, DependencyInfo, DefinedCrateInfo) -> Vec<ItemMirInfo> + Send + 'static,
{
    fn new(define_items: D, define_signatures: S, emit_mir: M) -> Self {
        Self {
            define_items: Some(define_items),
            define_signatures: Some(define_signatures),
            emit_mir: Some(emit_mir),
            context: Context::new(),
            gate: Arc::new(GenerateGate {
                state: Mutex::new(GenerateState::default()),
                cvar: Condvar::new(),
            }),
        }
    }
}

impl<D, S, M> rustc_driver::Callbacks for GenerateCallbacks<D, S, M>
where
    D: FnOnce(Context, DependencyInfo) -> CurrentCrateInfo + Send + 'static,
    S: FnOnce(Context, DependencyInfo, DefinedCrateInfo) -> Vec<ItemSignatureInfo> + Send + 'static,
    M: FnOnce(Context, DependencyInfo, DefinedCrateInfo) -> Vec<ItemMirInfo> + Send + 'static,
{
    fn config(&mut self, config: &mut rustc_interface::Config) {
        let define_items = self
            .define_items
            .take()
            .expect("define_items callback already used");
        let define_signatures = self
            .define_signatures
            .take()
            .expect("define_signatures callback already used");
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
            dbg!("locked");
            let mut guard = gate.state.lock().unwrap();
            if std::env::var("GEN_DEBUG").is_ok() {
                eprintln!("callbacks.config: storing callback");
            }
            guard.define_items = Some(Box::new(define_items));
            guard.define_signatures = Some(Box::new(define_signatures));
            guard.emit_mir = Some(Box::new(emit_mir));
            guard.context = Some(self.context.clone());
        }
    }

    fn after_expansion<'tcx>(
        &mut self,
        _compiler: &rustc_interface::interface::Compiler,
        tcx: TyCtxt<'tcx>,
    ) -> rustc_driver::Compilation {
        rustc_public::rustc_internal::run(tcx, || {
            let dependency_info = collect_dependency_info(tcx);

            let gate = GENERATE_STATE.get().unwrap();
            let (define_sigs, context, defined_crate, foreign_mod_def) = {
                let mut guard = gate.state.lock().unwrap();

                let context = guard.context.clone().unwrap();

                let current_crate_info =
                    guard.define_items.take().unwrap()(context.clone(), dependency_info.clone());

                context.register_with_source_map(tcx);

                let (defined_crate, foreign_mod_def) =
                    allocate_def_ids_for_crate(tcx, current_crate_info);

                guard.defined_crate.to_stage1(defined_crate.clone());

                (
                    guard.define_signatures.take().unwrap(),
                    context,
                    defined_crate,
                    foreign_mod_def,
                )
            };
            let sigs = define_sigs(
                context.clone(),
                dependency_info.clone(),
                defined_crate.clone(),
            );
            let emit_mir = {
                let mut guard = gate.state.lock().unwrap();
                guard.defined_crate.to_stage2(sigs, foreign_mod_def);

                guard.emit_mir.take().unwrap()
            };

            tcx.hir_crate(());

            let mirs = emit_mir(context, dependency_info, defined_crate);

            let mut guard = gate.state.lock().unwrap();
            guard.defined_crate.to_stage3(mirs);
        });
        rustc_driver::Compilation::Continue
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
            let def_id = RustcDefId {
                krate: cnum,
                index: rustc_span::def_id::DefIndex::from_usize(idx),
            };
            collect_dependency_def(tcx, def_id, &mut info, &mut alloc_fn_id);
        }
    }

    // Also expose local crate definitions so generated items can refer to user-defined
    // structs/functions from the current compilation unit.
    let local_count = tcx.definitions_untracked().def_index_count();
    for idx in 0..local_count {
        let def_id = RustcDefId {
            krate: rustc_hir::def_id::LOCAL_CRATE,
            index: rustc_span::def_id::DefIndex::from_usize(idx),
        };
        collect_dependency_def(tcx, def_id, &mut info, &mut alloc_fn_id);
    }

    info
}

fn collect_dependency_def<'tcx>(
    tcx: TyCtxt<'tcx>,
    def_id: RustcDefId,
    info: &mut DependencyInfo,
    alloc_fn_id: &mut impl FnMut() -> FunctionId,
) {
    let kind = tcx.def_kind(def_id);

    if matches!(
        kind,
        DefKind::Fn | DefKind::AssocFn | DefKind::Ctor(_, CtorKind::Fn)
    ) {
        let id = alloc_fn_id();
        let hash = tcx.def_path_hash(def_id);
        let (hi, lo): (u64, u64) =
            unsafe { std::mem::transmute::<Fingerprint, (u64, u64)>(hash.0) };
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

fn stable_adt_from_def_id<'tcx>(tcx: TyCtxt<'tcx>, def_id: RustcDefId) -> Option<AdtDef> {
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
    def_id: RustcDefId,
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
    dbg!("locked");
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
    // providers.local_def_id_to_hir_id = generated_local_def_id_to_hir_id;
    // providers.opt_hir_owner_nodes = generated_opt_hir_owner_nodes;
    providers.hir_owner_parent_q = generated_hir_owner_parent_q;
    providers.hir_attr_map = generated_hir_attr_map;
    providers.opt_ast_lowering_delayed_lints = generated_opt_ast_lowering_delayed_lints;
    providers.entry_fn = generated_entry_fn;
    providers.def_kind = generated_def_kind;
    providers.def_span = generated_def_span;
    providers.def_ident_span = generated_def_ident_span;
    providers.visibility = generated_visibility;
    // providers.generics_of = generated_generics_of;
    // providers.type_of = generated_type_of;
    // providers.fn_sig = generated_fn_sig;
    // providers.predicates_of = generated_predicates_of;
    // providers.explicit_predicates_of = generated_explicit_predicates_of;
    // providers.codegen_fn_attrs = generated_codegen_fn_attrs;
    providers.mir_built = generated_mir_built;
    // providers.mir_for_ctfe = generated_mir_for_ctfe;
    // providers.mir_drops_elaborated_and_const_checked =
    //     generated_mir_drops_elaborated_and_const_checked;
    // providers.optimized_mir = generated_optimized_mir;
}

fn generated_hir_crate<'tcx>(tcx: TyCtxt<'tcx>, key: ()) -> hir::Crate<'tcx> {
    if std::env::var("GEN_DEBUG").is_ok() {
        eprintln!("generated_hir_crate");
    }
    with_generated_and_original(tcx, |generated, _| generated.hir_crate(tcx, key))
}

#[cfg(false)]
fn generated_opt_hir_owner_nodes<'tcx>(
    tcx: TyCtxt<'tcx>,
    key: LocalDefId,
) -> Option<&'tcx hir::OwnerNodes<'tcx>> {
    dbg!(std::panic::Location::caller());
    if std::env::var("GEN_DEBUG").is_ok() {
        eprintln!("generated_opt_hir_owner_nodes {:?}", key);
    }
    with_generated_and_original(tcx, |generated, original| {
        generated.opt_hir_owner_nodes(tcx, key)
        // original.opt_hir_owner_nodes
    })
}

fn generated_local_def_id_to_hir_id<'tcx>(tcx: TyCtxt<'tcx>, key: LocalDefId) -> HirId {
    with_generated_and_original(tcx, |generated, original| match generated {
        DefinedCrateState::Stage0 => panic!("Invalid"),
        DefinedCrateState::Stage1(defined_crate_info)
        | DefinedCrateState::Stage2(defined_crate_info, _, _)
        | DefinedCrateState::Stage3(defined_crate_info, _, _, _) => {
            let item_id = defined_crate_info
                .items
                .iter()
                .find(|item| my_def_id_to_rustc_def_id(tcx, item.def_id()).as_local() == Some(key))
                .unwrap();
            match &item_id.kind {
                DefinedItemKind::Field(field_id) => todo!(),
                _ => HirId::make_owner(key),
            }
        }
    })
}

fn generated_hir_owner_parent_q<'tcx>(tcx: TyCtxt<'tcx>, key: OwnerId) -> HirId {
    with_generated_and_original(tcx, |generated, _| generated.hir_owner_parent_q(tcx, key))
}

fn generated_hir_attr_map<'tcx>(tcx: TyCtxt<'tcx>, key: OwnerId) -> &'tcx hir::AttributeMap<'tcx> {
    return hir::AttributeMap::EMPTY;
}

fn generated_opt_ast_lowering_delayed_lints<'tcx>(
    tcx: TyCtxt<'tcx>,
    key: OwnerId,
) -> Option<&'tcx hir::lints::DelayedLints> {
    with_generated_and_original(tcx, |generated, original| {
        if generated.contains_key(tcx, &key.def_id) {
            return None;
        }
        (original.hir_crate)(tcx, ()).owners[key.def_id]
            .as_owner()
            .map(|o| &o.delayed_lints)
    })
}

fn generated_entry_fn<'tcx>(tcx: TyCtxt<'tcx>, key: ()) -> Option<(RustcDefId, EntryFnType)> {
    with_generated_and_original(tcx, |generated, original| generated.entry_fn(tcx, key))
}

fn generated_def_kind<'tcx>(tcx: TyCtxt<'tcx>, key: LocalDefId) -> DefKind {
    if std::env::var("GEN_DEBUG").is_ok() {
        eprintln!("generated_def_kind {:?}", key);
    }
    with_generated_and_original(tcx, |generated, original| generated.def_kind(tcx, key))
}

fn generated_def_span<'tcx>(tcx: TyCtxt<'tcx>, key: LocalDefId) -> RustcSpan {
    with_generated_and_original(tcx, |generated, original| {
        if let Some(span) = generated.def_span(tcx, key) {
            return span;
        }

        (original.def_span)(tcx, key)
    })
}

fn generated_def_ident_span<'tcx>(tcx: TyCtxt<'tcx>, key: LocalDefId) -> Option<RustcSpan> {
    with_generated_and_original(tcx, |generated, original| {
        if let Some(span) = generated.def_span(tcx, key) {
            return Some(span);
        }

        (original.def_ident_span)(tcx, key)
    })
}

fn generated_visibility<'tcx>(_tcx: TyCtxt<'tcx>, _key: LocalDefId) -> ty::Visibility<RustcDefId> {
    ty::Visibility::Public
}

fn generated_generics_of<'tcx>(tcx: TyCtxt<'tcx>, key: LocalDefId) -> ty::Generics {
    with_generated_and_original(tcx, |generated, original| {
        if let Some(generics) = generated.generics_of(tcx, key) {
            return generics;
        }

        (original.generics_of)(tcx, key)
    })
}

fn generated_type_of<'tcx>(
    tcx: TyCtxt<'tcx>,
    key: LocalDefId,
) -> ty::EarlyBinder<'tcx, ty::Ty<'tcx>> {
    with_generated_and_original(tcx, |generated, original| {
        if let Some(ty) = generated.type_of(tcx, key) {
            return ty;
        }

        (original.type_of)(tcx, key)
    })
}

fn generated_fn_sig<'tcx>(
    tcx: TyCtxt<'tcx>,
    key: LocalDefId,
) -> ty::EarlyBinder<'tcx, ty::PolyFnSig<'tcx>> {
    with_generated_and_original(tcx, |generated, original| {
        if let Some(sig) = generated.fn_sig(tcx, key) {
            return sig;
        }

        (original.fn_sig)(tcx, key)
    })
}

fn generated_predicates_of<'tcx>(
    tcx: TyCtxt<'tcx>,
    key: RustcDefId,
) -> ty::GenericPredicates<'tcx> {
    with_generated_and_original(tcx, |generated, original| {
        if let Some(preds) = generated.predicates_of(tcx, key) {
            return preds;
        }

        (original.predicates_of)(tcx, key)
    })
}

fn generated_explicit_predicates_of<'tcx>(
    tcx: TyCtxt<'tcx>,
    key: LocalDefId,
) -> ty::GenericPredicates<'tcx> {
    with_generated_and_original(tcx, |generated, original| {
        if let Some(preds) = generated.explicit_predicates_of(tcx, key) {
            return preds;
        }

        (original.explicit_predicates_of)(tcx, key)
    })
}

fn generated_codegen_fn_attrs<'tcx>(
    tcx: TyCtxt<'tcx>,
    key: LocalDefId,
) -> rustc_middle::middle::codegen_fn_attrs::CodegenFnAttrs {
    with_generated_and_original(tcx, |generated, original| {
        if let Some(attrs) = generated.codegen_fn_attrs(tcx, key) {
            return attrs;
        }

        (original.codegen_fn_attrs)(tcx, key)
    })
}

fn generated_mir_built<'tcx>(
    tcx: TyCtxt<'tcx>,
    key: LocalDefId,
) -> &'tcx Steal<rustc_middle::mir::Body<'tcx>> {
    with_generated_and_original(tcx, |generated, original| {
        if let Some(body) = generated.mir_built(tcx, key) {
            return body;
        }

        (original.mir_built)(tcx, key)
    })
}

fn generated_mir_for_ctfe<'tcx>(
    tcx: TyCtxt<'tcx>,
    key: LocalDefId,
) -> &'tcx rustc_middle::mir::Body<'tcx> {
    with_generated_and_original(tcx, |generated, original| {
        if let Some(body) = generated.mir_for_ctfe(tcx, key) {
            return body;
        }

        (original.mir_for_ctfe)(tcx, key)
    })
}

fn generated_mir_drops_elaborated_and_const_checked<'tcx>(
    tcx: TyCtxt<'tcx>,
    key: LocalDefId,
) -> &'tcx Steal<rustc_middle::mir::Body<'tcx>> {
    with_generated_and_original(tcx, |generated, original| {
        if let Some(body) = generated.mir_drops_elaborated_and_const_checked(tcx, key) {
            return body;
        }

        (original.mir_drops_elaborated_and_const_checked)(tcx, key)
    })
}

fn generated_optimized_mir<'tcx>(
    tcx: TyCtxt<'tcx>,
    key: LocalDefId,
) -> &'tcx rustc_middle::mir::Body<'tcx> {
    with_generated_and_original(tcx, |generated, original| {
        // if let Some(generated_crate) = generated {
        //     if let Some(body) = generated_crate.optimized_mir(tcx, key) {
        //         return body;
        //     }
        // }
        (original.optimized_mir)(tcx, key)
    })
}

#[cfg(false)]
#[allow(invalid_reference_casting)]
fn allocate_def_ids<'tcx>(
    tcx: TyCtxt<'tcx>,
    structs: &[StructInfo],
    functions: &[FunctionInfo],
    foreign_functions: &[ForeignFunctionInfo],
) -> (
    LocalDefId,
    FxHashMap<ItemId, LocalDefId>,
    FxHashMap<ItemId, LocalDefId>,
    FxHashMap<ItemId, LocalDefId>,
) {
    let defs_guard = tcx.definitions_untracked();
    let defs_mut = unsafe { &mut *(&*defs_guard as *const Definitions as *mut Definitions) };
    let mut disamb =
        DisambiguatorState::with(CRATE_DEF_ID, DefPathData::ValueNs(Symbol::intern("gen")), 1);
    let foreign_mod = defs_mut.create_def(CRATE_DEF_ID, DefPathData::ForeignMod, &mut disamb);
    let mut struct_ids = FxHashMap::default();
    for strukt in structs {
        let def_id = defs_mut.create_def(
            CRATE_DEF_ID,
            DefPathData::TypeNs(Symbol::intern(&strukt.name)),
            &mut disamb,
        );
        struct_ids.insert(strukt.id, def_id);
    }
    let mut function_ids = FxHashMap::default();
    for function in functions {
        let def_id = defs_mut.create_def(
            CRATE_DEF_ID,
            DefPathData::ValueNs(Symbol::intern(&function.name)),
            &mut disamb,
        );
        function_ids.insert(function.id, def_id);
    }
    let mut foreign_function_ids = FxHashMap::default();
    for foreign in foreign_functions {
        let def_id = defs_mut.create_def(
            foreign_mod,
            DefPathData::ValueNs(Symbol::intern(&foreign.name)),
            &mut disamb,
        );
        foreign_function_ids.insert(foreign.id, def_id);
    }
    (foreign_mod, struct_ids, function_ids, foreign_function_ids)
}

#[cfg(false)]
struct GeneratedCrate {
    #[allow(dead_code)]
    crate_name: Symbol,
    context: Context,
    struct_infos: LocalDefIdMap<StructInfo>,
    foreign_function_infos: LocalDefIdMap<ForeignFunctionInfo>,
    foreign_function_sigs: LocalDefIdMap<(
        Vec<ty::Ty<'static>>,
        ty::Ty<'static>,
        hir::Safety,
        ExternAbi,
    )>,
    foreign_function_symbols: LocalDefIdMap<Symbol>,
    function_sigs: LocalDefIdMap<(
        Vec<ty::Ty<'static>>,
        ty::Ty<'static>,
        hir::Safety,
        ExternAbi,
    )>,
    function_symbols: LocalDefIdMap<Symbol>,
    field_types: LocalDefIdMap<ty::Ty<'static>>,
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

fn build_owner_nodes_for_crate(root: &'static hir::Mod<'static>) -> hir::OwnerNodes<'static> {
    let mut nodes: IndexVec<ItemLocalId, hir::ParentedNode<'static>> = IndexVec::new();
    nodes.push(hir::ParentedNode {
        parent: ItemLocalId::INVALID,
        node: hir::Node::Crate(root),
    });

    hir::OwnerNodes {
        opt_hash_including_bodies: Some(random_fingerprint()),
        nodes,
        bodies: rustc_data_structures::sorted_map::SortedMap::new(),
    }
}

fn random_fingerprint() -> Fingerprint {
    Fingerprint::new::<u64, u64>(rand::random(), rand::random())
}

fn build_owner_nodes_for_struct(
    item: &'static hir::Item<'static>,
    fields: &'static [hir::FieldDef<'static>],
) -> hir::OwnerNodes<'static> {
    let mut nodes: IndexVec<ItemLocalId, hir::ParentedNode<'static>> = IndexVec::new();

    // Node 0: the struct item itself
    nodes.push(hir::ParentedNode {
        parent: ItemLocalId::INVALID,
        node: hir::Node::Item(item),
    });

    // Nodes 1..N: the fields as children with parent = 0
    for (idx, field) in fields.iter().enumerate() {
        nodes.push(hir::ParentedNode {
            parent: ItemLocalId::ZERO, // Parent is the struct item (index 0)
            node: hir::Node::Field(field),
        });
    }

    hir::OwnerNodes {
        opt_hash_including_bodies: Some(random_fingerprint()),
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
        opt_hash_including_bodies: Some(random_fingerprint()),
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
        opt_hash_including_bodies: Some(random_fingerprint()),
        nodes,
        bodies: rustc_data_structures::sorted_map::SortedMap::new(),
    }
}

fn build_owner_nodes_for_field(field: &'static hir::FieldDef<'static>) -> hir::OwnerNodes<'static> {
    let mut nodes: IndexVec<ItemLocalId, hir::ParentedNode<'static>> = IndexVec::new();
    nodes.push(hir::ParentedNode {
        parent: ItemLocalId::INVALID,
        node: hir::Node::Field(field),
    });

    hir::OwnerNodes {
        opt_hash_including_bodies: Some(random_fingerprint()),
        nodes,
        bodies: rustc_data_structures::sorted_map::SortedMap::new(),
    }
}

fn build_owner_nodes_for_fn(
    item: &'static hir::Item<'static>,
    body: &'static hir::Body<'static>,
    expr: &'static hir::Expr<'static>,
    loop_expr: &'static hir::Block<'static>,
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
    nodes.push(hir::ParentedNode {
        parent: ItemLocalId::ZERO,
        node: hir::Node::Block(loop_expr),
    });

    let mut bodies = rustc_data_structures::sorted_map::SortedMap::new();
    bodies.insert(ItemLocalId::new(1), body);

    hir::OwnerNodes {
        opt_hash_including_bodies: Some(random_fingerprint()),
        nodes,
        bodies,
    }
}

fn build_mir_body<'tcx>(
    tcx: TyCtxt<'tcx>,
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
            MirProjection::Field(field, ty) => proj.push(rustc_middle::mir::PlaceElem::Field(
                rustc_abi::FieldIdx::from_usize(*field),
                mir_ty_to_rustc(tcx, ty),
            )),
            _ => todo!("projection elem not supported yet: {elem:?}"),
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
                rustc_public::mir::AggregateKind::Tuple => rustc_middle::mir::AggregateKind::Tuple,
                rustc_public::mir::AggregateKind::RawPtr(ty, mutability) => {
                    rustc_middle::mir::AggregateKind::RawPtr(
                        mir_ty_to_rustc(tcx, ty),
                        match mutability {
                            rustc_public::mir::Mutability::Mut => {
                                rustc_middle::mir::Mutability::Mut
                            }
                            rustc_public::mir::Mutability::Not => {
                                rustc_middle::mir::Mutability::Not
                            }
                        },
                    )
                }
                rustc_public::mir::AggregateKind::Adt(
                    adt,
                    variant_idx,
                    args,
                    user_ty,
                    active_field,
                ) => {
                    let adt_def_id = my_def_id_to_rustc_def_id(tcx, adt.0);
                    let args = generic_args_to_rustc_with_locals(tcx, args);
                    rustc_middle::mir::AggregateKind::Adt(
                        adt_def_id,
                        rustc_abi::VariantIdx::from_usize(variant_idx_to_usize(*variant_idx)),
                        args,
                        user_ty.map(rustc_middle::ty::UserTypeAnnotationIndex::from_usize),
                        active_field.map(rustc_abi::FieldIdx::from_usize),
                    )
                }
                _ => todo!("aggregate kind not supported yet: {kind:?}"),
            };
            rustc_middle::mir::Rvalue::Aggregate(
                Box::new(kind),
                ops.iter().map(|op| mir_operand_to_rustc(tcx, op)).collect(),
            )
        }
        MirRvalue::BinaryOp(op, lhs, rhs) => rustc_middle::mir::Rvalue::BinaryOp(
            mir_bin_op_to_rustc(*op),
            Box::new((
                mir_operand_to_rustc(tcx, lhs),
                mir_operand_to_rustc(tcx, rhs),
            )),
        ),
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
                                        rustc_public::mir::Safety::Unsafe => {
                                            rustc_hir::Safety::Unsafe
                                        }
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
                                        rustc_public::mir::Safety::Unsafe => {
                                            rustc_hir::Safety::Unsafe
                                        }
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
        _ => todo!("rvalue not supported yet: {rvalue:?}"),
    }
}

fn mir_bin_op_to_rustc(op: rustc_public::mir::BinOp) -> rustc_middle::mir::BinOp {
    match op {
        rustc_public::mir::BinOp::Add => rustc_middle::mir::BinOp::Add,
        rustc_public::mir::BinOp::AddUnchecked => rustc_middle::mir::BinOp::AddUnchecked,
        rustc_public::mir::BinOp::Sub => rustc_middle::mir::BinOp::Sub,
        rustc_public::mir::BinOp::SubUnchecked => rustc_middle::mir::BinOp::SubUnchecked,
        rustc_public::mir::BinOp::Mul => rustc_middle::mir::BinOp::Mul,
        rustc_public::mir::BinOp::MulUnchecked => rustc_middle::mir::BinOp::MulUnchecked,
        rustc_public::mir::BinOp::Div => rustc_middle::mir::BinOp::Div,
        rustc_public::mir::BinOp::Rem => rustc_middle::mir::BinOp::Rem,
        rustc_public::mir::BinOp::BitXor => rustc_middle::mir::BinOp::BitXor,
        rustc_public::mir::BinOp::BitAnd => rustc_middle::mir::BinOp::BitAnd,
        rustc_public::mir::BinOp::BitOr => rustc_middle::mir::BinOp::BitOr,
        rustc_public::mir::BinOp::Shl => rustc_middle::mir::BinOp::Shl,
        rustc_public::mir::BinOp::ShlUnchecked => rustc_middle::mir::BinOp::ShlUnchecked,
        rustc_public::mir::BinOp::Shr => rustc_middle::mir::BinOp::Shr,
        rustc_public::mir::BinOp::ShrUnchecked => rustc_middle::mir::BinOp::ShrUnchecked,
        rustc_public::mir::BinOp::Eq => rustc_middle::mir::BinOp::Eq,
        rustc_public::mir::BinOp::Lt => rustc_middle::mir::BinOp::Lt,
        rustc_public::mir::BinOp::Le => rustc_middle::mir::BinOp::Le,
        rustc_public::mir::BinOp::Ne => rustc_middle::mir::BinOp::Ne,
        rustc_public::mir::BinOp::Ge => rustc_middle::mir::BinOp::Ge,
        rustc_public::mir::BinOp::Gt => rustc_middle::mir::BinOp::Gt,
        rustc_public::mir::BinOp::Cmp => rustc_middle::mir::BinOp::Cmp,
        rustc_public::mir::BinOp::Offset => rustc_middle::mir::BinOp::Offset,
    }
}

fn generic_args_to_rustc_with_locals<'tcx>(
    tcx: TyCtxt<'tcx>,
    args: &GenericArgs,
) -> ty::GenericArgsRef<'tcx> {
    let mut rustc_args = Vec::with_capacity(args.0.len());
    for arg in &args.0 {
        let rustc_arg = match arg {
            GenericArgKind::Lifetime(region) => {
                ty::GenericArg::from(mir_region_to_rustc(tcx, region))
            }
            GenericArgKind::Type(ty) => ty::GenericArg::from(mir_ty_to_rustc(tcx, ty)),
            GenericArgKind::Const(konst) => {
                use rustc_public::rustc_internal::internal;
                let konst = run_with_public_context(tcx, || internal(tcx, konst.clone()));
                ty::GenericArg::from(konst)
            }
        };
        rustc_args.push(rustc_arg);
    }
    tcx.mk_args(&rustc_args)
}

fn mir_adt_to_rustc<'tcx>(tcx: TyCtxt<'tcx>, adt: AdtDef) -> ty::AdtDef<'tcx> {
    let ty = mir_ty_to_rustc(tcx, &adt.ty());
    match ty.kind() {
        rustc_middle::ty::TyKind::Adt(adt_def, _) => *adt_def,
        _ => panic!("expected ADT type for {adt:?}, got {:?}", ty.kind()),
    }
}

fn variant_idx_to_usize(idx: VariantIdx) -> usize {
    unsafe { std::mem::transmute::<VariantIdx, usize>(idx) }
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

fn insert_non_owner(
    owners: &mut IndexVec<LocalDefId, hir::MaybeOwner<'static>>,
    def_id: LocalDefId,
    info: hir::MaybeOwner<'static>,
) {
    if def_id.index() >= owners.len() {
        owners.resize(def_id.index() + 1, hir::MaybeOwner::Phantom);
    }
    owners[def_id] = info;
}

fn insert_owner(
    owners: &mut IndexVec<LocalDefId, hir::MaybeOwner<'static>>,
    def_id: LocalDefId,
    info: &'static hir::OwnerInfo<'static>,
) {
    insert_non_owner(owners, def_id, hir::MaybeOwner::Owner(info));
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

fn make_adt_ty<'tcx>(tcx: TyCtxt<'tcx>, owner: LocalDefId, adt: AdtDef) -> hir::Ty<'static> {
    let (def_id, kind) = {
        let def_id = my_def_id_to_rustc_def_id(tcx, adt.0);
        let kind = match tcx.def_kind(def_id) {
            DefKind::Struct => DefKind::Struct,
            DefKind::Union => DefKind::Union,
            DefKind::Enum => DefKind::Enum,
            other => panic!("expected ADT def kind, found {other:?} for {:?}", def_id),
        };
        (def_id, kind)
    };
    let ident = Ident::from_str(tcx.item_name(def_id).as_str());
    let segment = hir::PathSegment::new(ident, HirId::make_owner(owner), Res::Def(kind, def_id));
    let segments = leak(vec![segment].into_boxed_slice());
    let path = leak(hir::Path {
        span: DUMMY_SP,
        res: Res::Def(kind, def_id),
        segments,
    });
    hir::Ty {
        hir_id: HirId::make_owner(owner),
        span: DUMMY_SP,
        kind: hir::TyKind::Path(hir::QPath::Resolved(None, path)),
    }
}

fn hir_ty_to_rustc<'tcx>(tcx: TyCtxt<'tcx>, owner: LocalDefId, ty: &HirTy) -> hir::Ty<'static> {
    match &ty.kind {
        HirTyKind::Int(int_ty) => {
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
        HirTyKind::Uint(int_ty) => {
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
        HirTyKind::RawPtr(mutability, to) => {
            let pointee = leak(hir_ty_to_rustc(tcx, owner, &to));
            make_ptr_ty(
                owner,
                pointee,
                match mutability {
                    rustc_public::mir::Mutability::Not => hir::Mutability::Not,
                    rustc_public::mir::Mutability::Mut => hir::Mutability::Mut,
                },
            )
        }
        HirTyKind::Adt(adt, _args) => make_adt_ty(tcx, owner, *adt),
        HirTyKind::Tuple(elems) if elems.is_empty() => make_unit_ty(owner),
        _ => todo!("hir type support for {:?}", ty),
    }
}

fn leak<T>(value: T) -> &'static T {
    Box::leak(Box::new(value))
}
