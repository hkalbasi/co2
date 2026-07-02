//! The bridge between rustc and the compiler which uses rustc_public types to emit a crate.
//!
//! The content of this file is mostly LLM generated. This file is probably the most complex file in the project,
//! and it has lowest quality. It frequently uses unsafe blocks, some of them are blatantly unsound.
//! I spent much time and tokens to make it happen, and it works, but it is very fragile.
//! I tried many times to change parts of it, at least fixing its unsafe usages. But even minor changes make test fail,
//! in a way that you can only fix by reverting your change.
//!
//! If you think your LLM is very smart, try to remove this file and reimplementing it (ideally without unsafe) while passing the tests.
//! I guess no LLM can do this. If you are a human and want to fix this file by hand, thank you! But be aware that it is not a simple task,
//! and it probably needs a good expertise on rustc internals.

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::{Arc, Mutex};

use rustc_abi::ExternAbi;
use rustc_ast::token::{CommentKind, DocFragmentKind, Token};
use rustc_ast::tokenstream::{DelimSpan, TokenStream, TokenTree};
use rustc_ast::{Attribute, FloatTy, IntTy, Mutability as AstMutability, UintTy};
use rustc_data_structures::fingerprint::Fingerprint;
use rustc_data_structures::fx::{FxHashMap, FxIndexMap};
use rustc_data_structures::packed::Pu128;
use rustc_data_structures::smallvec::SmallVec;
use rustc_data_structures::steal::Steal;
use rustc_data_structures::thin_vec::ThinVec;
use rustc_errors::LintBuffer;
use rustc_hir as hir;
use rustc_hir::def::{CtorKind, DefKind, Namespace, Res};
use rustc_hir::def_id::{CRATE_DEF_ID, DefId as RustcDefId, LocalDefId, LocalDefIdMap};
use rustc_hir::definitions::{DefPathData, PerParentDisambiguatorState};
use rustc_hir::lang_items::LangItem;
use rustc_hir::{HirId, ItemLocalId, ItemLocalMap, OwnerId};
use rustc_hir_analysis::autoderef::{Autoderef, AutoderefKind};
use rustc_index::{Idx, IndexVec};
use rustc_infer::infer::DefineOpaqueTypes;
use rustc_infer::traits::{ObligationCause, PredicateObligation};
use rustc_lint::Level;
use rustc_middle::mir::interpret::{CtfeProvenance, Pointer, Scalar};
use rustc_middle::mir::{BorrowKind, ConstValue};
use rustc_middle::queries::mir_borrowck::ProvidedValue as BorrowckProvidedValue;
use rustc_middle::query::Providers as QueryProviders;
use rustc_middle::ty::{
    self, GenericParamDefKind, TyCtxt, TypeVisitableExt, fast_reject::SimplifiedType,
};
use rustc_middle::util::Providers as UtilProviders;
use rustc_public::rustc_internal::internal;
use rustc_session::config::{CrateType, EntryFnType};
use rustc_span::symbol::{Ident, Symbol};
use rustc_span::{BytePos, DUMMY_SP, Span as RustcSpan, SyntaxContext};
use rustc_trait_selection::infer::{InferCtxtExt as _, TyCtxtInferExt as _};
use rustc_trait_selection::traits::ObligationCtxt;
use rustc_trait_selection::traits::query::evaluate_obligation::InferCtxtExt;

use crate::hir_structure::{
    AdtRepr, FunctionAbi, FunctionSignature, GeneratedAttr, InlineHint, StructField, Visibility,
};
use crate::hir_ty::HirTyConst;
pub use crate::hir_ty::{HirTy, HirTyKind};
use crate::{
    CrateGeneratorState, DependencyChild, DependencyChildKind, DependencyConstValue,
    DependencyCrate, FileId, ImplFunction, ReceiverAdjustment, ReceiverAdjustmentStep,
    ResolvedMethod,
};
use rustc_public::DefId;
use rustc_public::mir::{
    Body as MirBody, ConstOperand as MirConst, Mutability as MirMutability, Operand as MirOperand,
    Place as MirPlace, ProjectionElem as MirProjection, Rvalue as MirRvalue,
    StatementKind as MirStatementKind, TerminatorKind as MirTerminatorKind,
};
use rustc_public::ty::{
    AdtDef, FnDef, FnSig, GenericArgKind, GenericArgs, RigidTy, Span as PublicSpan, Ty as MirTy,
    VariantIdx,
};

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
        let mut guard = self.0.custom_files.try_lock().unwrap();
        guard.push(CustomFile {
            id: FileId(id),
            path: path.into(),
            contents: contents.into(),
        });
        FileId(id)
    }

    pub fn take_custom_files(&self) -> Vec<CustomFile> {
        let mut guard = self.0.custom_files.try_lock().unwrap();
        std::mem::take(&mut *guard)
    }

    pub(crate) fn register_with_source_map(&self, tcx: TyCtxt<'_>) {
        let files = self.take_custom_files();
        if files.is_empty() {
            return;
        }
        let source_map = tcx.sess.source_map();
        let mut reg_guard = self.0.registered_files.try_lock().unwrap();
        for file in files {
            if reg_guard.contains_key(&file.id) {
                continue;
            }
            let filename = if file.path.exists() {
                let display_path = source_map
                    .working_dir()
                    .local_path()
                    .and_then(|working_dir| file.path.strip_prefix(working_dir).ok())
                    .unwrap_or(file.path.as_path());
                let real = source_map
                    .path_mapping()
                    .to_real_filename(source_map.working_dir(), display_path);
                rustc_span::FileName::Real(real)
            } else {
                rustc_span::FileName::Custom(file.path.display().to_string())
            };
            let source_file = source_map.new_source_file(filename, file.contents.clone());
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
        let guard = self.0.registered_files.try_lock().unwrap();
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

#[derive(Debug, Clone)]
pub struct ItemSignatureInfo {
    pub id: rustc_public::DefId,
    pub kind: ItemSignatureKind,
    pub span: PublicSpan,
}

impl ItemSignatureInfo {
    fn from_hir_structure(hir_structure: &crate::HirStructure) -> Vec<ItemSignatureInfo> {
        fn collect(module: &crate::HirModule, result: &mut Vec<ItemSignatureInfo>) {
            for item in &module.items {
                match item {
                    crate::HirModuleItem::Function {
                        name: _,
                        id,
                        sig,
                        span,
                        ..
                    } => result.push(ItemSignatureInfo {
                        id: id.0,
                        kind: ItemSignatureKind::Function(sig.clone()),
                        span: *span,
                    }),
                    crate::HirModuleItem::Adt {
                        name: _,
                        repr: _,
                        id,
                        kind,
                        span,
                        ..
                    } => match kind {
                        crate::hir_structure::HirAdtKind::Struct { fields } => {
                            result.push(ItemSignatureInfo {
                                id: id.0,
                                kind: ItemSignatureKind::Struct(fields.clone()),
                                span: *span,
                            });
                        }
                        crate::hir_structure::HirAdtKind::Union { fields } => {
                            result.push(ItemSignatureInfo {
                                id: id.0,
                                kind: ItemSignatureKind::Union(fields.clone()),
                                span: *span,
                            });
                        }
                    },
                    crate::HirModuleItem::TypeDef { id, span, ty, .. } => {
                        result.push(ItemSignatureInfo {
                            id: *id,
                            kind: ItemSignatureKind::TypeDef(ty.clone()),
                            span: *span,
                        });
                    }
                    crate::HirModuleItem::Const {
                        id, span, ty, rhs, ..
                    } => {
                        result.push(ItemSignatureInfo {
                            id: *id,
                            kind: ItemSignatureKind::Const {
                                ty: ty.clone(),
                                rhs: *rhs,
                            },
                            span: *span,
                        });
                    }
                    crate::HirModuleItem::Static {
                        id,
                        span,
                        ty,
                        mutable,
                        ..
                    } => {
                        result.push(ItemSignatureInfo {
                            id: *id,
                            kind: ItemSignatureKind::Static {
                                ty: ty.clone(),
                                mutable: *mutable,
                            },
                            span: *span,
                        });
                    }
                    crate::HirModuleItem::Impl {
                        id,
                        self_ty,
                        trait_def,
                        items,
                        span,
                    } => {
                        result.push(ItemSignatureInfo {
                            id: *id,
                            kind: ItemSignatureKind::Impl {
                                self_ty: self_ty.clone(),
                                trait_def: *trait_def,
                                items: items.clone(),
                            },
                            span: *span,
                        });
                    }
                    crate::HirModuleItem::Module {
                        id, module, span, ..
                    } => {
                        result.push(ItemSignatureInfo {
                            id: *id,
                            kind: ItemSignatureKind::Module,
                            span: *span,
                        });
                        collect(module, result);
                    }
                    crate::HirModuleItem::ForeignMod { id: _, items } => {
                        for item in items {
                            match item {
                                crate::ForeignModItem::ForeignFunction {
                                    name: _,
                                    id,
                                    sig,
                                    span,
                                } => {
                                    result.push(ItemSignatureInfo {
                                        id: id.0,
                                        kind: ItemSignatureKind::ForeignFunction(sig.clone()),
                                        span: *span,
                                    });
                                }
                                crate::ForeignModItem::ForeignType { name: _, id, span } => {
                                    result.push(ItemSignatureInfo {
                                        id: *id,
                                        kind: ItemSignatureKind::ForeignType,
                                        span: *span,
                                    });
                                }
                                crate::ForeignModItem::ForeignStatic {
                                    name: _,
                                    id,
                                    ty,
                                    mutable,
                                    span,
                                } => {
                                    result.push(ItemSignatureInfo {
                                        id: *id,
                                        kind: ItemSignatureKind::ForeignStatic {
                                            ty: ty.clone(),
                                            mutable: *mutable,
                                        },
                                        span: *span,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
        let mut result = vec![];
        collect(&hir_structure.root, &mut result);
        result
    }
}

#[derive(Debug, Clone)]
pub enum ItemSignatureKind {
    Module,
    Function(FunctionSignature),
    ForeignFunction(FunctionSignature),
    ForeignType,
    Struct(Vec<StructField>),
    Union(Vec<StructField>),
    TypeDef(HirTy),
    Const {
        ty: HirTy,
        rhs: DefId,
    },
    Static {
        ty: HirTy,
        mutable: bool,
    },
    ForeignStatic {
        ty: HirTy,
        mutable: bool,
    },
    Impl {
        self_ty: HirTy,
        trait_def: Option<DefId>,
        items: Vec<crate::hir_structure::HirImplItem>,
    },
}

#[derive(Debug, Clone)]
pub struct DefinedCrateInfo {
    pub items: Vec<DefinedItemInfo>,
    pub attrs: Vec<GeneratedAttr>,
    pub no_main: bool,
}

impl DefinedCrateInfo {
    fn owners(
        &self,
        tcx: TyCtxt<'_>,
        signatures: &[ItemSignatureInfo],
        foreign_mod_def: LocalDefId,
    ) -> IndexVec<LocalDefId, hir::MaybeOwner<'static>> {
        let mut owners: IndexVec<LocalDefId, hir::MaybeOwner<'static>> = IndexVec::new();
        let mut owner_parents: LocalDefIdMap<HirId> = LocalDefIdMap::default();

        let crate_def = CRATE_DEF_ID;
        let parent_local = |my_def_id: rustc_public::DefId| {
            self.items
                .iter()
                .find(|item| item.def_id() == my_def_id)
                .and_then(|item| item.parent)
                .and_then(|parent| my_def_id_to_rustc_def_id(tcx, parent).as_local())
                .unwrap_or(crate_def)
        };
        let is_pub_map: std::collections::HashMap<_, _> = self
            .items
            .iter()
            .map(|item| (item.def_id(), item.visibility))
            .collect();
        let vis_span = |my_def_id: rustc_public::DefId, span: RustcSpan| {
            let vis = is_pub_map
                .get(&my_def_id)
                .copied()
                .unwrap_or(Visibility::Public);
            match vis {
                Visibility::Private => DUMMY_SP,
                _ => span,
            }
        };
        let is_mod_item = |kind: DefinedItemKind| {
            matches!(
                kind,
                DefinedItemKind::Function { .. }
                    | DefinedItemKind::Struct(_, _)
                    | DefinedItemKind::Union(_, _)
                    | DefinedItemKind::TypeDef(_)
                    | DefinedItemKind::Static { .. }
                    | DefinedItemKind::Const(_)
                    | DefinedItemKind::Impl { .. }
                    | DefinedItemKind::Module(_)
                    | DefinedItemKind::ForeignMod(_)
            )
        };

        let mut foreign_item_ids = Vec::new();
        let mut foreign_items_hir = Vec::new();

        for (my_def_id, foreign, span) in signatures.iter().filter_map(|item| match &item.kind {
            ItemSignatureKind::ForeignFunction(function_signature) => {
                Some((item.id, function_signature, item.span))
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
            let mut item_allocator = HirItemAllocator::new(def_id);

            let foreign_item_id = hir::ForeignItemId {
                owner_id: OwnerId { def_id },
            };
            foreign_item_ids.push(foreign_item_id);

            let fn_sig = generate_sig(tcx, def_id, foreign, &mut item_allocator);

            let foreign_item = hir::ForeignItem {
                ident: Ident::from_str(name),
                kind: hir::ForeignItemKind::Fn(
                    fn_sig,
                    leak(
                        foreign
                            .inputs
                            .iter()
                            .map(|input| Some(input_ident(input)))
                            .collect::<Vec<_>>()
                            .into_boxed_slice(),
                    ),
                    hir::Generics::empty(),
                ),
                owner_id: OwnerId { def_id },
                span: internal(tcx, span),
                vis_span: DUMMY_SP,
                has_delayed_lints: false,
            };
            item_allocator.set_root_node(hir::Node::ForeignItem(leak(foreign_item)));
            foreign_items_hir.push((def_id, item_allocator));
        }

        let mut adt_items_hir = Vec::new();
        for (my_def_id, is_union, strukt, span) in
            signatures.iter().filter_map(|item| match &item.kind {
                ItemSignatureKind::Struct(fields) => Some((item.id, false, fields, item.span)),
                ItemSignatureKind::Union(fields) => Some((item.id, true, fields, item.span)),
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

            let mut item_allocator = HirItemAllocator::new(def_id);

            let fields_hir: &'static [hir::FieldDef<'static>] = leak(
                strukt
                    .iter()
                    .map(|field| {
                        let field_def_id = my_def_id_to_rustc_def_id(tcx, field.id).expect_local();

                        let hir_id = item_allocator.new_item();
                        let vis_span = if field.visibility == Visibility::Public {
                            internal(tcx, field.span)
                        } else {
                            DUMMY_SP
                        };
                        let hir_field_def = hir::FieldDef {
                            span: internal(tcx, field.span),
                            vis_span,
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
                            ty: leak(hir_ty_to_rustc(tcx, def_id, &field.ty, &mut item_allocator)),
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
            let kind = if is_union {
                hir::ItemKind::Union(
                    Ident::from_str(name),
                    hir::Generics::empty(),
                    hir::VariantData::Struct {
                        fields: fields_hir,
                        recovered: rustc_ast::Recovered::No,
                    },
                )
            } else {
                hir::ItemKind::Struct(
                    Ident::from_str(name),
                    hir::Generics::empty(),
                    hir::VariantData::Struct {
                        fields: fields_hir,
                        recovered: rustc_ast::Recovered::No,
                    },
                )
            };
            let item = hir::Item {
                owner_id: OwnerId { def_id },
                kind,
                span: internal(tcx, span),
                vis_span: vis_span(my_def_id, internal(tcx, span)),
                has_delayed_lints: false,
                eii: false,
            };
            item_allocator.set_root_node(hir::Node::Item(leak(item)));
            for field in fields_hir {
                item_allocator.set_node(
                    field.hir_id.local_id,
                    hir::Node::Field(field),
                    ItemLocalId::ZERO,
                );
            }
            adt_items_hir.push((def_id, item_allocator));
        }

        let mut module_items_hir = Vec::new();
        for (my_def_id, span) in signatures.iter().filter_map(|item| match item.kind {
            ItemSignatureKind::Module => Some((item.id, item.span)),
            _ => None,
        }) {
            let name = &self
                .items
                .iter()
                .find(|item| item.def_id() == my_def_id)
                .unwrap()
                .name;
            let def_id = my_def_id_to_rustc_def_id(tcx, my_def_id).expect_local();
            let span = internal(tcx, span);
            let child_item_ids = self
                .items
                .iter()
                .filter(|item| item.parent == Some(my_def_id) && is_mod_item(item.kind))
                .filter_map(|item| {
                    my_def_id_to_rustc_def_id(tcx, item.def_id())
                        .as_local()
                        .map(|def_id| hir::ItemId {
                            owner_id: OwnerId { def_id },
                        })
                })
                .collect::<Vec<_>>();
            let item = hir::Item {
                owner_id: OwnerId { def_id },
                kind: hir::ItemKind::Mod(
                    Ident::from_str(name),
                    leak(hir::Mod {
                        spans: hir::ModSpans {
                            inner_span: span,
                            inject_use_span: DUMMY_SP,
                        },
                        item_ids: leak(child_item_ids.into_boxed_slice()),
                    }),
                ),
                span,
                vis_span: vis_span(my_def_id, span),
                has_delayed_lints: false,
                eii: false,
            };
            module_items_hir.push((def_id, leak(item)));
        }

        let mut impl_items_hir = Vec::new();
        let mut impl_item_fns_hir = Vec::new();
        for (my_def_id, self_ty, trait_def, items, span) in
            signatures.iter().filter_map(|item| match &item.kind {
                ItemSignatureKind::Impl {
                    self_ty,
                    trait_def,
                    items,
                } => Some((item.id, self_ty, *trait_def, items, item.span)),
                _ => None,
            })
        {
            let def_id = my_def_id_to_rustc_def_id(tcx, my_def_id).expect_local();
            let span = internal(tcx, span);
            let mut item_allocator = HirItemAllocator::new(def_id);
            let self_ty_hir = leak(hir_ty_to_rustc(tcx, def_id, self_ty, &mut item_allocator));
            let impl_item_ids = leak(
                items
                    .iter()
                    .map(|item| hir::ImplItemId {
                        owner_id: OwnerId {
                            def_id: my_def_id_to_rustc_def_id(tcx, item.id).expect_local(),
                        },
                    })
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            );

            let of_trait = trait_def.map(|trait_def| {
                let trait_def_id = my_def_id_to_rustc_def_id(tcx, trait_def);
                let trait_path = leak(make_def_path(tcx, def_id, trait_def_id, DefKind::Trait));
                let trait_ref = hir::TraitRef {
                    path: trait_path,
                    hir_ref_id: HirId::make_owner(def_id),
                };
                leak(hir::TraitImplHeader {
                    safety: hir::Safety::Safe,
                    polarity: hir::ImplPolarity::Positive,
                    defaultness: hir::Defaultness::Final,
                    defaultness_span: None,
                    trait_ref,
                })
            });

            let impl_item = hir::Item {
                owner_id: OwnerId { def_id },
                kind: hir::ItemKind::Impl(hir::Impl {
                    generics: hir::Generics::empty(),
                    of_trait,
                    self_ty: self_ty_hir,
                    items: impl_item_ids,
                    constness: hir::Constness::NotConst,
                }),
                span,
                vis_span: vis_span(my_def_id, span),
                has_delayed_lints: false,
                eii: false,
            };
            impl_items_hir.push((def_id, leak(impl_item)));

            for item in items {
                let item_def_id = my_def_id_to_rustc_def_id(tcx, item.id).expect_local();
                let mut item_allocator = HirItemAllocator::new(item_def_id);
                let item_span = internal(tcx, item.span);

                let impl_kind = if trait_def.is_some() {
                    let trait_item_def_id = match &item.kind {
                        crate::hir_structure::HirImplItemKind::Fn {
                            trait_item_def_id, ..
                        } => trait_item_def_id
                            .map(|id| my_def_id_to_rustc_def_id(tcx, id))
                            .expect("missing trait item def id for impl item"),
                    };
                    hir::ImplItemImplKind::Trait {
                        defaultness: hir::Defaultness::Final,
                        trait_item_def_id: Ok(trait_item_def_id),
                    }
                } else {
                    hir::ImplItemImplKind::Inherent { vis_span: DUMMY_SP }
                };

                let (fn_sig, body, fn_generics) = match &item.kind {
                    crate::hir_structure::HirImplItemKind::Fn { sig, self_kind, .. } => {
                        let body_hir_id = item_allocator.new_item();
                        let fn_generics =
                            build_fn_generics(tcx, &sig.lifetimes, &mut item_allocator);
                        let fn_sig = generate_sig_with_self(
                            tcx,
                            item_def_id,
                            sig,
                            self_ty,
                            *self_kind,
                            &mut item_allocator,
                        );
                        let mut params =
                            if matches!(self_kind, crate::hir_structure::HirSelfKind::None) {
                                Vec::new()
                            } else {
                                vec![make_self_param(&mut item_allocator)]
                            };
                        params.extend(sig.inputs.iter().map(|input| {
                            make_named_param(&mut item_allocator, input.name.as_deref())
                        }));
                        if sig.c_variadic {
                            params.push(make_c_variadic_param(&mut item_allocator));
                        }
                        let loop_expr = leak(hir::Block {
                            stmts: &[],
                            expr: None,
                            hir_id: item_allocator.new_item(),
                            rules: rustc_hir::BlockCheckMode::DefaultBlock,
                            span: item_span,
                            targeted_by_break: false,
                        });
                        let body_kind = hir::ExprKind::Loop(
                            loop_expr,
                            None,
                            rustc_hir::LoopSource::Loop,
                            DUMMY_SP,
                        );
                        let body_expr = leak(hir::Expr {
                            hir_id: body_hir_id,
                            kind: body_kind,
                            span: item_span,
                        });
                        item_allocator.set_node(
                            body_expr.hir_id.local_id,
                            hir::Node::Expr(body_expr),
                            ItemLocalId::ZERO,
                        );
                        item_allocator.set_node(
                            loop_expr.hir_id.local_id,
                            hir::Node::Block(loop_expr),
                            body_expr.hir_id.local_id,
                        );
                        let body = leak(hir::Body {
                            params: leak(params.into_boxed_slice()),
                            value: body_expr,
                        });
                        for p in body.params {
                            item_allocator.set_node(
                                p.hir_id.local_id,
                                hir::Node::Param(p),
                                ItemLocalId::ZERO,
                            );
                        }
                        item_allocator.insert_body(body_expr.hir_id.local_id, body);
                        (fn_sig, body, fn_generics)
                    }
                };

                let impl_item = hir::ImplItem {
                    ident: Ident::from_str(&item.name),
                    owner_id: OwnerId {
                        def_id: item_def_id,
                    },
                    generics: fn_generics,
                    kind: hir::ImplItemKind::Fn(fn_sig, body.id()),
                    impl_kind,
                    span: item_span,
                    has_delayed_lints: false,
                };
                item_allocator.set_root_node(hir::Node::ImplItem(leak(impl_item)));
                impl_item_fns_hir.push((item_def_id, item_allocator, def_id));
            }
        }

        let mut items_hir = Vec::new();
        for (my_def_id, alias_ty, span) in signatures.iter().filter_map(|item| match &item.kind {
            ItemSignatureKind::TypeDef(ty) => Some((item.id, ty, item.span)),
            _ => None,
        }) {
            let name = &self
                .items
                .iter()
                .find(|item| item.def_id() == my_def_id)
                .unwrap()
                .name;
            let def_id = my_def_id_to_rustc_def_id(tcx, my_def_id).expect_local();
            let mut item_allocator = HirItemAllocator::new(def_id);
            let span = internal(tcx, span);
            let item = hir::Item {
                owner_id: OwnerId { def_id },
                kind: hir::ItemKind::TyAlias(
                    Ident::from_str(name),
                    hir::Generics::empty(),
                    leak(hir_ty_to_rustc(tcx, def_id, alias_ty, &mut item_allocator)),
                ),
                span,
                vis_span: vis_span(my_def_id, span),
                has_delayed_lints: false,
                eii: false,
            };
            item_allocator.set_root_node(hir::Node::Item(leak(item)));
            items_hir.push((def_id, item_allocator));
        }

        for (my_def_id, function, span) in signatures.iter().filter_map(|item| match &item.kind {
            ItemSignatureKind::Function(sig) => Some((item.id, sig, item.span)),
            _ => None,
        }) {
            let name = &self
                .items
                .iter()
                .find(|item| item.def_id() == my_def_id)
                .unwrap()
                .name;
            let def_id = my_def_id_to_rustc_def_id(tcx, my_def_id).expect_local();
            let mut item_allocator = HirItemAllocator::new(def_id);
            let body_hir_id = item_allocator.new_item();
            let span = internal(tcx, span);

            let fn_sig = generate_sig(tcx, def_id, function, &mut item_allocator);
            let loop_expr = leak(hir::Block {
                stmts: &[],
                expr: None,
                hir_id: item_allocator.new_item(),
                rules: rustc_hir::BlockCheckMode::DefaultBlock,
                span,
                targeted_by_break: false,
            });
            let body_kind =
                hir::ExprKind::Loop(loop_expr, None, rustc_hir::LoopSource::Loop, DUMMY_SP);
            let body_expr = leak(hir::Expr {
                hir_id: body_hir_id,
                kind: body_kind,
                span,
            });
            item_allocator.set_node(
                body_expr.hir_id.local_id,
                hir::Node::Expr(body_expr),
                ItemLocalId::ZERO,
            );
            item_allocator.set_node(
                loop_expr.hir_id.local_id,
                hir::Node::Block(loop_expr),
                body_expr.hir_id.local_id,
            );
            let mut params = function
                .inputs
                .iter()
                .map(|input| make_named_param(&mut item_allocator, input.name.as_deref()))
                .collect::<Vec<_>>();

            if function.c_variadic {
                params.push(make_c_variadic_param(&mut item_allocator));
            }

            let body = leak(hir::Body {
                params: leak(params.into_boxed_slice()),
                value: body_expr,
            });
            for p in body.params {
                item_allocator.set_node(p.hir_id.local_id, hir::Node::Param(p), ItemLocalId::ZERO);
            }
            item_allocator.insert_body(body_expr.hir_id.local_id, body);
            let item = hir::Item {
                owner_id: OwnerId { def_id },
                kind: hir::ItemKind::Fn {
                    sig: fn_sig,
                    ident: Ident::from_str(name),
                    generics: hir::Generics::empty(),
                    body: body.id(),
                    has_body: true,
                },
                span,
                vis_span: vis_span(my_def_id, span),
                has_delayed_lints: false,
                eii: false,
            };
            item_allocator.set_root_node(hir::Node::Item(leak(item)));
            items_hir.push((def_id, item_allocator));
        }

        for (my_def_id, static_ty, mutable, span) in
            signatures.iter().filter_map(|item| match &item.kind {
                ItemSignatureKind::Static { ty, mutable } => {
                    Some((item.id, ty, *mutable, item.span))
                }
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
            let mut item_allocator = HirItemAllocator::new(def_id);
            let body_hir_id = item_allocator.new_item();
            let span = internal(tcx, span);

            let loop_expr = leak(hir::Block {
                stmts: &[],
                expr: None,
                hir_id: item_allocator.new_item(),
                rules: rustc_hir::BlockCheckMode::DefaultBlock,
                span,
                targeted_by_break: false,
            });
            let body_kind =
                hir::ExprKind::Loop(loop_expr, None, rustc_hir::LoopSource::Loop, DUMMY_SP);
            let body_expr = leak(hir::Expr {
                hir_id: body_hir_id,
                kind: body_kind,
                span,
            });
            item_allocator.set_node(
                body_expr.hir_id.local_id,
                hir::Node::Expr(body_expr),
                ItemLocalId::ZERO,
            );
            item_allocator.set_node(
                loop_expr.hir_id.local_id,
                hir::Node::Block(loop_expr),
                body_expr.hir_id.local_id,
            );
            let body = leak(hir::Body {
                params: &[],
                value: body_expr,
            });
            item_allocator.insert_body(body_expr.hir_id.local_id, body);
            let item = hir::Item {
                owner_id: OwnerId { def_id },
                kind: hir::ItemKind::Static(
                    if mutable {
                        rustc_ast::Mutability::Mut
                    } else {
                        rustc_ast::Mutability::Not
                    },
                    Ident::from_str(name),
                    leak(hir_ty_to_rustc(tcx, def_id, static_ty, &mut item_allocator)),
                    body.id(),
                ),
                span,
                vis_span: vis_span(my_def_id, span),
                has_delayed_lints: false,
                eii: false,
            };
            item_allocator.set_root_node(hir::Node::Item(leak(item)));
            items_hir.push((def_id, item_allocator));
        }

        for (my_def_id, static_ty, mutable, span) in
            signatures.iter().filter_map(|item| match &item.kind {
                ItemSignatureKind::ForeignStatic { ty, mutable } => {
                    Some((item.id, ty, *mutable, item.span))
                }
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
            let mut item_allocator = HirItemAllocator::new(def_id);
            let span = internal(tcx, span);

            let foreign_item_id = hir::ForeignItemId {
                owner_id: OwnerId { def_id },
            };
            foreign_item_ids.push(foreign_item_id);

            let foreign_item = hir::ForeignItem {
                ident: Ident::from_str(name),
                kind: hir::ForeignItemKind::Static(
                    leak(hir_ty_to_rustc(tcx, def_id, static_ty, &mut item_allocator)),
                    if mutable {
                        rustc_ast::Mutability::Mut
                    } else {
                        rustc_ast::Mutability::Not
                    },
                    hir::Safety::Safe,
                ),
                owner_id: OwnerId { def_id },
                span,
                vis_span: span,
                has_delayed_lints: false,
            };
            item_allocator.set_root_node(hir::Node::ForeignItem(leak(foreign_item)));
            foreign_items_hir.push((def_id, item_allocator));
        }

        for (my_def_id, span) in signatures.iter().filter_map(|item| match item.kind {
            ItemSignatureKind::ForeignType => Some((item.id, item.span)),
            _ => None,
        }) {
            let name = &self
                .items
                .iter()
                .find(|item| item.def_id() == my_def_id)
                .unwrap()
                .name;
            let def_id = my_def_id_to_rustc_def_id(tcx, my_def_id).expect_local();
            let mut item_allocator = HirItemAllocator::new(def_id);

            let foreign_item_id = hir::ForeignItemId {
                owner_id: OwnerId { def_id },
            };
            foreign_item_ids.push(foreign_item_id);

            let foreign_item = hir::ForeignItem {
                ident: Ident::from_str(name),
                kind: hir::ForeignItemKind::Type,
                owner_id: OwnerId { def_id },
                span: internal(tcx, span),
                vis_span: DUMMY_SP,
                has_delayed_lints: false,
            };
            item_allocator.set_root_node(hir::Node::ForeignItem(leak(foreign_item)));
            foreign_items_hir.push((def_id, item_allocator));
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

        for (my_def_id, const_ty, rhs, span) in
            signatures.iter().filter_map(|item| match &item.kind {
                ItemSignatureKind::Const { ty, rhs } => Some((item.id, ty, *rhs, item.span)),
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
            let mut item_allocator = HirItemAllocator::new(def_id);
            let body_hir_id = item_allocator.new_item();
            let span = internal(tcx, span);

            let anon_const_def_id = my_def_id_to_rustc_def_id(tcx, rhs).expect_local();
            let anon_const_hir_id = item_allocator.new_item();

            let loop_expr = leak(hir::Block {
                stmts: &[],
                expr: None,
                hir_id: item_allocator.new_item(),
                rules: rustc_hir::BlockCheckMode::DefaultBlock,
                span,
                targeted_by_break: false,
            });
            let body_kind =
                hir::ExprKind::Loop(loop_expr, None, rustc_hir::LoopSource::Loop, DUMMY_SP);
            let body_expr = leak(hir::Expr {
                hir_id: body_hir_id,
                kind: body_kind,
                span,
            });
            item_allocator.set_node(
                body_expr.hir_id.local_id,
                hir::Node::Expr(body_expr),
                anon_const_hir_id.local_id,
            );
            item_allocator.set_node(
                loop_expr.hir_id.local_id,
                hir::Node::Block(loop_expr),
                body_expr.hir_id.local_id,
            );
            let body = leak(hir::Body {
                params: &[],
                value: body_expr,
            });
            item_allocator.insert_body(body_expr.hir_id.local_id, body);

            let anon_const = leak(hir::AnonConst {
                hir_id: anon_const_hir_id,
                def_id: anon_const_def_id,
                body: body.id(),
                span,
            });
            insert_non_owner(
                &mut owners,
                anon_const_def_id,
                hir::MaybeOwner::NonOwner(anon_const.hir_id),
            );
            let const_arg = leak(hir::ConstArg {
                hir_id: item_allocator.new_item(),
                span,
                kind: hir::ConstArgKind::Anon(anon_const),
            });
            item_allocator.set_node(
                anon_const.hir_id.local_id,
                hir::Node::AnonConst(anon_const),
                const_arg.hir_id.local_id,
            );
            item_allocator.set_node(
                const_arg.hir_id.local_id,
                hir::Node::ConstArg(const_arg),
                ItemLocalId::ZERO,
            );

            let item = hir::Item {
                owner_id: OwnerId { def_id },
                kind: hir::ItemKind::Const(
                    Ident::from_str(name),
                    hir::Generics::empty(),
                    leak(hir_ty_to_rustc(tcx, def_id, const_ty, &mut item_allocator)),
                    rustc_hir::ConstItemRhs::TypeConst(const_arg),
                ),
                span,
                vis_span: vis_span(my_def_id, span),
                has_delayed_lints: false,
                eii: false,
            };
            item_allocator.set_root_node(hir::Node::Item(leak(item)));
            items_hir.push((def_id, item_allocator));
        }

        let root_item_ids = self
            .items
            .iter()
            .filter(|item| item.parent.is_none() && is_mod_item(item.kind))
            .filter_map(|item| {
                my_def_id_to_rustc_def_id(tcx, item.def_id())
                    .as_local()
                    .map(|def_id| hir::ItemId {
                        owner_id: OwnerId { def_id },
                    })
            })
            .collect::<Vec<_>>();
        let root_mod = leak(hir::Mod {
            spans: hir::ModSpans {
                inner_span: DUMMY_SP,
                inject_use_span: DUMMY_SP,
            },
            item_ids: leak(root_item_ids.into_boxed_slice()),
        });

        let mut item_allocator = HirItemAllocator::new(crate_def);
        item_allocator.set_root_node(hir::Node::Crate(root_mod));

        let crate_owner_nodes = item_allocator.into_owner_nodes();
        let mut crate_attrs = generated_attrs(&self.attrs);
        if self.no_main {
            crate_attrs.push(hir::Attribute::Parsed(hir::attrs::AttributeKind::NoMain));
        }
        insert_owner(
            &mut owners,
            crate_def,
            leak(make_owner_info_with_attrs(
                crate_owner_nodes,
                (!crate_attrs.is_empty()).then_some(crate_attrs),
            )),
        );
        owner_parents.insert(crate_def, HirId::make_owner(crate_def));

        let foreign_mod_nodes = build_owner_nodes_for_item(foreign_mod_item);
        insert_owner(
            &mut owners,
            foreign_mod_def,
            leak(make_owner_info(foreign_mod_nodes)),
        );
        owner_parents.insert(
            foreign_mod_def,
            HirId::make_owner(parent_local(rustc_def_to_my_def(
                tcx,
                foreign_mod_def.to_def_id(),
            ))),
        );

        for (def_id, item_allocator) in foreign_items_hir {
            let foreign_nodes = item_allocator.into_owner_nodes();
            insert_owner(&mut owners, def_id, leak(make_owner_info(foreign_nodes)));
            owner_parents.insert(def_id, HirId::make_owner(foreign_mod_def));
        }

        for (def_id, item_allocator) in adt_items_hir {
            let nodes = item_allocator.into_owner_nodes();
            insert_owner(&mut owners, def_id, leak(make_owner_info(nodes)));
            owner_parents.insert(
                def_id,
                HirId::make_owner(parent_local(rustc_def_to_my_def(tcx, def_id.to_def_id()))),
            );
        }

        for (def_id, item) in module_items_hir {
            let nodes = build_owner_nodes_for_item(item);
            insert_owner(&mut owners, def_id, leak(make_owner_info(nodes)));
            owner_parents.insert(
                def_id,
                HirId::make_owner(parent_local(rustc_def_to_my_def(tcx, def_id.to_def_id()))),
            );
        }

        for (def_id, item) in impl_items_hir {
            let nodes = build_owner_nodes_for_item(item);
            insert_owner(&mut owners, def_id, leak(make_owner_info(nodes)));
            owner_parents.insert(
                def_id,
                HirId::make_owner(parent_local(rustc_def_to_my_def(tcx, def_id.to_def_id()))),
            );
        }

        for (def_id, item_allocator) in items_hir {
            let nodes = item_allocator.into_owner_nodes();
            insert_owner(&mut owners, def_id, leak(make_owner_info(nodes)));
            owner_parents.insert(
                def_id,
                HirId::make_owner(parent_local(rustc_def_to_my_def(tcx, def_id.to_def_id()))),
            );
        }

        for (def_id, item_allocator, impl_def_id) in impl_item_fns_hir {
            let nodes = item_allocator.into_owner_nodes();
            insert_owner(&mut owners, def_id, leak(make_owner_info(nodes)));
            owner_parents.insert(def_id, HirId::make_owner(impl_def_id));
        }

        if std::env::var("GEN_DEBUG").is_ok() {
            eprintln!("GeneratedCrate::build: owners len {}", owners.len());
        }

        owners
    }

    fn from_hir_structure(
        tcx: TyCtxt<'_>,
        hir_structure: &crate::HirStructure,
    ) -> (Self, LocalDefId) {
        fn collect_module(
            tcx: TyCtxt<'_>,
            module: &crate::HirModule,
            parent: Option<DefId>,
            items: &mut Vec<DefinedItemInfo>,
            the_foreign_def: &mut Option<LocalDefId>,
        ) {
            for hir_item in &module.items {
                let kind = match hir_item.clone() {
                    crate::hir_structure::HirModuleItem::Function {
                        id, sig, no_mangle, ..
                    } => DefinedItemKind::Function {
                        fn_def: id,
                        abi: sig.abi,
                        no_mangle,
                    },
                    crate::HirModuleItem::TypeDef { id, .. } => DefinedItemKind::TypeDef(id),
                    crate::HirModuleItem::Const { id, rhs, .. } => {
                        items.push(DefinedItemInfo {
                            name: String::new(),
                            kind: DefinedItemKind::AnonConst(rhs),
                            attrs: Vec::new(),
                            span: DUMMY_SP,
                            ident_span: None,
                            parent: Some(id),
                            visibility: Visibility::Public,
                        });
                        DefinedItemKind::Const(id)
                    }
                    crate::HirModuleItem::Static { id, no_mangle, .. } => DefinedItemKind::Static {
                        def_id: id,
                        no_mangle,
                    },
                    crate::HirModuleItem::Adt {
                        name: _,
                        id,
                        kind,
                        span: _,
                        repr,
                        ..
                    } => {
                        let result = match kind {
                            crate::HirAdtKind::Struct { fields: _ } => {
                                DefinedItemKind::Struct(id, repr)
                            }
                            crate::HirAdtKind::Union { fields: _ } => {
                                DefinedItemKind::Union(id, repr)
                            }
                        };
                        match kind {
                            crate::HirAdtKind::Struct { fields }
                            | crate::HirAdtKind::Union { fields } => {
                                for field in fields {
                                    items.push(DefinedItemInfo {
                                        name: field.name,
                                        kind: DefinedItemKind::Field(field.id),
                                        attrs: Vec::new(),
                                        span: internal(tcx, field.span),
                                        ident_span: None,
                                        parent: Some(id.0),
                                        visibility: field.visibility,
                                    });
                                }
                            }
                        }
                        result
                    }
                    crate::HirModuleItem::Impl {
                        id,
                        items: impl_items,
                        trait_def,
                        ..
                    } => {
                        items.push(DefinedItemInfo {
                            name: String::new(),
                            kind: DefinedItemKind::Impl {
                                def_id: id,
                                of_trait: trait_def.is_some(),
                            },
                            attrs: Vec::new(),
                            span: DUMMY_SP,
                            ident_span: None,
                            parent,
                            visibility: Visibility::Public,
                        });
                        for item in impl_items {
                            match &item.kind {
                                crate::hir_structure::HirImplItemKind::Fn { .. } => {
                                    items.push(DefinedItemInfo {
                                        name: item.name.clone(),
                                        kind: DefinedItemKind::ImplItemFn(item.id),
                                        attrs: Vec::new(),
                                        span: DUMMY_SP,
                                        ident_span: None,
                                        parent: Some(id),
                                        visibility: Visibility::Public,
                                    });
                                }
                            }
                        }
                        continue;
                    }
                    crate::HirModuleItem::Module {
                        name,
                        id,
                        module,
                        attrs,
                        span,
                        visibility,
                    } => {
                        items.push(DefinedItemInfo {
                            name,
                            kind: DefinedItemKind::Module(id),
                            attrs,
                            span: internal(tcx, span),
                            ident_span: None,
                            parent,
                            visibility,
                        });
                        collect_module(tcx, &module, Some(id), items, the_foreign_def);
                        continue;
                    }
                    crate::HirModuleItem::ForeignMod {
                        id: foreign_mod_id,
                        items: foreign_items,
                    } => {
                        *the_foreign_def =
                            Some(my_def_id_to_rustc_def_id(tcx, foreign_mod_id).expect_local());
                        items.push(DefinedItemInfo {
                            name: String::new(),
                            kind: DefinedItemKind::ForeignMod(foreign_mod_id),
                            attrs: Vec::new(),
                            span: DUMMY_SP,
                            ident_span: None,
                            parent,
                            visibility: Visibility::Public,
                        });
                        for item in foreign_items {
                            match item {
                                crate::hir_structure::ForeignModItem::ForeignFunction {
                                    name,
                                    id,
                                    sig: _,
                                    span: _,
                                } => items.push(DefinedItemInfo {
                                    name,
                                    kind: DefinedItemKind::ForeignFunction(id),
                                    attrs: Vec::new(),
                                    span: DUMMY_SP,
                                    ident_span: None,
                                    parent: Some(foreign_mod_id),
                                    visibility: Visibility::Public,
                                }),
                                crate::hir_structure::ForeignModItem::ForeignType {
                                    name,
                                    id,
                                    span: _,
                                } => items.push(DefinedItemInfo {
                                    name,
                                    kind: DefinedItemKind::ForeignType(id),
                                    attrs: Vec::new(),
                                    span: DUMMY_SP,
                                    ident_span: None,
                                    parent: Some(foreign_mod_id),
                                    visibility: Visibility::Public,
                                }),
                                crate::hir_structure::ForeignModItem::ForeignStatic {
                                    name,
                                    id,
                                    mutable: _,
                                    ty: _,
                                    span: _,
                                } => items.push(DefinedItemInfo {
                                    name,
                                    kind: DefinedItemKind::Static {
                                        def_id: id,
                                        no_mangle: false,
                                    },
                                    attrs: Vec::new(),
                                    span: DUMMY_SP,
                                    ident_span: None,
                                    parent: Some(foreign_mod_id),
                                    visibility: Visibility::Public,
                                }),
                            }
                        }
                        continue;
                    }
                };
                let visibility = hir_item.visibility();
                items.push(DefinedItemInfo {
                    name: hir_item.name().unwrap().to_owned(),
                    kind,
                    attrs: hir_item.attrs().to_vec(),
                    span: hir_item.span().map_or(DUMMY_SP, |s| internal(tcx, s)),
                    ident_span: hir_item.ident_span().map(|s| internal(tcx, s)),
                    parent,
                    visibility,
                });
            }
        }
        let mut items = vec![];
        let mut the_foreign_def = None;
        collect_module(
            tcx,
            &hir_structure.root,
            None,
            &mut items,
            &mut the_foreign_def,
        );

        (
            Self {
                items,
                attrs: hir_structure.root.attrs.clone(),
                no_main: hir_structure.no_main,
            },
            the_foreign_def.expect("missing foreign mod"),
        )
    }
}

fn generate_sig(
    tcx: TyCtxt<'_>,
    owner: LocalDefId,
    function: &FunctionSignature,
    item_allocator: &mut HirItemAllocator,
) -> rustc_hir::FnSig<'static> {
    let fn_decl = leak(hir::FnDecl {
        inputs: leak(
            function
                .inputs
                .iter()
                .map(|input| hir_ty_to_rustc(tcx, owner, &input.ty, item_allocator))
                .collect::<Vec<_>>(),
        ),
        output: hir::FnRetTy::Return(leak(hir_ty_to_rustc(
            tcx,
            owner,
            &function.output,
            item_allocator,
        ))),
        fn_decl_kind: hir::FnDeclFlags::default()
            .set_c_variadic(function.c_variadic)
            .set_implicit_self(hir::ImplicitSelfKind::None)
            .set_lifetime_elision_allowed(true),
    });

    hir::FnSig {
        header: hir::FnHeader {
            safety: if function.is_unsafe {
                hir::HeaderSafety::Normal(hir::Safety::Unsafe)
            } else {
                hir::HeaderSafety::Normal(hir::Safety::Safe)
            },
            constness: hir::Constness::NotConst,
            asyncness: hir::IsAsync::NotAsync,
            abi: match function.abi {
                FunctionAbi::Rust => ExternAbi::Rust,
                FunctionAbi::C => ExternAbi::C { unwind: false },
            },
        },
        decl: fn_decl,
        span: DUMMY_SP,
    }
}

fn generate_sig_with_self(
    tcx: TyCtxt<'_>,
    owner: LocalDefId,
    sig: &FunctionSignature,
    self_ty: &HirTy,
    self_kind: crate::hir_structure::HirSelfKind,
    item_allocator: &mut HirItemAllocator,
) -> rustc_hir::FnSig<'static> {
    let implicit_self = match self_kind {
        crate::hir_structure::HirSelfKind::None => hir::ImplicitSelfKind::None,
        crate::hir_structure::HirSelfKind::Imm => hir::ImplicitSelfKind::Imm,
        crate::hir_structure::HirSelfKind::Mut => hir::ImplicitSelfKind::Mut,
        crate::hir_structure::HirSelfKind::RefImm(_) => hir::ImplicitSelfKind::RefImm,
        crate::hir_structure::HirSelfKind::RefMut(_) => hir::ImplicitSelfKind::RefMut,
    };
    let mut inputs = Vec::with_capacity(sig.inputs.len() + 1);
    if !matches!(self_kind, crate::hir_structure::HirSelfKind::None) {
        let self_param = match self_kind {
            crate::hir_structure::HirSelfKind::RefImm(lifetime) => HirTy::new_ref(
                self_ty.clone(),
                rustc_public::mir::Mutability::Not,
                lifetime,
                self_ty.span,
            ),
            crate::hir_structure::HirSelfKind::RefMut(lifetime) => HirTy::new_ref(
                self_ty.clone(),
                rustc_public::mir::Mutability::Mut,
                lifetime,
                self_ty.span,
            ),
            crate::hir_structure::HirSelfKind::Imm | crate::hir_structure::HirSelfKind::Mut => {
                self_ty.clone()
            }
            crate::hir_structure::HirSelfKind::None => self_ty.clone(),
        };
        inputs.push(hir_ty_to_rustc(tcx, owner, &self_param, item_allocator));
    }
    inputs.extend(
        sig.inputs
            .iter()
            .map(|input| hir_ty_to_rustc(tcx, owner, &input.ty, item_allocator)),
    );
    let fn_decl = leak(hir::FnDecl {
        inputs: leak(inputs),
        output: hir::FnRetTy::Return(leak(hir_ty_to_rustc(
            tcx,
            owner,
            &sig.output,
            item_allocator,
        ))),
        fn_decl_kind: hir::FnDeclFlags::default()
            .set_c_variadic(sig.c_variadic)
            .set_implicit_self(implicit_self)
            .set_lifetime_elision_allowed(true),
    });

    let safety = if sig.is_unsafe {
        hir::Safety::Unsafe
    } else {
        hir::Safety::Safe
    };
    let abi = match sig.abi {
        FunctionAbi::Rust => ExternAbi::Rust,
        FunctionAbi::C => ExternAbi::C { unwind: false },
    };
    hir::FnSig {
        header: hir::FnHeader {
            safety: hir::HeaderSafety::Normal(safety),
            constness: hir::Constness::NotConst,
            asyncness: hir::IsAsync::NotAsync,
            abi,
        },
        decl: fn_decl,
        span: DUMMY_SP,
    }
}

impl DefinedCrateState {
    fn stage_id(&self) -> i32 {
        match self {
            DefinedCrateState::Stage0 => 0,
            DefinedCrateState::Stage1(..) => 1,
            DefinedCrateState::Stage2(..) => 2,
        }
    }

    fn hir_crate<'tcx>(
        &self,
        tcx: TyCtxt<'tcx>,
        original_owners: Option<&IndexVec<LocalDefId, hir::MaybeOwner<'static>>>,
        (): (),
    ) -> rustc_middle::hir::Crate<'tcx> {
        let DefinedCrateState::Stage2(defined_crate, signatures, foreign_def_id, ()) = self else {
            panic!("hir_crate query in stage {}", self.stage_id());
        };
        let generated_owners = defined_crate.owners(tcx, signatures, *foreign_def_id);
        let mut owners = original_owners.cloned().unwrap_or_default();
        for (def_id, owner) in generated_owners.iter_enumerated() {
            if matches!(owner, hir::MaybeOwner::Phantom) {
                continue;
            }
            if def_id.index() >= owners.len() {
                owners.resize(def_id.index() + 1, hir::MaybeOwner::Phantom);
            }
            owners[def_id] = *owner;
        }
        rustc_middle::hir::Crate::new(
            owners,
            Default::default(),
            Steal::new((
                rustc_middle::ty::ResolverAstLowering {
                    partial_res_map: Default::default(),
                    extra_lifetime_params_map: Default::default(),
                    next_node_id: rustc_ast::ast::CRATE_NODE_ID,
                    owners: Default::default(),
                    lint_buffer: Steal::new(LintBuffer::default()),
                    disambiguators: Default::default(),
                },
                Arc::new(rustc_ast::ast::Crate {
                    id: rustc_ast::ast::CRATE_NODE_ID,
                    attrs: ThinVec::new(),
                    items: ThinVec::new(),
                    spans: Default::default(),
                    is_placeholder: false,
                }),
            )),
            Some(random_fingerprint()),
        )
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

    fn contains_key(&self, tcx: TyCtxt<'_>, key: &LocalDefId) -> bool {
        match self {
            DefinedCrateState::Stage0 => false,
            DefinedCrateState::Stage1(defined_crate_info)
            | DefinedCrateState::Stage2(defined_crate_info, _, _, ()) => defined_crate_info
                .items
                .iter()
                .any(|item| my_def_id_to_rustc_def_id(tcx, item.def_id()).as_local() == Some(*key)),
        }
    }

    fn entry_fn(&self, tcx: TyCtxt<'_>, (): ()) -> Option<(RustcDefId, EntryFnType)> {
        let entry_fn = match self {
            DefinedCrateState::Stage0 => panic!("Can't eval entry_fn at stage0"),
            DefinedCrateState::Stage1(defined_crate_info)
            | DefinedCrateState::Stage2(defined_crate_info, _, _, ()) => defined_crate_info
                .items
                .iter()
                .find(|item| {
                    item.name == "main"
                        && matches!(
                            item.kind,
                            DefinedItemKind::Function {
                                abi: FunctionAbi::Rust,
                                ..
                            }
                        )
                })
                .map(DefinedItemInfo::def_id),
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

    fn def_kind(&self, tcx: TyCtxt<'_>, key: LocalDefId) -> Option<DefKind> {
        let DefinedCrateState::Stage2(items, _, _, ()) = self else {
            return None;
        };
        let key = rustc_def_to_my_def(tcx, key.to_def_id());
        let kind = items.items.iter().find(|item| item.def_id() == key)?.kind;
        Some(match kind {
            DefinedItemKind::ForeignMod(_) => DefKind::ForeignMod,
            DefinedItemKind::Module(_) => DefKind::Mod,
            DefinedItemKind::Function { .. } | DefinedItemKind::ForeignFunction(_) => DefKind::Fn,
            DefinedItemKind::ForeignType(_) => DefKind::ForeignTy,
            DefinedItemKind::Const(_) => DefKind::Const {
                is_type_const: true,
            },
            DefinedItemKind::AnonConst(_) => DefKind::AnonConst,
            DefinedItemKind::Static { .. } => DefKind::Static {
                safety: hir::Safety::Safe,
                mutability: rustc_ast::Mutability::Mut,
                nested: false,
            },
            DefinedItemKind::Struct(_, _) => DefKind::Struct,
            DefinedItemKind::Union(_, _) => DefKind::Union,
            DefinedItemKind::Field(_) => DefKind::Field,
            DefinedItemKind::TypeDef(_) => DefKind::TyAlias,
            DefinedItemKind::Impl { of_trait, .. } => DefKind::Impl { of_trait },
            DefinedItemKind::ImplItemFn(_) => DefKind::AssocFn,
        })
    }

    fn def_span(&self, tcx: TyCtxt<'_>, key: LocalDefId) -> Option<RustcSpan> {
        let DefinedCrateState::Stage2(items, _, _, ()) = self else {
            return None;
        };
        let key = rustc_def_to_my_def(tcx, key.to_def_id());
        Some(items.items.iter().find(|item| item.def_id() == key)?.span)
    }

    fn def_ident_span(&self, tcx: TyCtxt<'_>, key: LocalDefId) -> Option<RustcSpan> {
        let DefinedCrateState::Stage2(items, _, _, ()) = self else {
            return None;
        };
        let key = rustc_def_to_my_def(tcx, key.to_def_id());
        items
            .items
            .iter()
            .find(|item| item.def_id() == key)?
            .ident_span
    }

    fn advance_to_stage1(&mut self, defined_crate: DefinedCrateInfo) {
        let DefinedCrateState::Stage0 = self else {
            panic!("Moving to stage1 from stage {}", self.stage_id());
        };
        *self = DefinedCrateState::Stage1(defined_crate);
    }

    fn advance_to_stage2<S: CrateGeneratorState>(
        &mut self,
        sigs: Vec<ItemSignatureInfo>,
        foreign_mod_def: LocalDefId,
        state: S,
        context: Context,
    ) {
        _ = MIR_STATE.set(Mutex::new(MirState(Box::new(state), context)));
        let this = std::mem::replace(self, DefinedCrateState::Stage0);
        let DefinedCrateState::Stage1(defined_crate) = this else {
            panic!("Moving to stage1 from stage {}", this.stage_id());
        };
        *self = DefinedCrateState::Stage2(defined_crate, sigs, foreign_mod_def, ());
    }
}

#[derive(Debug, Clone)]
pub struct DefinedItemInfo {
    pub name: String,
    pub kind: DefinedItemKind,
    pub attrs: Vec<GeneratedAttr>,
    pub span: RustcSpan,
    pub ident_span: Option<RustcSpan>,
    pub parent: Option<DefId>,
    pub visibility: Visibility,
}

impl DefinedItemInfo {
    pub fn def_id(&self) -> rustc_public::DefId {
        match self.kind {
            DefinedItemKind::Function { fn_def, .. } | DefinedItemKind::ForeignFunction(fn_def) => {
                fn_def.0
            }
            DefinedItemKind::Struct(adt_def, _) | DefinedItemKind::Union(adt_def, _) => adt_def.0,
            DefinedItemKind::ForeignMod(def_id)
            | DefinedItemKind::ForeignType(def_id)
            | DefinedItemKind::Module(def_id)
            | DefinedItemKind::TypeDef(def_id)
            | DefinedItemKind::Static { def_id, .. }
            | DefinedItemKind::Const(def_id)
            | DefinedItemKind::AnonConst(def_id)
            | DefinedItemKind::Field(def_id)
            | DefinedItemKind::Impl { def_id, .. }
            | DefinedItemKind::ImplItemFn(def_id) => def_id,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum DefinedItemKind {
    ForeignMod(DefId),
    Module(DefId),
    Function {
        fn_def: FnDef,
        abi: FunctionAbi,
        no_mangle: bool,
    },
    ForeignFunction(FnDef),
    ForeignType(DefId),
    Const(DefId),
    AnonConst(DefId),
    Static {
        def_id: DefId,
        no_mangle: bool,
    },
    Struct(AdtDef, AdtRepr),
    Union(AdtDef, AdtRepr),
    Field(DefId),
    TypeDef(DefId),
    Impl {
        def_id: DefId,
        of_trait: bool,
    },
    ImplItemFn(DefId),
}

pub fn generate_with_args<S: CrateGeneratorState>(mut args: Vec<String>) {
    if args.len() == 1 {
        // Provide a dummy crate name if invoked programmatically without args.
        args.push(String::from("--crate-name"));
        args.push(String::from("synthetic"));
        args.push(String::from("--crate-type=bin"));
        args.push(String::from("/dev/null"));
    }
    let mut callbacks = GenerateCallbacks::<S>::new();
    rustc_driver::run_compiler(&args, &mut callbacks);
}

struct GenerateCallbacks<S: CrateGeneratorState> {
    interface: InterfaceCallbacks<S>,
    after_analysis_hook:
        Option<Box<dyn for<'tcx> FnOnce(TyCtxt<'tcx>) -> rustc_driver::Compilation + Send>>,
}

#[derive(Debug, Default)]
enum DefinedCrateState {
    #[default]
    Stage0,
    Stage1(DefinedCrateInfo),
    Stage2(DefinedCrateInfo, Vec<ItemSignatureInfo>, LocalDefId, ()),
}

#[derive(Default)]
struct GenerateState {
    defined_crate: DefinedCrateState,
    original: Option<OriginalProviders>,
    original_owners: Option<IndexVec<LocalDefId, hir::MaybeOwner<'static>>>,
    context: Option<Context>,
    prior_override_queries: Option<fn(&rustc_session::Session, &mut UtilProviders)>,
    use_generated_hir_owner_queries: bool,
}

struct GenerateGate {
    state: Mutex<GenerateState>,
}

#[derive(Copy, Clone)]
struct OriginalProviders {
    hir_crate: for<'tcx> fn(TyCtxt<'tcx>, ()) -> rustc_middle::hir::Crate<'tcx>,
    resolutions: for<'tcx> fn(TyCtxt<'tcx>, ()) -> &'tcx rustc_middle::ty::ResolverGlobalCtxt,
    effective_visibilities:
        for<'tcx> fn(
            TyCtxt<'tcx>,
            (),
        ) -> &'tcx rustc_middle::middle::privacy::EffectiveVisibilities,
    entry_fn: for<'tcx> fn(TyCtxt<'tcx>, ()) -> Option<(RustcDefId, EntryFnType)>,
    def_kind: for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> DefKind,
    // def_span: for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> RustcSpan,
    // def_ident_span: for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> Option<RustcSpan>,
    reachable_set:
        for<'tcx> fn(TyCtxt<'tcx>, ()) -> rustc_data_structures::unord::UnordSet<LocalDefId>,
    mir_built: for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> &'tcx Steal<rustc_middle::mir::Body<'tcx>>,
    mir_borrowck: for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> BorrowckProvidedValue<'tcx>,
    // impl_parent: for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> Option<RustcDefId>,
    // specialization_graph_of:
    //     for<'tcx> fn(TyCtxt<'tcx>, RustcDefId) -> Result<&'tcx Graph, ErrorGuaranteed>,
    // all_local_trait_impls:
    //     for<'tcx> fn(TyCtxt<'tcx>, ()) -> &'tcx FxIndexMap<RustcDefId, Vec<LocalDefId>>,
    // impl_trait_header: for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> ty::ImplTraitHeader<'tcx>,
    // is_copy_raw: for<'tcx> fn(TyCtxt<'tcx>, ty::PseudoCanonicalInput<'tcx, ty::Ty<'tcx>>) -> bool,
    // trait_impls_of: for<'tcx> fn(TyCtxt<'tcx>, RustcDefId) -> TraitImpls,
    // fn_sig: for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> ty::EarlyBinder<'tcx, ty::PolyFnSig<'tcx>>,
}

static GENERATE_STATE: OnceLock<Arc<GenerateGate>> = OnceLock::new();
static MIR_STATE: OnceLock<Mutex<MirState>> = OnceLock::new();

struct MirState(Box<dyn Any + Send + Sync>, Context);

unsafe impl Sync for GenerateGate {}
unsafe impl Send for GenerateGate {}

pub(crate) struct InterfaceCallbacks<S: CrateGeneratorState> {
    context: Context,
    gate: Arc<GenerateGate>,
    capture_original_owners: bool,
    phantom: PhantomData<S>,
}

fn with_generated_and_original<R>(
    _tcx: TyCtxt<'_>,
    f: impl FnOnce(&mut DefinedCrateState, OriginalProviders) -> R,
) -> R {
    let state = GENERATE_STATE
        .get()
        .cloned()
        .expect("generate state missing");
    let mut guard = state.state.try_lock().unwrap();
    let original = guard.original.expect("original providers missing");
    f(&mut guard.defined_crate, original)
}

fn with_generated_original_and_owners<R>(
    _tcx: TyCtxt<'_>,
    f: impl FnOnce(
        &mut DefinedCrateState,
        OriginalProviders,
        Option<&IndexVec<LocalDefId, hir::MaybeOwner<'static>>>,
    ) -> R,
) -> R {
    let state = GENERATE_STATE
        .get()
        .cloned()
        .expect("generate state missing");
    let mut guard = state.state.try_lock().unwrap();
    let original = guard.original.expect("original providers missing");
    let original_owners = guard.original_owners.clone();
    f(&mut guard.defined_crate, original, original_owners.as_ref())
}

pub fn root_crate_def_id(tcx: TyCtxt<'_>) -> DefId {
    rustc_def_to_my_def(tcx, CRATE_DEF_ID.to_def_id())
}

#[allow(invalid_reference_casting)]
pub fn allocate_def_id(
    tcx: TyCtxt<'_>,
    parent: rustc_public::DefId,
    kind: &crate::DefData,
) -> rustc_public::DefId {
    let defs_guard = tcx.definitions_untracked();
    let defs_mut = unsafe { &mut *(&raw const *defs_guard).cast_mut() };
    let parent = my_def_id_to_rustc_def_id(tcx, parent).expect_local();
    let data = match kind {
        crate::DefData::ForeignMod => DefPathData::ForeignMod,
        crate::DefData::Module(name) | crate::DefData::TypeNs(name) => {
            DefPathData::TypeNs(Symbol::intern(name))
        }
        crate::DefData::ValueNs(name) => DefPathData::ValueNs(Symbol::intern(name)),
        crate::DefData::LifetimeNs(name) => DefPathData::LifetimeNs(Symbol::intern(name)),
        crate::DefData::Impl => DefPathData::Impl,
        crate::DefData::AnonConst => DefPathData::AnonConst,
    };
    let def_id = DEF_DISAMBIGUATORS.with_borrow_mut(|disamb| {
        defs_mut.create_def(
            parent,
            data,
            disamb
                .entry(parent)
                .or_insert_with(|| PerParentDisambiguatorState::new(parent)),
        )
    });
    rustc_def_to_my_def(tcx, def_id.to_def_id())
}

thread_local! {
    static CACHE_TO: RefCell<HashMap<DefId, RustcDefId>> = RefCell::new(HashMap::new());
    static CACHE_FROM: RefCell<HashMap<RustcDefId, DefId>> = RefCell::new(HashMap::new());
    static DEF_DISAMBIGUATORS: RefCell<LocalDefIdMap<PerParentDisambiguatorState>> =
        RefCell::new(LocalDefIdMap::default());
}

fn my_def_id_to_rustc_def_id(tcx: TyCtxt<'_>, def_id: DefId) -> RustcDefId {
    CACHE_TO.with_borrow_mut(|ct| {
        CACHE_FROM.with_borrow_mut(|cf| {
            if let Some(r) = ct.get(&def_id) {
                return *r;
            }
            let r = rustc_public::rustc_internal::internal(tcx, def_id);
            ct.insert(def_id, r);
            cf.insert(r, def_id);
            r
        })
    })
}

fn rustc_def_to_my_def(_tcx: TyCtxt<'_>, def_id: RustcDefId) -> DefId {
    CACHE_TO.with_borrow_mut(|ct| {
        CACHE_FROM.with_borrow_mut(|cf| {
            if let Some(r) = cf.get(&def_id) {
                return *r;
            }
            let r = rustc_public::rustc_internal::stable(def_id);
            cf.insert(def_id, r);
            ct.insert(r, def_id);
            r
        })
    })
}

impl<S: CrateGeneratorState> GenerateCallbacks<S> {
    fn new() -> Self {
        Self {
            interface: InterfaceCallbacks::new(),
            after_analysis_hook: None,
        }
    }

    fn with_after_analysis(
        mut self,
        hook: Box<dyn for<'tcx> FnOnce(TyCtxt<'tcx>) -> rustc_driver::Compilation + Send>,
    ) -> Self {
        self.after_analysis_hook = Some(hook);
        self
    }
}

impl<S: CrateGeneratorState> InterfaceCallbacks<S> {
    pub(crate) fn new() -> Self {
        Self {
            context: Context::new(),
            gate: Arc::new(GenerateGate {
                state: Mutex::new(GenerateState::default()),
            }),
            capture_original_owners: true,
            phantom: PhantomData,
        }
    }

    pub(crate) fn new_without_original_owners() -> Self {
        Self {
            capture_original_owners: false,
            ..Self::new()
        }
    }

    pub(crate) fn config(&mut self, config: &mut rustc_interface::Config) {
        if std::env::var("GEN_DEBUG").is_ok() {
            eprintln!("callbacks.config");
        }
        let _ = GENERATE_STATE.set(self.gate.clone());

        config.opts.lint_opts.extend([
            ("unused".to_owned(), Level::Allow),
            ("nonstandard_style".to_owned(), Level::Allow),
            ("arithmetic_overflow".to_owned(), Level::Warn),
        ]);

        if let Some(gate) = GENERATE_STATE.get() {
            let mut guard = gate.state.try_lock().unwrap();
            guard.prior_override_queries = config.override_queries;
            guard.use_generated_hir_owner_queries = !self.capture_original_owners;
            if std::env::var("GEN_DEBUG").is_ok() {
                eprintln!("callbacks.config: storing callback");
            }
            guard.context = Some(self.context.clone());
        }

        config.override_queries = Some(override_queries::<S>);
    }

    pub(crate) fn after_crate_root_parsing(&mut self, krate: &mut rustc_ast::Crate) {
        let is_co2 = krate.attrs.iter().any(|attr| {
            let Some(meta) = attr.meta() else {
                return false;
            };
            let path_segments = meta
                .path
                .segments
                .iter()
                .map(|s| s.ident.as_str())
                .collect::<Vec<_>>();

            path_segments.as_slice() == ["language"]
                && match &meta.kind {
                    rustc_ast::MetaItemKind::List(items) => items.iter().any(|item| match item {
                        rustc_ast::MetaItemInner::MetaItem(item) => {
                            item.path.segments.iter().all(|s| s.ident.as_str() == "co2")
                        }
                        rustc_ast::MetaItemInner::Lit(_) => false,
                    }),
                    _ => false,
                }
        });

        if is_co2 {
            krate.attrs.retain(|attr| {
                let Some(meta) = attr.meta() else {
                    return true;
                };
                let path_segments = meta
                    .path
                    .segments
                    .iter()
                    .map(|s| s.ident.as_str())
                    .collect::<Vec<_>>();

                !(path_segments.as_slice() == ["language"]
                    && match &meta.kind {
                        rustc_ast::MetaItemKind::List(items) => {
                            items.iter().any(|item| match item {
                                rustc_ast::MetaItemInner::MetaItem(item) => {
                                    item.path.segments.iter().all(|s| s.ident.as_str() == "co2")
                                }
                                rustc_ast::MetaItemInner::Lit(_) => false,
                            })
                        }
                        _ => false,
                    })
            });
        }

        if S::force_no_main_attr() {
            krate.attrs.push(Attribute {
                kind: rustc_ast::AttrKind::Normal(Box::new(rustc_ast::NormalAttr {
                    item: rustc_ast::AttrItem {
                        unsafety: rustc_ast::Safety::Default,
                        path: rustc_ast::Path {
                            span: DUMMY_SP,
                            segments: [rustc_ast::PathSegment {
                                ident: Ident::from_str("no_main"),
                                id: rustc_ast::NodeId::from_usize(666_660),
                                args: None,
                            }]
                            .into(),
                            tokens: None,
                        },
                        args: rustc_ast::AttrItemKind::Unparsed(rustc_ast::AttrArgs::Empty),
                        tokens: None,
                    },
                    tokens: None,
                })),
                id: rustc_span::AttrId::from_usize(660),
                style: rustc_ast::AttrStyle::Inner,
                span: DUMMY_SP,
            });
        }

        for (idx, feature) in ["c_variadic", "extern_types"].into_iter().enumerate() {
            krate.attrs.push(Attribute {
                kind: rustc_ast::AttrKind::Normal(Box::new(rustc_ast::NormalAttr {
                    item: rustc_ast::AttrItem {
                        unsafety: rustc_ast::Safety::Default,
                        path: rustc_ast::Path {
                            span: DUMMY_SP,
                            segments: [rustc_ast::PathSegment {
                                ident: Ident::from_str("feature"),
                                id: rustc_ast::NodeId::from_usize(666_666 + idx),
                                args: None,
                            }]
                            .into(),
                            tokens: None,
                        },
                        args: rustc_ast::AttrItemKind::Unparsed(rustc_ast::AttrArgs::Delimited(
                            rustc_ast::DelimArgs {
                                dspan: DelimSpan::dummy(),
                                delim: rustc_ast::token::Delimiter::Parenthesis,
                                tokens: TokenStream::new(vec![TokenTree::Token(
                                    Token {
                                        kind: rustc_ast::token::TokenKind::Ident(
                                            Symbol::intern(feature),
                                            rustc_ast::token::IdentIsRaw::No,
                                        ),
                                        span: DUMMY_SP,
                                    },
                                    rustc_ast::tokenstream::Spacing::Alone,
                                )]),
                            },
                        )),
                        tokens: None,
                    },
                    tokens: None,
                })),
                id: rustc_span::AttrId::from_usize(666 + idx),
                style: rustc_ast::AttrStyle::Inner,
                span: DUMMY_SP,
            });
        }
    }

    pub(crate) fn after_expansion(&mut self, tcx: TyCtxt<'_>) {
        _ = rustc_public::rustc_internal::run(tcx, || {
            let gate = GENERATE_STATE.get().unwrap();
            let context = {
                let guard = gate.state.try_lock().unwrap();
                guard.context.clone().unwrap()
            };
            let original = {
                let guard = gate.state.try_lock().unwrap();
                guard.original.expect("original providers missing")
            };
            if self.capture_original_owners {
                let original_crate = (original.hir_crate)(tcx, ());
                let num_defs = tcx.definitions_untracked().num_definitions();
                let mut owners = IndexVec::with_capacity(num_defs);
                for idx in 0..num_defs {
                    owners.push(original_crate.owner(tcx, LocalDefId::new(idx)));
                }
                let original_owners = unsafe {
                    std::mem::transmute::<
                        IndexVec<LocalDefId, hir::MaybeOwner<'_>>,
                        IndexVec<LocalDefId, hir::MaybeOwner<'static>>,
                    >(owners)
                };
                let mut guard = gate.state.try_lock().unwrap();
                guard.original_owners = Some(original_owners);
            }

            let (state, hir_structure) = S::hir_structure(crate::HirStructureCtx {
                tcx,
                inner: context.clone(),
            });

            let (defined_crate, foreign_mod_def) =
                DefinedCrateInfo::from_hir_structure(tcx, &hir_structure);

            {
                let mut guard = gate.state.try_lock().unwrap();
                guard.defined_crate.advance_to_stage1(defined_crate.clone());
            };
            let sigs = ItemSignatureInfo::from_hir_structure(&hir_structure);
            {
                let mut guard = gate.state.try_lock().unwrap();
                guard.defined_crate.advance_to_stage2(
                    sigs.clone(),
                    foreign_mod_def,
                    state,
                    context.clone(),
                );
            }
            if should_patch_cached_resolutions(tcx) {
                augment_cached_generated_resolutions(tcx);
            }
            defined_crate.owners(tcx, &sigs, foreign_mod_def);
            if self.capture_original_owners {
                _ = tcx.hir_crate(());
            }
        });
    }
}

impl<S: CrateGeneratorState> rustc_driver::Callbacks for GenerateCallbacks<S> {
    fn config(&mut self, config: &mut rustc_interface::Config) {
        self.interface.config(config);
    }

    fn after_crate_root_parsing(
        &mut self,
        _compiler: &rustc_interface::interface::Compiler,
        krate: &mut rustc_ast::Crate,
    ) -> rustc_driver::Compilation {
        self.interface.after_crate_root_parsing(krate);
        rustc_driver::Compilation::Continue
    }

    fn after_expansion(
        &mut self,
        _compiler: &rustc_interface::interface::Compiler,
        tcx: TyCtxt<'_>,
    ) -> rustc_driver::Compilation {
        self.interface.after_expansion(tcx);
        rustc_driver::Compilation::Continue
    }
    fn after_analysis(
        &mut self,
        _compiler: &rustc_interface::interface::Compiler,
        tcx: TyCtxt<'_>,
    ) -> rustc_driver::Compilation {
        if let Some(hook) = self.after_analysis_hook.take() {
            hook(tcx)
        } else {
            rustc_driver::Compilation::Continue
        }
    }
}

pub fn generate_with_args_and_after_analysis<S: CrateGeneratorState>(
    mut args: Vec<String>,
    hook: Box<dyn for<'tcx> FnOnce(TyCtxt<'tcx>) -> rustc_driver::Compilation + Send>,
) {
    if args.len() == 1 {
        args.push(String::from("--crate-name"));
        args.push(String::from("synthetic"));
        args.push(String::from("--crate-type=bin"));
        args.push(String::from("/dev/null"));
    }
    let mut callbacks = GenerateCallbacks::<S>::new().with_after_analysis(hook);
    rustc_driver::run_compiler(&args, &mut callbacks);
}

pub fn dependency_roots(tcx: TyCtxt<'_>) -> Vec<(DependencyCrate, DefId)> {
    let _ = tcx.resolutions(());
    tcx.crates(())
        .iter()
        .filter_map(|&krate| {
            let name = tcx.crate_name(krate).to_string();
            if name == "co2_std" {
                return None;
            }
            let disambiguator = tcx.crate_hash(krate).to_hex();
            // The crate root DefId has index 0 in the crate's DefId space.
            let root_def_id = rustc_def_to_my_def(
                tcx,
                RustcDefId {
                    krate,
                    index: rustc_span::def_id::DefIndex::from_usize(0),
                },
            );
            Some((
                DependencyCrate {
                    name,
                    disambiguator,
                },
                root_def_id,
            ))
        })
        .collect()
}

fn child_kind_from_def_kind(kind: DefKind) -> DependencyChildKind {
    match kind {
        DefKind::Mod => DependencyChildKind::Module,
        DefKind::Fn | DefKind::AssocFn | DefKind::Ctor(_, CtorKind::Fn) => {
            DependencyChildKind::Function
        }
        DefKind::Struct => DependencyChildKind::Struct,
        DefKind::Enum => DependencyChildKind::Enum,
        DefKind::Union => DependencyChildKind::Union,
        DefKind::Trait => DependencyChildKind::Trait,
        DefKind::Const { is_type_const: _ } => DependencyChildKind::Const,
        DefKind::Static { .. } => DependencyChildKind::Static,
        DefKind::TyAlias { .. } => DependencyChildKind::TypeAlias,
        _ => DependencyChildKind::Other,
    }
}

pub fn dependency_children(tcx: TyCtxt<'_>, def_id: DefId) -> Vec<DependencyChild> {
    let rustc_def_id = my_def_id_to_rustc_def_id(tcx, def_id);
    let kind = tcx.def_kind(rustc_def_id);

    match kind {
        DefKind::Mod => {
            let mod_children = tcx.module_children(rustc_def_id);
            let mut result: Vec<DependencyChild> = mod_children
                .iter()
                .filter_map(|child| {
                    let Res::Def(_, child_def_id) = child.res else {
                        return None;
                    };
                    let child_kind = tcx.def_kind(child_def_id);
                    let dep_child_kind = child_kind_from_def_kind(child_kind);
                    if dep_child_kind == DependencyChildKind::Other {
                        return None;
                    }
                    Some(DependencyChild {
                        def_id: rustc_def_to_my_def(tcx, child_def_id),
                        name: child.ident.to_string(),
                        kind: dep_child_kind,
                        pub_vis: child.vis.is_public(),
                    })
                })
                .collect();
            // For modules with a corresponding primitive type (like "str"), also
            // include the children of the "prim_X" module if it exists.
            let module_name = tcx.item_name(rustc_def_id);
            let module_name_str = module_name.as_str();
            if !module_name_str.starts_with("prim_") {
                let prim_name = Symbol::intern(&format!("prim_{module_name_str}"));
                // Try to find the prim_X module in the parent module
                if let Some(parent) = tcx.opt_parent(rustc_def_id) {
                    for child in tcx.module_children(parent) {
                        if child.ident.name == prim_name {
                            if let Res::Def(_, prim_def_id) = child.res {
                                for subchild in tcx.module_children(prim_def_id) {
                                    if let Res::Def(_, sub_def_id) = subchild.res {
                                        let sub_kind = tcx.def_kind(sub_def_id);
                                        let dep_kind = child_kind_from_def_kind(sub_kind);
                                        if dep_kind != DependencyChildKind::Other {
                                            result.push(DependencyChild {
                                                def_id: rustc_def_to_my_def(tcx, sub_def_id),
                                                name: subchild.ident.to_string(),
                                                kind: dep_kind,
                                                pub_vis: subchild.vis.is_public(),
                                            });
                                        }
                                    }
                                }
                            }
                            break;
                        }
                    }
                }
            }
            // For modules whose name matches a primitive type (like "str"), also
            // include the incoherent inherent impl methods for that primitive type.
            let impls = dependency_incoherent_impls_for_name(tcx, module_name_str);
            for impl_fn in impls {
                result.push(DependencyChild {
                    def_id: impl_fn.def_id,
                    name: impl_fn.name,
                    kind: DependencyChildKind::Function,
                    pub_vis: true,
                });
            }
            result
        }
        DefKind::Trait | DefKind::Impl { .. } => tcx
            .associated_item_def_ids(rustc_def_id)
            .iter()
            .filter_map(|&child_id| {
                let child_kind = tcx.def_kind(child_id);
                let dep_child_kind = child_kind_from_def_kind(child_kind);
                if dep_child_kind == DependencyChildKind::Other {
                    return None;
                }
                let full = tcx.def_path_str(child_id);
                let name = full.split("::").last().unwrap_or(&full).to_string();
                Some(DependencyChild {
                    def_id: rustc_def_to_my_def(tcx, child_id),
                    name,
                    kind: dep_child_kind,
                    pub_vis: true,
                })
            })
            .collect(),
        _ => vec![],
    }
}

pub fn dependency_impls(tcx: TyCtxt<'_>, def_id: DefId) -> Vec<ImplFunction> {
    let rustc_def_id = my_def_id_to_rustc_def_id(tcx, def_id);
    let mut result: Vec<ImplFunction> = tcx
        .inherent_impls(rustc_def_id)
        .iter()
        .flat_map(|&impl_def_id| tcx.associated_item_def_ids(impl_def_id).to_vec())
        .filter(|&item_id| {
            matches!(
                tcx.def_kind(item_id),
                DefKind::Fn | DefKind::AssocFn | DefKind::Ctor(_, CtorKind::Fn)
            )
        })
        .map(|item_id| {
            let full = tcx.def_path_str(item_id);
            let name = full.split("::").last().unwrap_or(&full).to_string();
            ImplFunction {
                def_id: rustc_def_to_my_def(tcx, item_id),
                name,
            }
        })
        .collect();

    // Include enum/struct variant constructors (e.g. Option::Some, Option::None)
    // which are not part of inherent impl blocks.
    if matches!(
        tcx.def_kind(rustc_def_id),
        DefKind::Struct | DefKind::Union | DefKind::Enum
    ) {
        let adt_def = tcx.adt_def(rustc_def_id);
        for variant in adt_def.variants() {
            if let Some(ctor_def_id) = variant.ctor_def_id()
                && matches!(tcx.def_kind(ctor_def_id), DefKind::Ctor(_, CtorKind::Fn))
            {
                let full = tcx.def_path_str(ctor_def_id);
                let name = full.split("::").last().unwrap_or(&full).to_string();
                if !result.iter().any(|f| f.name == name) {
                    result.push(ImplFunction {
                        def_id: rustc_def_to_my_def(tcx, ctor_def_id),
                        name,
                    });
                }
            }
        }
    }

    result
}

fn dependency_incoherent_simplified_type(
    tcx: TyCtxt<'_>,
    receiver_ty: rustc_public::ty::Ty,
) -> Option<SimplifiedType> {
    use rustc_public::ty::TyKind;

    match receiver_ty.kind() {
        TyKind::RigidTy(rigid) => match rigid {
            RigidTy::Bool => Some(SimplifiedType::Bool),
            RigidTy::Char => Some(SimplifiedType::Char),
            RigidTy::Int(int) => Some(SimplifiedType::Int(match int {
                rustc_public::ty::IntTy::Isize => IntTy::Isize,
                rustc_public::ty::IntTy::I8 => IntTy::I8,
                rustc_public::ty::IntTy::I16 => IntTy::I16,
                rustc_public::ty::IntTy::I32 => IntTy::I32,
                rustc_public::ty::IntTy::I64 => IntTy::I64,
                rustc_public::ty::IntTy::I128 => IntTy::I128,
            })),
            RigidTy::Uint(uint) => Some(SimplifiedType::Uint(match uint {
                rustc_public::ty::UintTy::Usize => UintTy::Usize,
                rustc_public::ty::UintTy::U8 => UintTy::U8,
                rustc_public::ty::UintTy::U16 => UintTy::U16,
                rustc_public::ty::UintTy::U32 => UintTy::U32,
                rustc_public::ty::UintTy::U64 => UintTy::U64,
                rustc_public::ty::UintTy::U128 => UintTy::U128,
            })),
            RigidTy::Float(float) => Some(SimplifiedType::Float(match float {
                rustc_public::ty::FloatTy::F16 => FloatTy::F16,
                rustc_public::ty::FloatTy::F32 => FloatTy::F32,
                rustc_public::ty::FloatTy::F64 => FloatTy::F64,
                rustc_public::ty::FloatTy::F128 => FloatTy::F128,
            })),
            RigidTy::Adt(adt, _) => {
                Some(SimplifiedType::Adt(my_def_id_to_rustc_def_id(tcx, adt.0)))
            }
            RigidTy::Foreign(def_id) => Some(SimplifiedType::Foreign(my_def_id_to_rustc_def_id(
                tcx, def_id.0,
            ))),
            RigidTy::Str => Some(SimplifiedType::Str),
            RigidTy::Array(_, _) => Some(SimplifiedType::Array),
            RigidTy::Slice(_) => Some(SimplifiedType::Slice),
            RigidTy::RawPtr(_, mutability) => Some(SimplifiedType::Ptr(match mutability {
                rustc_public::mir::Mutability::Mut => AstMutability::Mut,
                rustc_public::mir::Mutability::Not => AstMutability::Not,
            })),
            _ => None,
        },
        _ => None,
    }
}

pub fn dependency_incoherent_impls(
    tcx: TyCtxt<'_>,
    receiver_ty: rustc_public::ty::Ty,
) -> Vec<ImplFunction> {
    let Some(simplified) = dependency_incoherent_simplified_type(tcx, receiver_ty) else {
        return vec![];
    };

    tcx.crates(())
        .iter()
        .flat_map(|&cnum| {
            tcx.crate_incoherent_impls((cnum, simplified))
                .iter()
                .copied()
        })
        .flat_map(|impl_def_id| tcx.associated_item_def_ids(impl_def_id).to_vec())
        .filter(|&item_id| {
            matches!(
                tcx.def_kind(item_id),
                DefKind::Fn | DefKind::AssocFn | DefKind::Ctor(_, CtorKind::Fn)
            )
        })
        .map(|item_id| {
            let full = tcx.def_path_str(item_id);
            let name = full.split("::").last().unwrap_or(&full).to_string();
            ImplFunction {
                def_id: rustc_def_to_my_def(tcx, item_id),
                name,
            }
        })
        .collect()
}

pub fn dependency_incoherent_impls_for_name(
    _tcx: TyCtxt<'_>,
    _type_name: &str,
) -> Vec<ImplFunction> {
    // We cannot use tcx.incoherent_impls() during stage 0 (Resolver feeding phase)
    // as it requires hir_crate. Instead, try to find inherent impls by scanning
    // the core crate's metadata for impl blocks on this type.
    // For now, return empty - this function is only used by dependency_children
    // which can't access incoherent_impls during stage 0.
    vec![]
}

pub fn dependency_is_trait(tcx: TyCtxt<'_>, def_id: DefId) -> bool {
    let rustc_def_id = my_def_id_to_rustc_def_id(tcx, def_id);
    matches!(tcx.def_kind(rustc_def_id), DefKind::Trait)
}

fn dependency_const_value(tcx: TyCtxt<'_>, def_id: RustcDefId) -> Option<DependencyConstValue> {
    let kind = tcx.def_kind(def_id);
    if !matches!(kind, DefKind::Const { is_type_const: _ }) {
        return None;
    }

    let value =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| tcx.const_eval_poly(def_id)))
            .ok()?
            .ok()?;
    let scalar = value.try_to_scalar()?;
    let ty = tcx.type_of(def_id).instantiate_identity();

    match ty.skip_normalization().kind() {
        ty::Bool => Some(DependencyConstValue::Bool(scalar.to_bool().discard_err()?)),
        ty::Char => Some(DependencyConstValue::Char(scalar.to_char().discard_err()?)),
        ty::Int(int_ty) => Some(match int_ty {
            IntTy::I8 => DependencyConstValue::I8(scalar.to_i8().discard_err()?),
            IntTy::I16 => DependencyConstValue::I16(scalar.to_i16().discard_err()?),
            IntTy::I32 => DependencyConstValue::I32(scalar.to_i32().discard_err()?),
            IntTy::I64 => DependencyConstValue::I64(scalar.to_i64().discard_err()?),
            IntTy::I128 => DependencyConstValue::I128(scalar.to_i128().discard_err()?),
            IntTy::Isize => {
                DependencyConstValue::Isize(scalar.to_target_isize(&tcx).discard_err()?)
            }
        }),
        ty::Uint(uint_ty) => Some(match uint_ty {
            UintTy::U8 => DependencyConstValue::U8(scalar.to_u8().discard_err()?),
            UintTy::U16 => DependencyConstValue::U16(scalar.to_u16().discard_err()?),
            UintTy::U32 => DependencyConstValue::U32(scalar.to_u32().discard_err()?),
            UintTy::U64 => DependencyConstValue::U64(scalar.to_u64().discard_err()?),
            UintTy::U128 => DependencyConstValue::U128(scalar.to_u128().discard_err()?),
            UintTy::Usize => {
                DependencyConstValue::Usize(scalar.to_target_usize(&tcx).discard_err()?)
            }
        }),
        ty::Float(float_ty) => match float_ty {
            FloatTy::F32 => Some(DependencyConstValue::F32(f32::from_bits(
                scalar.to_u32().discard_err()?,
            ))),
            FloatTy::F64 => Some(DependencyConstValue::F64(f64::from_bits(
                scalar.to_u64().discard_err()?,
            ))),
            FloatTy::F16 | FloatTy::F128 => None,
        },
        _ => None,
    }
}

pub(crate) fn dependency_const_value_for_def_id(
    tcx: TyCtxt<'_>,
    def_id: DefId,
) -> Option<DependencyConstValue> {
    dependency_const_value(tcx, my_def_id_to_rustc_def_id(tcx, def_id))
}

pub(crate) fn type_implements_trait(
    tcx: TyCtxt<'_>,
    owner: DefId,
    ty: MirTy,
    trait_def_id: DefId,
) -> bool {
    let owner = my_def_id_to_rustc_def_id(tcx, owner);
    let trait_def_id = my_def_id_to_rustc_def_id(tcx, trait_def_id);
    let ty = mir_ty_to_rustc(tcx, &ty);
    let infcx = tcx.infer_ctxt().build(ty::TypingMode::non_body_analysis());
    infcx
        .type_implements_trait(trait_def_id, [ty], tcx.param_env(owner))
        .must_apply_modulo_regions()
}

pub(crate) fn type_is_copy(tcx: TyCtxt<'_>, owner: DefId, ty: MirTy) -> bool {
    let copy_trait = rustc_def_to_my_def(tcx, tcx.require_lang_item(LangItem::Copy, DUMMY_SP));
    type_implements_trait(tcx, owner, ty, copy_trait)
}

/// Check that the trait bounds on a function's generic parameters
/// are satisfied with the given concrete generic arguments.
/// Returns `Ok(())` if all bounds are satisfied, or an error message otherwise.
pub(crate) fn check_fn_predicates(
    tcx: TyCtxt<'_>,
    fn_def_id: DefId,
    fn_generic_args: &GenericArgs,
    owner: DefId,
) -> Result<(), String> {
    let rustc_fn_def_id = my_def_id_to_rustc_def_id(tcx, fn_def_id);
    let rustc_owner = my_def_id_to_rustc_def_id(tcx, owner);
    let predicates = tcx.predicates_of(rustc_fn_def_id);
    let rustc_args = tcx.mk_args_from_iter(fn_generic_args.0.iter().map(|arg| match arg {
        GenericArgKind::Lifetime(region) => ty::GenericArg::from(mir_region_to_rustc(tcx, region)),
        GenericArgKind::Type(ty) => ty::GenericArg::from(mir_ty_to_rustc(tcx, ty)),
        GenericArgKind::Const(konst) => ty::GenericArg::from(internal(tcx, konst.clone())),
    }));

    let infcx = tcx.infer_ctxt().build(ty::TypingMode::non_body_analysis());
    let param_env = tcx.param_env(rustc_owner);

    for (clause, _span) in predicates.predicates {
        let instantiated_clause: ty::Clause<'_> = ty::EarlyBinder::bind(*clause)
            .instantiate(tcx, rustc_args)
            .skip_normalization();
        let clause_kind = instantiated_clause.kind().skip_binder();
        if let ty::ClauseKind::Trait(pred) = clause_kind {
            if pred.trait_ref.has_bound_vars() {
                continue;
            }
            // Skip bounds that involve unresolved generic params — assume they'd hold
            if pred.trait_ref.args.iter().any(|arg| {
                arg.walk().any(|inner| {
                    matches!(inner.kind(), ty::GenericArgKind::Type(ty) if matches!(ty.kind(), ty::TyKind::Param(_)))
                })
            }) {
                continue;
            }
            if !infcx
                .type_implements_trait(pred.trait_ref.def_id, pred.trait_ref.args.iter(), param_env)
                .must_apply_modulo_regions()
            {
                let trait_ref = pred.trait_ref;
                let trait_name = tcx.def_path_str(trait_ref.def_id);
                let args: Vec<String> = trait_ref
                    .args
                    .iter()
                    .map(|arg| match arg.kind() {
                        ty::GenericArgKind::Type(ty) => format!("{ty}"),
                        ty::GenericArgKind::Lifetime(lt) => format!("{lt}"),
                        ty::GenericArgKind::Const(ct) => format!("{ct}"),
                    })
                    .collect();
                return Err(format!(
                    "a trait bound is not satisfied: `{}` for types [{}]",
                    trait_name,
                    args.join(", ")
                ));
            }
        }
    }
    Ok(())
}

pub(crate) fn resolve_inherent_method(
    tcx: TyCtxt<'_>,
    owner: DefId,
    receiver_ty: MirTy,
    method: &str,
) -> Result<Option<ResolvedMethod>, String> {
    resolve_method(tcx, owner, receiver_ty, method, &[], &[])
}

pub(crate) fn resolve_method(
    tcx: TyCtxt<'_>,
    owner: DefId,
    receiver_ty: MirTy,
    method: &str,
    traits_in_scope: &[DefId],
    arg_tys: &[MirTy],
) -> Result<Option<ResolvedMethod>, String> {
    let owner = my_def_id_to_rustc_def_id(tcx, owner);
    let receiver_ty = normalize_ty_defaults_to_rustc(tcx, receiver_ty);
    if receiver_ty.walk().any(|arg| matches!(arg.kind(), ty::GenericArgKind::Type(ty) if matches!(ty.kind(), ty::TyKind::Placeholder(_) | ty::TyKind::Infer(_) | ty::TyKind::Error(_)))) {
        return Err(format!(
            "cannot resolve method `{method}` on receiver type `{receiver_ty}` with incomplete type"
        ));
    }
    let param_env = tcx.param_env(owner);
    let infcx = tcx.infer_ctxt().build(ty::TypingMode::non_body_analysis());
    let cause = ObligationCause::dummy();
    let mut matches = Vec::new();
    let owner_local = owner.as_local().unwrap_or(CRATE_DEF_ID);
    let deref_steps = autoderef_steps(tcx, &infcx, param_env, owner_local, receiver_ty);
    let trait_def_ids: Vec<_> = traits_in_scope
        .iter()
        .map(|trait_def_id| my_def_id_to_rustc_def_id(tcx, *trait_def_id))
        .collect();
    let internal_arg_tys: Vec<ty::Ty<'_>> = arg_tys
        .iter()
        .map(|ty| normalize_ty_defaults_to_rustc(tcx, *ty))
        .collect();

    for step in deref_steps {
        let mut step_inherent = Vec::new();

        for impl_def_id in inherent_impl_def_ids_for_type(tcx, step.ty) {
            let Some(item) = associated_fn_named(tcx, impl_def_id, method) else {
                continue;
            };
            for (adjusted_self_ty, adjustment) in receiver_adjustments(tcx, &step, step.ty) {
                if let Some(probe) = probe_inherent_method(
                    tcx,
                    &infcx,
                    param_env,
                    &cause,
                    impl_def_id,
                    item.def_id,
                    step.ty,
                    adjusted_self_ty,
                    adjustment,
                    Some(&internal_arg_tys),
                ) {
                    step_inherent.push(probe);
                }
            }
        }

        // Inherent methods are always checked on every step.
        step_inherent.dedup_by_key(|m| m.def_id);
        if step_inherent.len() == 1 {
            matches.push(step_inherent.pop().unwrap());
            break;
        }
        if step_inherent.len() > 1 {
            return Err(format!(
                "multiple inherent methods named `{method}` apply to receiver type {:?}",
                step.ty
            ));
        }

        // For trait methods, skip reference and raw-pointer steps — these types are
        // transparent in CO2 (autoderef includes them), so we want trait methods on
        // the inner/pointee type, not blanket impls (like Clone → ToOwned) on the
        // pointer/reference itself.
        let mut step_trait = Vec::new();
        if !matches!(
            step.ty.kind(),
            ty::TyKind::Ref(_, _, _) | ty::TyKind::RawPtr(_, _)
        ) {
            for &trait_def_id in &trait_def_ids {
                let Some(item) = associated_fn_named(tcx, trait_def_id, method) else {
                    continue;
                };
                for (adjusted_self_ty, adjustment) in receiver_adjustments(tcx, &step, step.ty) {
                    if let Some(probe) = probe_trait_method(
                        tcx,
                        &infcx,
                        param_env,
                        &cause,
                        trait_def_id,
                        item.def_id,
                        step.ty,
                        adjusted_self_ty,
                        adjustment,
                        Some(&internal_arg_tys),
                    ) {
                        step_trait.push(probe);
                    }
                }
            }
        }

        step_trait.dedup_by_key(|m| m.def_id);
        if step_trait.len() == 1 {
            matches.push(step_trait.pop().unwrap());
            break;
        }
        if step_trait.len() > 1 {
            return Err(format!(
                "multiple trait methods named `{method}` apply to receiver type {:?}",
                step.ty
            ));
        }
    }

    if matches.is_empty() {
        return Ok(None);
    }
    let matched = matches.pop().unwrap();
    {
        let sig = matched.sig.clone();
        let internal_sig: ty::FnSig<'_> = rustc_public::rustc_internal::internal(tcx, sig);
        if internal_sig.inputs_and_output.iter().any(|ty| ty.walk().any(|arg| matches!(arg.kind(), ty::GenericArgKind::Type(inner) if matches!(inner.kind(), ty::TyKind::Placeholder(_) | ty::TyKind::Infer(_) | ty::TyKind::Error(_))))) {
            return Err(format!(
                "cannot resolve method `{method}` on receiver type with incomplete type signature"
            ));
        }
    }

    Ok(Some(matched))
}

#[derive(Clone)]
struct AutoderefStep<'tcx> {
    ty: ty::Ty<'tcx>,
    autoderefs: usize,
    steps: Vec<ReceiverAdjustmentStep>,
}

fn autoderef_steps<'tcx>(
    tcx: TyCtxt<'tcx>,
    infcx: &rustc_infer::infer::InferCtxt<'tcx>,
    param_env: ty::ParamEnv<'tcx>,
    owner: LocalDefId,
    receiver_ty: ty::Ty<'tcx>,
) -> Vec<AutoderefStep<'tcx>> {
    let mut out = Vec::new();
    let mut adjustment_steps = Vec::new();
    let mut autoderef = Autoderef::new(infcx, param_env, owner, DUMMY_SP, receiver_ty)
        .include_raw_pointers()
        .silence_errors();
    while let Some((ty, autoderefs)) = autoderef.next() {
        while adjustment_steps.len() < autoderefs {
            let Some((source, kind)) = autoderef.steps().get(adjustment_steps.len()).copied()
            else {
                break;
            };
            let target = if adjustment_steps.len() + 1 == autoderefs {
                ty
            } else {
                autoderef
                    .steps()
                    .get(adjustment_steps.len() + 1)
                    .map(|(next_source, _)| *next_source)
                    .unwrap_or(ty)
            };
            adjustment_steps.push(match kind {
                AutoderefKind::Builtin => ReceiverAdjustmentStep::BuiltinDeref {
                    source: rustc_public::rustc_internal::stable(source),
                    target: rustc_public::rustc_internal::stable(target),
                },
                AutoderefKind::Overloaded => overloaded_deref_step(tcx, source, target),
            });
        }
        out.push(AutoderefStep {
            ty,
            autoderefs,
            steps: adjustment_steps.clone(),
        });
    }
    out
}

fn overloaded_deref_step<'tcx>(
    tcx: TyCtxt<'tcx>,
    source: ty::Ty<'tcx>,
    target: ty::Ty<'tcx>,
) -> ReceiverAdjustmentStep {
    let deref_trait = tcx.require_lang_item(LangItem::Deref, DUMMY_SP);
    let deref_method = tcx
        .associated_items(deref_trait)
        .in_definition_order()
        .find(|item| {
            matches!(item.kind, ty::AssocKind::Fn { .. }) && item.name().as_str() == "deref"
        })
        .expect("Deref trait must define deref")
        .def_id;
    let trait_args = tcx.mk_args(&[source.into()]);
    let method_args = ty::GenericArgs::for_item(tcx, deref_method, |param, _| {
        let index = param.index as usize;
        if index < trait_args.len() {
            trait_args[index]
        } else {
            tcx.mk_param_from_def(param)
        }
    });
    let sig = tcx.fn_sig(deref_method).instantiate(tcx, method_args);
    let sig = tcx.instantiate_bound_regions_with_erased(sig.skip_normalization());
    ReceiverAdjustmentStep::OverloadedDeref {
        source: rustc_public::rustc_internal::stable(source),
        target: rustc_public::rustc_internal::stable(target),
        target_ref: rustc_public::rustc_internal::stable(sig.output()),
        method_def_id: rustc_def_to_my_def(tcx, deref_method),
        generic_args: stable_generic_args(tcx, method_args),
        sig: rustc_public::rustc_internal::stable(sig),
    }
}

fn inherent_receiver_def_id(ty: ty::Ty<'_>) -> Option<RustcDefId> {
    match ty.kind() {
        ty::TyKind::Adt(def, _) => Some(def.did()),
        _ => None,
    }
}

fn inherent_impl_def_ids_for_type<'tcx>(tcx: TyCtxt<'tcx>, ty: ty::Ty<'tcx>) -> Vec<RustcDefId> {
    if let Some(def_id) = inherent_receiver_def_id(ty) {
        return tcx.inherent_impls(def_id).to_vec();
    }
    let Some(simplified) = simplified_type_for_type(ty) else {
        return vec![];
    };
    tcx.crates(())
        .iter()
        .flat_map(|&cnum| {
            tcx.crate_incoherent_impls((cnum, simplified))
                .iter()
                .copied()
        })
        .collect()
}

fn simplified_type_for_type(ty: ty::Ty<'_>) -> Option<SimplifiedType> {
    match ty.kind() {
        ty::TyKind::Bool => Some(SimplifiedType::Bool),
        ty::TyKind::Char => Some(SimplifiedType::Char),
        ty::TyKind::Int(int) => Some(SimplifiedType::Int(match int {
            IntTy::Isize => IntTy::Isize,
            IntTy::I8 => IntTy::I8,
            IntTy::I16 => IntTy::I16,
            IntTy::I32 => IntTy::I32,
            IntTy::I64 => IntTy::I64,
            IntTy::I128 => IntTy::I128,
        })),
        ty::TyKind::Uint(uint) => Some(SimplifiedType::Uint(match uint {
            UintTy::Usize => UintTy::Usize,
            UintTy::U8 => UintTy::U8,
            UintTy::U16 => UintTy::U16,
            UintTy::U32 => UintTy::U32,
            UintTy::U64 => UintTy::U64,
            UintTy::U128 => UintTy::U128,
        })),
        ty::TyKind::Float(float) => Some(SimplifiedType::Float(match float {
            FloatTy::F16 => FloatTy::F16,
            FloatTy::F32 => FloatTy::F32,
            FloatTy::F64 => FloatTy::F64,
            FloatTy::F128 => FloatTy::F128,
        })),
        ty::TyKind::Adt(def, _) => Some(SimplifiedType::Adt(def.did())),
        ty::TyKind::Foreign(def_id) => Some(SimplifiedType::Foreign(*def_id)),
        ty::TyKind::Str => Some(SimplifiedType::Str),
        ty::TyKind::Array(_, _) => Some(SimplifiedType::Array),
        ty::TyKind::Slice(_) => Some(SimplifiedType::Slice),
        ty::TyKind::RawPtr(_, mutability) => Some(SimplifiedType::Ptr(match mutability {
            rustc_hir::Mutability::Mut => AstMutability::Mut,
            rustc_hir::Mutability::Not => AstMutability::Not,
        })),
        _ => None,
    }
}

fn associated_fn_named(
    tcx: TyCtxt<'_>,
    impl_def_id: RustcDefId,
    method: &str,
) -> Option<ty::AssocItem> {
    tcx.associated_items(impl_def_id)
        .in_definition_order()
        .find(|item| {
            matches!(item.kind, ty::AssocKind::Fn { .. }) && item.name().as_str() == method
        })
        .copied()
}

fn receiver_adjustments<'tcx>(
    tcx: TyCtxt<'tcx>,
    step: &AutoderefStep<'tcx>,
    ty: ty::Ty<'tcx>,
) -> Vec<(ty::Ty<'tcx>, ReceiverAdjustment)> {
    let mut out = vec![(
        ty,
        ReceiverAdjustment {
            autoderefs: step.autoderefs,
            steps: step.steps.clone(),
            autoref: None,
            mut_ptr_to_const_ptr: false,
        },
    )];
    out.push((
        ty::Ty::new_ref(tcx, tcx.lifetimes.re_erased, ty, rustc_hir::Mutability::Not),
        ReceiverAdjustment {
            autoderefs: step.autoderefs,
            steps: step.steps.clone(),
            autoref: Some(rustc_public::mir::Mutability::Not),
            mut_ptr_to_const_ptr: false,
        },
    ));
    out.push((
        ty::Ty::new_ref(tcx, tcx.lifetimes.re_erased, ty, rustc_hir::Mutability::Mut),
        ReceiverAdjustment {
            autoderefs: step.autoderefs,
            steps: step.steps.clone(),
            autoref: Some(rustc_public::mir::Mutability::Mut),
            mut_ptr_to_const_ptr: false,
        },
    ));
    if let ty::TyKind::RawPtr(inner, rustc_hir::Mutability::Mut) = ty.kind() {
        out.push((
            ty::Ty::new_imm_ptr(tcx, *inner),
            ReceiverAdjustment {
                autoderefs: step.autoderefs,
                steps: step.steps.clone(),
                autoref: None,
                mut_ptr_to_const_ptr: true,
            },
        ));
    }
    out
}

fn probe_inherent_method<'tcx>(
    tcx: TyCtxt<'tcx>,
    infcx: &rustc_infer::infer::InferCtxt<'tcx>,
    param_env: ty::ParamEnv<'tcx>,
    cause: &ObligationCause<'tcx>,
    impl_def_id: RustcDefId,
    method_def_id: RustcDefId,
    step_ty: ty::Ty<'tcx>,
    adjusted_self_ty: ty::Ty<'tcx>,
    adjustment: ReceiverAdjustment,
    arg_tys: Option<&[ty::Ty<'tcx>]>,
) -> Option<ResolvedMethod> {
    infcx.probe(|_| {
        let impl_args = infcx.fresh_args_for_item(DUMMY_SP, impl_def_id);
        let impl_self_ty = tcx.type_of(impl_def_id).instantiate(tcx, impl_args);
        let _ = infcx
            .at(cause, param_env)
            .eq(
                DefineOpaqueTypes::Yes,
                impl_self_ty.skip_normalization(),
                step_ty,
            )
            .ok()?;
        let method_args = method_args_for_impl(tcx, infcx, method_def_id, impl_args);
        probe_method_sig_and_obligations(
            tcx,
            infcx,
            param_env,
            cause,
            method_def_id,
            method_args,
            adjusted_self_ty,
            adjustment,
            Some((impl_def_id, impl_args)),
            None,
            arg_tys,
        )
    })
}

fn probe_trait_method<'tcx>(
    tcx: TyCtxt<'tcx>,
    infcx: &rustc_infer::infer::InferCtxt<'tcx>,
    param_env: ty::ParamEnv<'tcx>,
    cause: &ObligationCause<'tcx>,
    trait_def_id: RustcDefId,
    method_def_id: RustcDefId,
    step_ty: ty::Ty<'tcx>,
    adjusted_self_ty: ty::Ty<'tcx>,
    adjustment: ReceiverAdjustment,
    arg_tys: Option<&[ty::Ty<'tcx>]>,
) -> Option<ResolvedMethod> {
    infcx.probe(|_| {
        let trait_args = infcx.fresh_args_for_item(DUMMY_SP, trait_def_id);
        let _ = infcx
            .at(cause, param_env)
            .eq(DefineOpaqueTypes::Yes, trait_args.type_at(0), step_ty)
            .ok()?;
        let method_args = method_args_for_trait(tcx, infcx, method_def_id, trait_args);
        probe_method_sig_and_obligations(
            tcx,
            infcx,
            param_env,
            cause,
            method_def_id,
            method_args,
            adjusted_self_ty,
            adjustment,
            None,
            Some((trait_def_id, trait_args)),
            arg_tys,
        )
    })
}

fn probe_method_sig_and_obligations<'tcx>(
    tcx: TyCtxt<'tcx>,
    infcx: &rustc_infer::infer::InferCtxt<'tcx>,
    param_env: ty::ParamEnv<'tcx>,
    cause: &ObligationCause<'tcx>,
    method_def_id: RustcDefId,
    method_args: ty::GenericArgsRef<'tcx>,
    adjusted_self_ty: ty::Ty<'tcx>,
    adjustment: ReceiverAdjustment,
    impl_info: Option<(RustcDefId, ty::GenericArgsRef<'tcx>)>,
    trait_info: Option<(RustcDefId, ty::GenericArgsRef<'tcx>)>,
    arg_tys: Option<&[ty::Ty<'tcx>]>,
) -> Option<ResolvedMethod> {
    let sig = tcx.fn_sig(method_def_id).instantiate(tcx, method_args);
    let sig = tcx.instantiate_bound_regions_with_erased(sig.skip_normalization());
    let sig = infcx.resolve_vars_if_possible(sig);
    let self_input = sig.inputs().first().copied()?;
    let _ = infcx
        .at(cause, param_env)
        .eq(DefineOpaqueTypes::Yes, self_input, adjusted_self_ty)
        .ok()?;

    if let Some(arg_tys) = arg_tys {
        let inputs = sig.inputs();
        for (i, &arg_ty) in arg_tys.iter().enumerate() {
            if let Some(&input_ty) = inputs.get(i + 1) {
                let _ = infcx
                    .at(cause, param_env)
                    .eq(DefineOpaqueTypes::Yes, input_ty, arg_ty)
                    .ok();
            }
        }
    }

    if let Some((impl_def_id, impl_args)) = impl_info {
        let impl_bounds = tcx.predicates_of(impl_def_id).instantiate(tcx, impl_args);
        for (clause, _) in impl_bounds {
            let obligation = rustc_trait_selection::traits::Obligation::new(
                tcx,
                cause.clone(),
                param_env,
                clause.skip_normalization(),
            );
            let result = infcx.evaluate_obligation(&obligation);
            if matches!(
                result,
                Ok(rustc_trait_selection::traits::EvaluationResult::EvaluatedToErr { .. })
            ) {
                return None;
            }
        }
    }
    if let Some((trait_def_id, trait_args)) = trait_info {
        let trait_ref = ty::TraitRef::new(tcx, trait_def_id, trait_args);
        let obligation = rustc_trait_selection::traits::Obligation::new(
            tcx,
            cause.clone(),
            param_env,
            ty::Binder::dummy(trait_ref),
        );
        let result = infcx.evaluate_obligation(&obligation);
        if matches!(
            result,
            Ok(rustc_trait_selection::traits::EvaluationResult::EvaluatedToErr { .. })
        ) {
            return None;
        }
    }
    let method_bounds = tcx
        .predicates_of(method_def_id)
        .instantiate(tcx, method_args);
    let sized_trait = tcx.lang_items().sized_trait();
    for (clause, _) in method_bounds {
        // Skip implicit `Sized` bounds — method-level type params are implicitly
        // Sized and we can't evaluate that in the caller's param_env.
        if let Some(sized_trait) = sized_trait {
            if let ty::ClauseKind::Trait(trait_pred) = clause.kind().skip_binder() {
                if trait_pred.def_id() == sized_trait {
                    continue;
                }
            }
        }
        // Skip all other method bounds too — they involve method-level generics
        // which can't be evaluated in the caller's param_env. They will be checked
        // at the call site after type inference from the arguments.
    }

    let resolved_args = tcx.mk_args_from_iter(
        method_args
            .iter()
            .map(|arg| infcx.resolve_vars_if_possible(arg)),
    );
    let resolved_sig = infcx.resolve_vars_if_possible(sig);
    Some(ResolvedMethod {
        def_id: rustc_def_to_my_def(tcx, method_def_id),
        generic_args: stable_generic_args(tcx, resolved_args),
        sig: rustc_public::rustc_internal::stable(resolved_sig),
        receiver_adjustment: adjustment,
    })
}

fn method_args_for_impl<'tcx>(
    tcx: TyCtxt<'tcx>,
    infcx: &rustc_infer::infer::InferCtxt<'tcx>,
    method_def_id: RustcDefId,
    impl_args: ty::GenericArgsRef<'tcx>,
) -> ty::GenericArgsRef<'tcx> {
    ty::GenericArgs::for_item(tcx, method_def_id, |param, _| {
        let index = param.index as usize;
        if index < impl_args.len() {
            return infcx.resolve_vars_if_possible(impl_args[index]);
        }
        match param.kind {
            GenericParamDefKind::Lifetime => tcx.lifetimes.re_erased.into(),
            GenericParamDefKind::Type { .. } | GenericParamDefKind::Const { .. } => {
                tcx.mk_param_from_def(param)
            }
        }
    })
}

fn method_args_for_trait<'tcx>(
    tcx: TyCtxt<'tcx>,
    infcx: &rustc_infer::infer::InferCtxt<'tcx>,
    method_def_id: RustcDefId,
    trait_args: ty::GenericArgsRef<'tcx>,
) -> ty::GenericArgsRef<'tcx> {
    ty::GenericArgs::for_item(tcx, method_def_id, |param, _| {
        let index = param.index as usize;
        if index < trait_args.len() {
            return infcx.resolve_vars_if_possible(trait_args[index]);
        }
        match param.kind {
            GenericParamDefKind::Lifetime => tcx.lifetimes.re_erased.into(),
            GenericParamDefKind::Type { .. } | GenericParamDefKind::Const { .. } => {
                tcx.mk_param_from_def(param)
            }
        }
    })
}

/// Given a function definition with partially-resolved generic args (some may still be
/// `Param` types from generic params not yet inferred), use rustc's inference context to
/// resolve them by unifying the function's input types with the provided argument types.
///
/// Returns the fully-resolved `GenericArgs` and `FnSig`.
pub(crate) fn infer_fn_args(
    tcx: TyCtxt<'_>,
    fn_def_id: DefId,
    fn_generic_args: &GenericArgs,
    arg_tys: &[MirTy],
    skip: usize,
) -> Result<(GenericArgs, FnSig), String> {
    let rustc_fn_def_id = my_def_id_to_rustc_def_id(tcx, fn_def_id);
    let param_env = tcx.param_env(rustc_fn_def_id);
    let infcx = tcx.infer_ctxt().build(ty::TypingMode::non_body_analysis());
    let cause = ObligationCause::dummy();

    // Convert current generic args to rustc, replacing Param types with fresh inference vars
    let mut infer_for_param: Vec<Option<ty::Ty<'_>>> = vec![None; fn_generic_args.0.len()];
    let rustc_args: Vec<ty::GenericArg<'_>> = fn_generic_args
        .0
        .iter()
        .enumerate()
        .map(|(idx, arg)| match arg {
            GenericArgKind::Lifetime(region) => {
                ty::GenericArg::from(mir_region_to_rustc(tcx, region))
            }
            GenericArgKind::Type(ty) => {
                let rustc_ty = mir_ty_to_rustc(tcx, ty);
                if matches!(rustc_ty.kind(), ty::TyKind::Param(_)) {
                    let var = infcx.next_ty_var(DUMMY_SP);
                    infer_for_param[idx] = Some(var);
                    ty::GenericArg::from(var)
                } else {
                    ty::GenericArg::from(rustc_ty)
                }
            }
            GenericArgKind::Const(konst) => ty::GenericArg::from(internal(tcx, konst.clone())),
        })
        .collect();
    let rustc_args = tcx.mk_args_from_iter(rustc_args.into_iter());

    // Get the fn sig with inference vars for unresolved params
    let fn_sig = tcx.fn_sig(rustc_fn_def_id).instantiate(tcx, rustc_args);
    let fn_sig = tcx.instantiate_bound_regions_with_erased(fn_sig.skip_normalization());

    // Convert arg types to rustc
    let internal_arg_tys: Vec<ty::Ty<'_>> = arg_tys
        .iter()
        .map(|ty| normalize_ty_defaults_to_rustc(tcx, *ty))
        .collect();

    // Unify remaining inputs (after skip) with argument types.
    // Failure to unify is ignored — the caller will catch unresolved params.
    let inputs = fn_sig.inputs();
    for (i, &arg_ty) in internal_arg_tys.iter().enumerate() {
        if let Some(&input_ty) = inputs.get(i + skip) {
            let _ = infcx
                .at(&cause, param_env)
                .eq(DefineOpaqueTypes::Yes, input_ty, arg_ty);
        }
    }

    // Register the function's predicates as obligations and process them
    // to resolve associated type projections generically through the trait solver.
    let ocx = ObligationCtxt::new(&infcx);
    let predicates = tcx.predicates_of(rustc_fn_def_id);
    for (clause, _span) in predicates.predicates {
        let instantiated_clause = ty::EarlyBinder::bind(*clause).instantiate(tcx, rustc_args);
        // Skip predicates that still involve unresolved Param types
        if instantiated_clause.has_param() {
            continue;
        }
        let instantiated_clause = instantiated_clause.skip_normalization();
        ocx.register_obligation(PredicateObligation {
            cause: ObligationCause::dummy(),
            param_env,
            recursion_depth: 0,
            predicate: instantiated_clause.as_predicate(),
        });
    }
    // Process — errors are ignored; check_fn_predicates catches them later.
    let _ = ocx.try_evaluate_obligations();

    let resolved_args = infcx.resolve_vars_if_possible(rustc_args);

    // For any param that was originally a Param type and is still an inference var
    // (i.e., not resolved by unification), convert it back to the original Param type.
    let final_args: Vec<ty::GenericArg<'_>> = resolved_args
        .iter()
        .enumerate()
        .map(|(idx, arg)| {
            if let Some(infer_ty) = infer_for_param[idx] {
                let resolved = infcx.resolve_vars_if_possible(infer_ty);
                if matches!(resolved.kind(), ty::TyKind::Infer(_)) {
                    // Not resolved — use the original Param type
                    let orig = &fn_generic_args.0[idx];
                    match orig {
                        GenericArgKind::Lifetime(region) => {
                            ty::GenericArg::from(mir_region_to_rustc(tcx, region))
                        }
                        GenericArgKind::Type(ty) => ty::GenericArg::from(mir_ty_to_rustc(tcx, ty)),
                        GenericArgKind::Const(konst) => {
                            ty::GenericArg::from(internal(tcx, konst.clone()))
                        }
                    }
                } else {
                    arg.into()
                }
            } else {
                arg.into()
            }
        })
        .collect();
    let final_args = tcx.mk_args_from_iter(final_args.into_iter());

    // Re-derive sig with the final args so inferences that did resolve are reflected
    let final_sig = tcx.fn_sig(rustc_fn_def_id).instantiate(tcx, final_args);
    let final_sig = tcx.instantiate_bound_regions_with_erased(final_sig.skip_normalization());
    let resolved_sig = infcx.resolve_vars_if_possible(final_sig);

    // Check if any generic params that were originally Param are still unresolved.
    // If ALL such params remain unresolved, inference failed entirely.
    let param_count = infer_for_param.iter().filter(|x| x.is_some()).count();
    if param_count > 0 {
        let unresolved_count = final_args
            .iter()
            .enumerate()
            .filter(|(idx, _)| {
                infer_for_param[*idx].is_some()
                    && matches!(
                        final_args[*idx].kind(),
                        ty::GenericArgKind::Type(ty) if matches!(ty.kind(), ty::TyKind::Param(_))
                    )
            })
            .count();
        if unresolved_count == param_count {
            return Err("failed to infer generic args.".to_string());
        }
    }

    Ok((
        stable_generic_args(tcx, final_args),
        rustc_public::rustc_internal::stable(resolved_sig),
    ))
}

fn stable_generic_args(_tcx: TyCtxt<'_>, args: ty::GenericArgsRef<'_>) -> GenericArgs {
    GenericArgs(
        args.iter()
            .map(|arg| rustc_public::rustc_internal::stable(arg.kind()))
            .collect(),
    )
}

/// For a function definition, find which generic parameters have `FnOnce` bounds
/// and return the mapping from the `FnOnce` parameter index to its output parameter index.
///
/// For example, given `fn map<U, F: FnOnce(T) -> U>(self, f: F) -> Option<U>`,
/// this returns `[(2, 1)]`, meaning param index 2 (`F`) has a `FnOnce` bound
/// whose `Output` is param index 1 (`U`).
pub(crate) fn fn_once_output_params(tcx: TyCtxt<'_>, fn_def_id: DefId) -> Vec<(u32, u32)> {
    let rustc_fn_def_id = my_def_id_to_rustc_def_id(tcx, fn_def_id);
    let fn_once_output = tcx.require_lang_item(LangItem::FnOnceOutput, DUMMY_SP);

    let predicates = tcx.predicates_of(rustc_fn_def_id);
    let mut result = Vec::new();

    for (clause, _span) in predicates.predicates {
        let clause = clause.kind().skip_binder();
        if let ty::ClauseKind::Projection(proj) = clause
            && proj.def_id() == fn_once_output
            && let ty::TyKind::Param(self_param) = proj.self_ty().kind()
            && let ty::TermKind::Ty(term_ty) = proj.term.kind()
            && let ty::TyKind::Param(term_param) = term_ty.kind()
        {
            result.push((self_param.index, term_param.index));
        }
    }

    result
}

pub(crate) fn normalize_ty_for_owner(tcx: TyCtxt<'_>, owner: DefId, ty: MirTy) -> MirTy {
    let owner = my_def_id_to_rustc_def_id(tcx, owner);
    let ty = mir_ty_to_rustc(tcx, &ty);
    let typing_env =
        ty::TypingEnv::non_body_analysis(tcx, owner).with_post_analysis_normalized(tcx);
    tcx.try_normalize_erasing_regions(typing_env, ty::Unnormalized::dummy(ty))
        .map_or_else(
            |_| rustc_public::rustc_internal::stable(ty),
            rustc_public::rustc_internal::stable,
        )
}

pub(crate) fn normalize_ty_for_owner_with_self(
    tcx: TyCtxt<'_>,
    owner: DefId,
    ty: MirTy,
    self_ty: MirTy,
) -> MirTy {
    let owner = my_def_id_to_rustc_def_id(tcx, owner);
    let ty = mir_ty_to_rustc(tcx, &ty);
    let self_ty = mir_ty_to_rustc(tcx, &self_ty);
    let ty = rustc_middle::ty::TypeFoldable::fold_with(
        ty,
        &mut rustc_middle::ty::BottomUpFolder {
            tcx,
            ty_op: |inner| match inner.kind() {
                ty::TyKind::Param(param) if param.index == 0 && param.name.as_str() == "Self" => {
                    self_ty
                }
                _ => inner,
            },
            lt_op: |lt| lt,
            ct_op: |ct| ct,
        },
    );
    let typing_env =
        ty::TypingEnv::non_body_analysis(tcx, owner).with_post_analysis_normalized(tcx);
    tcx.try_normalize_erasing_regions(typing_env, ty::Unnormalized::dummy(ty))
        .map_or_else(
            |_| rustc_public::rustc_internal::stable(ty),
            rustc_public::rustc_internal::stable,
        )
}

pub(crate) fn check_field_visibility(tcx: TyCtxt<'_>, owner: DefId, field_def_id: DefId) -> bool {
    let rustc_field = my_def_id_to_rustc_def_id(tcx, field_def_id);
    let vis = tcx.visibility(rustc_field);
    let rustc_owner = my_def_id_to_rustc_def_id(tcx, owner);
    let module = rustc_owner
        .as_local()
        .map(|local| tcx.parent_module_from_def_id(local).to_def_id())
        .unwrap_or(CRATE_DEF_ID.to_def_id());
    vis.is_accessible_from(module, tcx)
}

pub(crate) fn normalize_ty_defaults(tcx: TyCtxt<'_>, ty: MirTy) -> MirTy {
    rustc_public::rustc_internal::stable(normalize_ty_defaults_to_rustc(tcx, ty))
}

fn normalize_ty_defaults_to_rustc(tcx: TyCtxt<'_>, ty: MirTy) -> ty::Ty<'_> {
    use rustc_hir::Mutability as RustcMutability;
    use rustc_public::ty::TyKind;

    match ty.kind() {
        TyKind::RigidTy(RigidTy::Adt(adt, args)) => {
            let def_id = my_def_id_to_rustc_def_id(tcx, adt.0);
            let rustc_args = normalize_generic_args_defaults_to_rustc(tcx, adt.0, &args);
            ty::Ty::new_adt(tcx, tcx.adt_def(def_id), rustc_args)
        }
        TyKind::RigidTy(RigidTy::FnDef(def, args)) => {
            let def_id = my_def_id_to_rustc_def_id(tcx, def.0);
            let rustc_args = normalize_generic_args_defaults_to_rustc(tcx, def.0, &args);
            ty::Ty::new_fn_def(tcx, def_id, rustc_args)
        }
        TyKind::RigidTy(RigidTy::Ref(region, inner, mutability)) => ty::Ty::new_ref(
            tcx,
            mir_region_to_rustc(tcx, &region),
            normalize_ty_defaults_to_rustc(tcx, inner),
            match mutability {
                rustc_public::mir::Mutability::Mut => RustcMutability::Mut,
                rustc_public::mir::Mutability::Not => RustcMutability::Not,
            },
        ),
        TyKind::RigidTy(RigidTy::RawPtr(inner, mutability)) => match mutability {
            rustc_public::mir::Mutability::Mut => {
                ty::Ty::new_mut_ptr(tcx, normalize_ty_defaults_to_rustc(tcx, inner))
            }
            rustc_public::mir::Mutability::Not => {
                ty::Ty::new_imm_ptr(tcx, normalize_ty_defaults_to_rustc(tcx, inner))
            }
        },
        TyKind::RigidTy(RigidTy::Tuple(items)) => {
            let items = items
                .iter()
                .map(|item| normalize_ty_defaults_to_rustc(tcx, *item))
                .collect::<Vec<_>>();
            ty::Ty::new_tup(tcx, &items)
        }
        TyKind::RigidTy(RigidTy::Array(inner, len)) => {
            let rustc_len = internal(tcx, len);
            ty::Ty::new_array_with_const_len(
                tcx,
                normalize_ty_defaults_to_rustc(tcx, inner),
                rustc_len,
            )
        }
        _ => internal(tcx, ty),
    }
}

fn normalize_generic_args_defaults_to_rustc<'tcx>(
    tcx: TyCtxt<'tcx>,
    def_id: DefId,
    args: &GenericArgs,
) -> ty::GenericArgsRef<'tcx> {
    let rustc_def_id = my_def_id_to_rustc_def_id(tcx, def_id);
    let provided = tcx.mk_args_from_iter(args.0.iter().map(|arg| match arg {
        GenericArgKind::Lifetime(region) => ty::GenericArg::from(mir_region_to_rustc(tcx, region)),
        GenericArgKind::Type(ty) => ty::GenericArg::from(normalize_ty_defaults_to_rustc(tcx, *ty)),
        GenericArgKind::Const(konst) => ty::GenericArg::from(internal(tcx, konst.clone())),
    }));
    provided.extend_to(tcx, rustc_def_id, |param, current| {
        param.default_value(tcx).map_or_else(
            || tcx.mk_param_from_def(param),
            |default| default.instantiate(tcx, current).skip_normalization(),
        )
    })
}

fn override_queries<S: CrateGeneratorState>(
    sess: &rustc_session::Session,
    providers: &mut UtilProviders,
) {
    if std::env::var("GEN_DEBUG").is_ok() {
        eprintln!("override_queries");
    }
    if let Some(gate) = GENERATE_STATE.get() {
        if let Some(previous) = gate.state.try_lock().unwrap().prior_override_queries {
            previous(sess, providers);
        }
        override_providers::<S>(&mut providers.queries, gate);
    } else if std::env::var("GEN_DEBUG").is_ok() {
        eprintln!("override_queries: no state");
    }
}

fn override_providers<S: CrateGeneratorState>(providers: &mut QueryProviders, gate: &GenerateGate) {
    let mut guard = gate.state.try_lock().unwrap();
    if guard.original.is_none() {
        guard.original = Some(OriginalProviders {
            hir_crate: providers.hir_crate,
            resolutions: providers.resolutions,
            effective_visibilities: providers.effective_visibilities,
            entry_fn: providers.entry_fn,
            def_kind: providers.def_kind,
            // def_span: providers.def_span,
            // def_ident_span: providers.def_ident_span,
            reachable_set: providers.reachable_set,
            mir_built: providers.mir_built,
            mir_borrowck: providers.mir_borrowck,
            // impl_parent: providers.impl_parent,
            // specialization_graph_of: providers.specialization_graph_of,
            // all_local_trait_impls: providers.all_local_trait_impls,
            // impl_trait_header: providers.impl_trait_header,
            // is_copy_raw: providers.is_copy_raw,
            // trait_impls_of: providers.trait_impls_of,
            // fn_sig: providers.fn_sig,
        });
    }
    drop(guard);

    providers.hir_crate = generated_hir_crate;
    providers.resolutions = generated_resolutions;
    providers.effective_visibilities = generated_effective_visibilities;
    // Leave hir_crate_items/hir_module_items to the original providers.
    let use_generated_hir_owner_queries = gate
        .state
        .try_lock()
        .unwrap()
        .use_generated_hir_owner_queries;
    if use_generated_hir_owner_queries {
        providers.hir_owner = |tcx, def_id| {
            rustc_middle::hir::ProjectedMaybeOwner::new(
                tcx.hir_crate(()).owner(tcx, def_id),
                def_id,
            )
        };
    }
    providers.doc_link_resolutions = generated_doc_link_resolutions;
    providers.doc_link_traits_in_scope = generated_doc_link_traits_in_scope;
    providers.hir_attr_map = generated_hir_attr_map;
    providers.entry_fn = generated_entry_fn;
    providers.def_kind = generated_def_kind;
    providers.def_span = generated_def_span;
    providers.def_ident_span = generated_def_ident_span;
    providers.visibility = generated_visibility;
    providers.reachable_set = generated_reachable_set;
    providers.all_local_trait_impls = generated_all_local_trait_impls;
    providers.local_trait_impls = generated_local_trait_impls;
    // providers.impl_parent = generated_impl_parent;
    // providers.specialization_graph_of = generated_specialization_graph_of;
    // providers.impl_trait_header = generated_impl_trait_header;
    // providers.is_copy_raw = generated_is_copy_raw;
    // providers.trait_impls_of = generated_trait_impls_of;
    // providers.fn_sig = generated_fn_sig;
    // providers.generics_of = generated_generics_of;
    // providers.type_of = generated_type_of;
    // providers.fn_sig = generated_fn_sig;
    // providers.predicates_of = generated_predicates_of;
    // providers.explicit_predicates_of = generated_explicit_predicates_of;
    // providers.codegen_fn_attrs = generated_codegen_fn_attrs;
    providers.mir_built = generated_mir_built::<S>;
    providers.mir_borrowck = generated_mir_borrowck;
    // providers.mir_for_ctfe = generated_mir_for_ctfe;
    // providers.mir_drops_elaborated_and_const_checked =
    //     generated_mir_drops_elaborated_and_const_checked;
    // providers.optimized_mir = generated_optimized_mir;
}

fn generated_hir_crate(tcx: TyCtxt<'_>, key: ()) -> rustc_middle::hir::Crate<'_> {
    if std::env::var("GEN_DEBUG").is_ok() {
        eprintln!("generated_hir_crate");
    }
    with_generated_original_and_owners(tcx, |generated, _original, original_owners| {
        generated.hir_crate(tcx, original_owners, key)
    })
}

fn generated_resolutions(tcx: TyCtxt<'_>, (): ()) -> &rustc_middle::ty::ResolverGlobalCtxt {
    let (original, items) = {
        let state = GENERATE_STATE
            .get()
            .cloned()
            .expect("generate state missing");
        let guard = state.state.try_lock().unwrap();
        let original = guard.original.expect("original providers missing");
        let items = match &guard.defined_crate {
            DefinedCrateState::Stage2(defined_crate, _, _, ()) => defined_crate.items.clone(),
            _ => Vec::new(),
        };
        (original, items)
    };
    let mut resolutions = clone_resolver_global_ctxt((original.resolutions)(tcx, ()));
    if items.is_empty() {
        return tcx.arena.alloc(resolutions);
    }

    augment_resolutions_with_items(&mut resolutions, &items, tcx);

    tcx.arena.alloc(resolutions)
}

fn generated_doc_link_resolutions(
    tcx: TyCtxt<'_>,
    def_id: LocalDefId,
) -> &rustc_hir::def::DocLinkResMap {
    if let Some(map) = tcx.resolutions(()).doc_link_resolutions.get(&def_id) {
        return map;
    }
    panic!("no doc_link_resolutions entry for {def_id:?}");
}

fn generated_doc_link_traits_in_scope(tcx: TyCtxt<'_>, def_id: LocalDefId) -> &[RustcDefId] {
    if let Some(traits) = tcx.resolutions(()).doc_link_traits_in_scope.get(&def_id) {
        traits.as_slice()
    } else {
        &[]
    }
}

fn generated_effective_visibilities(
    tcx: TyCtxt<'_>,
    (): (),
) -> &rustc_middle::middle::privacy::EffectiveVisibilities {
    let (original, items) = {
        let state = GENERATE_STATE
            .get()
            .cloned()
            .expect("generate state missing");
        let guard = state.state.try_lock().unwrap();
        let original = guard.original.expect("original providers missing");
        let items = match &guard.defined_crate {
            DefinedCrateState::Stage2(defined_crate, _, _, ()) => defined_crate.items.clone(),
            _ => Vec::new(),
        };
        (original, items)
    };
    let mut vis = (original.effective_visibilities)(tcx, ()).clone();
    if items.is_empty() {
        return tcx.arena.alloc(vis);
    }

    augment_effective_visibilities_with_items(&mut vis, &items, tcx);

    tcx.arena.alloc(vis)
}

#[allow(invalid_reference_casting)]
fn augment_cached_generated_resolutions(tcx: TyCtxt<'_>) {
    let items = {
        let state = GENERATE_STATE
            .get()
            .cloned()
            .expect("generate state missing");
        let guard = state.state.try_lock().unwrap();
        match &guard.defined_crate {
            DefinedCrateState::Stage2(defined_crate, _, _, ()) => defined_crate.items.clone(),
            _ => Vec::new(),
        }
    };
    if items.is_empty() {
        return;
    }
    let resolutions = tcx.resolutions(());
    let resolutions = unsafe {
        &mut *std::ptr::from_ref::<rustc_middle::ty::ResolverGlobalCtxt>(resolutions).cast_mut()
    };
    augment_resolutions_with_items(resolutions, &items, tcx);
}

fn should_patch_cached_resolutions(tcx: TyCtxt<'_>) -> bool {
    tcx.crate_types()
        .iter()
        .any(|crate_type| !matches!(crate_type, CrateType::Executable))
}

fn augment_resolutions_with_items(
    resolutions: &mut rustc_middle::ty::ResolverGlobalCtxt,
    items: &[DefinedItemInfo],
    tcx: TyCtxt<'_>,
) {
    let mut module_children: HashMap<LocalDefId, Vec<rustc_middle::metadata::ModChild>> =
        HashMap::new();
    for item in items {
        let Some(local_def_id) = my_def_id_to_rustc_def_id(tcx, item.def_id()).as_local() else {
            continue;
        };
        let res = match item.kind {
            DefinedItemKind::Function { .. } | DefinedItemKind::ForeignFunction(_) => {
                Res::Def(DefKind::Fn, local_def_id.to_def_id())
            }
            DefinedItemKind::Struct(_, _) => Res::Def(DefKind::Struct, local_def_id.to_def_id()),
            DefinedItemKind::Union(_, _) => Res::Def(DefKind::Union, local_def_id.to_def_id()),
            DefinedItemKind::TypeDef(_) => Res::Def(DefKind::TyAlias, local_def_id.to_def_id()),
            DefinedItemKind::Module(_) => Res::Def(DefKind::Mod, local_def_id.to_def_id()),
            DefinedItemKind::Static { .. } => Res::Def(
                DefKind::Static {
                    safety: rustc_hir::Safety::Safe,
                    mutability: ty::Mutability::Mut,
                    nested: false,
                },
                local_def_id.to_def_id(),
            ),
            DefinedItemKind::Const(_) => Res::Def(
                DefKind::Const {
                    is_type_const: true,
                },
                local_def_id.to_def_id(),
            ),
            _ => continue,
        };
        let child = rustc_middle::metadata::ModChild {
            ident: Ident::from_str(&item.name),
            res,
            vis: ty::Visibility::Public,
            reexport_chain: SmallVec::default(),
        };
        let parent = item
            .parent
            .and_then(|parent| my_def_id_to_rustc_def_id(tcx, parent).as_local())
            .unwrap_or(CRATE_DEF_ID);
        module_children.entry(parent).or_default().push(child);
    }
    for (module, children) in module_children {
        let existing = resolutions.module_children.entry(module).or_default();
        for child in children {
            let child_def_id = child.res.def_id();
            if !existing
                .iter()
                .any(|existing_child| existing_child.res.def_id() == child_def_id)
            {
                existing.push(child);
            }
        }
    }

    // Ensure every generated module has an entry in doc_link_resolutions to
    // prevent the encoder (encoder.rs:2573) from panicking on missing def_ids.
    for item in items {
        if let DefinedItemKind::Module(_) = item.kind
            && let Some(local_def_id) = my_def_id_to_rustc_def_id(tcx, item.def_id()).as_local()
        {
            resolutions
                .doc_link_resolutions
                .entry(local_def_id)
                .or_default();
            resolutions
                .doc_link_traits_in_scope
                .entry(local_def_id)
                .or_default();
        }
    }

    // Build a parent-to-children index for resolving intra-doc links.
    let mut children_by_module: HashMap<LocalDefId, Vec<(String, LocalDefId, DefKind)>> =
        HashMap::new();
    for item in items {
        let Some(child_def_id) = my_def_id_to_rustc_def_id(tcx, item.def_id()).as_local() else {
            continue;
        };
        let def_kind = match item.kind {
            DefinedItemKind::Function { .. } | DefinedItemKind::ForeignFunction(_) => DefKind::Fn,
            DefinedItemKind::Struct(_, _) => DefKind::Struct,
            DefinedItemKind::Union(_, _) => DefKind::Union,
            DefinedItemKind::TypeDef(_) => DefKind::TyAlias,
            DefinedItemKind::Module(_) => DefKind::Mod,
            DefinedItemKind::Static { .. } => DefKind::Static {
                safety: rustc_hir::Safety::Safe,
                mutability: ty::Mutability::Mut,
                nested: false,
            },
            DefinedItemKind::Const(_) => DefKind::Const {
                is_type_const: true,
            },
            _ => continue,
        };
        let parent = item
            .parent
            .and_then(|parent| my_def_id_to_rustc_def_id(tcx, parent).as_local())
            .unwrap_or(CRATE_DEF_ID);
        children_by_module.entry(parent).or_default().push((
            item.name.clone(),
            child_def_id,
            def_kind,
        ));
    }

    // Scan generated items for doc comments containing intra-doc links and
    // populate doc_link_resolutions so that rustdoc's resolve_path does not
    // panic on missing entries (collect_intra_doc_links.rs:370).
    // For multi-segment paths (e.g. `nested::deeper::deep_value`), resolve
    // by walking the module tree through children_by_module.
    // Run BEFORE the direct-child loop below, so that the loop overwrites
    // placeholder None entries with the correct child resolution.
    for item in items {
        let parent_def_id = item
            .parent
            .and_then(|parent| my_def_id_to_rustc_def_id(tcx, parent).as_local())
            .unwrap_or(CRATE_DEF_ID);
        for attr in &item.attrs {
            if let GeneratedAttr::DocComment { comment, .. } = attr {
                for link_path in extract_intra_doc_links(comment) {
                    let segments: Vec<&str> = link_path.split("::").collect();
                    // Resolve the full multi-segment path if applicable.
                    let full_resolved = if segments.len() > 1 {
                        resolve_multi_segment_path(&children_by_module, parent_def_id, &segments)
                    } else {
                        None
                    };
                    let sym = Symbol::intern(&link_path);
                    let map = resolutions
                        .doc_link_resolutions
                        .entry(parent_def_id)
                        .or_default();
                    if let Some((kind, def_id)) = full_resolved {
                        let resolved = Some(Res::Def(kind, def_id));
                        map.insert((sym, Namespace::TypeNS), resolved);
                        map.insert((sym, Namespace::ValueNS), resolved);
                        map.insert((sym, Namespace::MacroNS), resolved);
                    } else {
                        // Insert placeholder None; the direct-child loop below
                        // will overwrite with the correct resolution for actual
                        // children.
                        map.insert((sym, Namespace::TypeNS), None);
                        map.insert((sym, Namespace::ValueNS), None);
                        map.insert((sym, Namespace::MacroNS), None);
                    }
                    // Also insert prefix paths (e.g. `nested::deeper`, `nested`)
                    // so that resolve_path doesn't panic on the associated-item
                    // fallback branch in rustdoc's resolve function.
                    if segments.len() > 1 {
                        for i in 1..segments.len() {
                            let prefix = segments[..i].join("::");
                            let p_sym = Symbol::intern(&prefix);
                            let p_children = children_by_module.entry(parent_def_id).or_default();
                            let p_type_res = p_children.iter().find_map(|(name, id, kind)| {
                                if name == &prefix
                                    && matches!(
                                        kind,
                                        DefKind::Struct
                                            | DefKind::Union
                                            | DefKind::TyAlias
                                            | DefKind::Mod
                                    )
                                {
                                    Some(Res::Def(*kind, id.to_def_id()))
                                } else {
                                    None
                                }
                            });
                            let p_value_res = p_children.iter().find_map(|(name, id, kind)| {
                                if name == &prefix
                                    && matches!(
                                        kind,
                                        DefKind::Fn
                                            | DefKind::Const { is_type_const: _ }
                                            | DefKind::Static { .. }
                                    )
                                {
                                    Some(Res::Def(*kind, id.to_def_id()))
                                } else {
                                    None
                                }
                            });
                            map.insert((p_sym, Namespace::TypeNS), p_type_res);
                            map.insert((p_sym, Namespace::ValueNS), p_value_res);
                            map.insert((p_sym, Namespace::MacroNS), None);
                        }
                    }
                }
            }
        }
    }

    // Populate doc_link_resolutions for all children in each module, so that
    // rustdoc's resolve_path can find simple child names (collect_intra_doc_links.rs:370).
    // Runs AFTER the doc-comment scanning loop above, so its entries overwrite
    // the placeholder None entries for actual children.
    for (module, children) in &children_by_module {
        let map = resolutions.doc_link_resolutions.entry(*module).or_default();
        for (name, id, kind) in children {
            let sym = Symbol::intern(name);
            if matches!(
                kind,
                DefKind::Struct | DefKind::Union | DefKind::TyAlias | DefKind::Mod
            ) {
                map.insert(
                    (sym, Namespace::TypeNS),
                    Some(Res::Def(*kind, id.to_def_id())),
                );
            }
            if matches!(
                kind,
                DefKind::Fn | DefKind::Const { is_type_const: _ } | DefKind::Static { .. }
            ) {
                map.insert(
                    (sym, Namespace::ValueNS),
                    Some(Res::Def(*kind, id.to_def_id())),
                );
            }
            map.entry((sym, Namespace::MacroNS)).or_insert(None);
        }
    }

    let mut vis = resolutions.effective_visibilities.clone();
    augment_effective_visibilities_with_items(&mut vis, items, tcx);
    resolutions.effective_visibilities = vis;
}

fn augment_effective_visibilities_with_items(
    vis: &mut rustc_middle::middle::privacy::EffectiveVisibilities,
    items: &[DefinedItemInfo],
    tcx: TyCtxt<'_>,
) {
    vis.update_root();
    for item in items {
        let Some(local_def_id) = my_def_id_to_rustc_def_id(tcx, item.def_id()).as_local() else {
            continue;
        };
        let item_vis = match item.visibility {
            Visibility::Public => ty::Visibility::<LocalDefId>::Public,
            _ => ty::Visibility::Restricted(CRATE_DEF_ID),
        };
        vis.update_eff_vis(
            local_def_id,
            &rustc_middle::middle::privacy::EffectiveVisibility::from_vis(item_vis),
            tcx,
        );
    }
}

/// Extract intra-doc link paths from a doc comment string.
/// Returns the resolved link target strings (URLs), not display text.
/// Handles `[path]`, `` [`path`] ``, and `[display](path)` patterns.
fn extract_intra_doc_links(text: &str) -> Vec<String> {
    let mut links = Vec::new();
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if bytes[i] != b'[' || (i + 1 < len && bytes[i + 1] == b'[') {
            i += 1;
            continue;
        }
        // Found a '[' not followed by another '['. Search for matching ']'.
        let open = i;
        i += 1;
        let mut depth = 1;
        while i < len && depth > 0 {
            if bytes[i] == b'[' {
                depth += 1;
            } else if bytes[i] == b']' {
                depth -= 1;
            }
            i += 1;
        }
        if depth != 0 {
            continue;
        }
        let bracket_content = &text[open + 1..i - 1];

        // Determine the link URL: if `]` is followed by `(`, use the parenthesized content.
        let url = if i < len && bytes[i] == b'(' {
            let paren_open = i;
            i += 1;
            depth = 1;
            while i < len && depth > 0 {
                if bytes[i] == b'(' {
                    depth += 1;
                } else if bytes[i] == b')' {
                    depth -= 1;
                }
                i += 1;
            }
            if depth == 0 {
                text[paren_open + 1..i - 1].trim()
            } else {
                bracket_content.trim()
            }
        } else {
            bracket_content.trim()
        };

        let link = url.trim().trim_matches('`').trim();
        if link.is_empty()
            || link.starts_with("http://")
            || link.starts_with("https://")
            || link.starts_with("ftp://")
            || link.starts_with("mailto:")
        {
            continue;
        }
        links.push(link.to_owned());
    }

    links
}

/// Walk the module tree through `children_by_module` to resolve a
/// multi-segment path starting from `start_module`.
fn resolve_multi_segment_path(
    children_by_module: &HashMap<LocalDefId, Vec<(String, LocalDefId, DefKind)>>,
    start_module: LocalDefId,
    segments: &[&str],
) -> Option<(DefKind, RustcDefId)> {
    let mut current_module = start_module;
    for (i, segment) in segments.iter().enumerate() {
        let is_last = i == segments.len() - 1;
        if let Some(children) = children_by_module.get(&current_module) {
            if let Some((_name, child_id, kind)) =
                children.iter().find(|(name, _, _)| name == segment)
            {
                if is_last {
                    return Some((*kind, child_id.to_def_id()));
                }
                if matches!(kind, DefKind::Mod) {
                    current_module = *child_id;
                } else {
                    return None;
                }
            } else {
                return None;
            }
        } else {
            return None;
        }
    }
    None
}

fn clone_resolver_global_ctxt(
    original: &rustc_middle::ty::ResolverGlobalCtxt,
) -> rustc_middle::ty::ResolverGlobalCtxt {
    rustc_middle::ty::ResolverGlobalCtxt {
        visibilities_for_hashing: original.visibilities_for_hashing.clone(),
        expn_that_defined: original.expn_that_defined.clone(),
        effective_visibilities: original.effective_visibilities.clone(),
        extern_crate_map: original.extern_crate_map.clone(),
        maybe_unused_trait_imports: original.maybe_unused_trait_imports.clone(),
        module_children: original
            .module_children
            .items()
            .map(|(def_id, children)| {
                (
                    *def_id,
                    children.iter().map(clone_mod_child).collect::<Vec<_>>(),
                )
            })
            .collect(),
        ambig_module_children: original
            .ambig_module_children
            .items()
            .map(|(def_id, children)| {
                (
                    *def_id,
                    children
                        .iter()
                        .map(|child| rustc_middle::metadata::AmbigModChild {
                            main: clone_mod_child(&child.main),
                            second: clone_mod_child(&child.second),
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .collect(),
        glob_map: original.glob_map.clone(),
        main_def: original.main_def,
        trait_impls: original.trait_impls.clone(),
        proc_macros: original.proc_macros.clone(),
        confused_type_with_std_module: original.confused_type_with_std_module.clone(),
        doc_link_resolutions: original.doc_link_resolutions.clone(),
        doc_link_traits_in_scope: original.doc_link_traits_in_scope.clone(),
        all_macro_rules: original.all_macro_rules.clone(),
        stripped_cfg_items: original.stripped_cfg_items.clone(),
        macro_reachable_adts: original.macro_reachable_adts.clone(),
        delegation_infos: original.delegation_infos.clone(),
    }
}

fn clone_mod_child(child: &rustc_middle::metadata::ModChild) -> rustc_middle::metadata::ModChild {
    rustc_middle::metadata::ModChild {
        ident: child.ident,
        res: child.res,
        vis: child.vis,
        reexport_chain: child.reexport_chain.clone(),
    }
}

fn generated_trait_impl_map(tcx: TyCtxt<'_>) -> FxIndexMap<RustcDefId, Vec<LocalDefId>> {
    let (original, generated_pairs) = {
        let state = GENERATE_STATE
            .get()
            .cloned()
            .expect("generate state missing");
        let guard = state.state.try_lock().unwrap();
        let original = guard.original.expect("original providers missing");
        let generated_pairs: Vec<(RustcDefId, LocalDefId)> = match &guard.defined_crate {
            DefinedCrateState::Stage2(_, signatures, _, ()) => signatures
                .iter()
                .filter_map(|item| match &item.kind {
                    ItemSignatureKind::Impl {
                        trait_def: Some(trait_def),
                        ..
                    } => Some((
                        my_def_id_to_rustc_def_id(tcx, *trait_def),
                        my_def_id_to_rustc_def_id(tcx, item.id).as_local()?,
                    )),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        (original, generated_pairs)
    };

    let mut trait_impls = (original.resolutions)(tcx, ()).trait_impls.clone();
    for (trait_def, impl_def) in generated_pairs {
        trait_impls.entry(trait_def).or_default().push(impl_def);
    }
    trait_impls
}

fn generated_all_local_trait_impls(
    tcx: TyCtxt<'_>,
    (): (),
) -> &FxIndexMap<RustcDefId, Vec<LocalDefId>> {
    leak(generated_trait_impl_map(tcx))
}

fn generated_local_trait_impls(tcx: TyCtxt<'_>, trait_id: RustcDefId) -> &[LocalDefId] {
    generated_all_local_trait_impls(tcx, ())
        .get(&trait_id)
        .map_or(&[], |impls| leak(impls.clone().into_boxed_slice()))
}

fn generated_hir_attr_map(tcx: TyCtxt<'_>, key: OwnerId) -> &hir::AttributeMap<'_> {
    with_generated_and_original(tcx, |generated, _original| {
        let DefinedCrateState::Stage2(items, _, _, ()) = generated else {
            return hir::AttributeMap::EMPTY;
        };
        let key = key.to_def_id();
        if key.is_crate_root() {
            let mut attrs = generated_attrs(&items.attrs);
            if items.no_main {
                attrs.push(hir::Attribute::Parsed(hir::attrs::AttributeKind::NoMain));
            }
            if !attrs.is_empty() {
                return leak(hir::AttributeMap {
                    map: [(
                        ItemLocalId::ZERO,
                        leak(attrs.into_boxed_slice()) as &[hir::Attribute],
                    )]
                    .into_iter()
                    .collect(),
                    define_opaque: None,
                    opt_hash: Some(random_fingerprint()),
                });
            }
            return hir::AttributeMap::EMPTY;
        }
        let key = rustc_def_to_my_def(tcx, key);
        let Some(info) = items.items.iter().find(|item| item.def_id() == key) else {
            return hir::AttributeMap::EMPTY;
        };
        let attrs = generated_item_attrs(info);
        if attrs.is_empty() {
            return hir::AttributeMap::EMPTY;
        }
        leak(hir::AttributeMap {
            map: [(
                ItemLocalId::ZERO,
                leak(attrs.into_boxed_slice()) as &[hir::Attribute],
            )]
            .into_iter()
            .collect(),
            define_opaque: None,
            opt_hash: Some(random_fingerprint()),
        })
    })
}

fn generated_item_attrs(info: &DefinedItemInfo) -> Vec<hir::Attribute> {
    let mut attrs = generated_attrs(&info.attrs);
    let Some(attr) = generated_builtin_item_attr(info.kind) else {
        return attrs;
    };
    attrs.push(hir::Attribute::Parsed(attr));
    attrs
}

fn generated_builtin_item_attr(kind: DefinedItemKind) -> Option<hir::attrs::AttributeKind> {
    let attr = match kind {
        DefinedItemKind::Function {
            no_mangle: true, ..
        }
        | DefinedItemKind::Static {
            no_mangle: true, ..
        } => hir::attrs::AttributeKind::NoMangle(DUMMY_SP),
        DefinedItemKind::Struct(_, repr) | DefinedItemKind::Union(_, repr) => {
            let reprs = match repr {
                AdtRepr::Rust => ThinVec::from_iter([(hir::attrs::ReprAttr::ReprRust, DUMMY_SP)]),
                AdtRepr::C => ThinVec::from_iter([(hir::attrs::ReprAttr::ReprC, DUMMY_SP)]),
                AdtRepr::CPacked(n) => ThinVec::from_iter([
                    (hir::attrs::ReprAttr::ReprC, DUMMY_SP),
                    (
                        hir::attrs::ReprAttr::ReprPacked(
                            rustc_abi::Align::from_bytes(u64::from(n))
                                .expect("invalid pack alignment"),
                        ),
                        DUMMY_SP,
                    ),
                ]),
            };
            hir::attrs::AttributeKind::Repr {
                reprs,
                first_span: DUMMY_SP,
            }
        }
        _ => return None,
    };
    Some(attr)
}

fn generated_attrs(attrs: &[GeneratedAttr]) -> Vec<hir::Attribute> {
    attrs.iter().filter_map(generated_attr).collect()
}

fn generated_attr(attr: &GeneratedAttr) -> Option<hir::Attribute> {
    let kind = match attr {
        GeneratedAttr::DocComment { comment, inner } => hir::attrs::AttributeKind::DocComment {
            style: if *inner {
                rustc_ast::AttrStyle::Inner
            } else {
                rustc_ast::AttrStyle::Outer
            },
            kind: DocFragmentKind::Sugared(CommentKind::Line),
            span: DUMMY_SP,
            comment: Symbol::intern(comment),
        },
        GeneratedAttr::Word { path } if path.as_slice() == ["no_main"] => {
            hir::attrs::AttributeKind::NoMain
        }
        GeneratedAttr::InlineHint(hint) => {
            let inline_attr = match hint {
                InlineHint::Hint => hir::attrs::InlineAttr::Hint,
                InlineHint::Always => hir::attrs::InlineAttr::Always,
                InlineHint::Never => hir::attrs::InlineAttr::Never,
            };
            hir::attrs::AttributeKind::Inline(inline_attr, DUMMY_SP)
        }
        GeneratedAttr::Word { .. } => return None,
    };
    Some(hir::Attribute::Parsed(kind))
}

fn generated_entry_fn(tcx: TyCtxt<'_>, key: ()) -> Option<(RustcDefId, EntryFnType)> {
    let (generated_entry, no_main, original) = {
        let state = GENERATE_STATE
            .get()
            .cloned()
            .expect("generate state missing");
        let guard = state.state.try_lock().unwrap();
        let original = guard.original.expect("original providers missing");
        let no_main = matches!(
            &guard.defined_crate,
            DefinedCrateState::Stage1(items) | DefinedCrateState::Stage2(items, _, _, ())
                if items.no_main
        );
        (guard.defined_crate.entry_fn(tcx, key), no_main, original)
    };
    if no_main {
        return None;
    }
    generated_entry.or_else(|| (original.entry_fn)(tcx, key))
}

fn generated_def_kind(tcx: TyCtxt<'_>, key: LocalDefId) -> DefKind {
    if std::env::var("GEN_DEBUG").is_ok() {
        eprintln!("generated_def_kind {key:?}");
    }
    with_generated_and_original(tcx, |generated, original| {
        if let Some(kind) = generated.def_kind(tcx, key) {
            return kind;
        }
        // Lifetime params allocated inside generated items won't be in DefinedItemInfo,
        // but their DefKey will have LifetimeNs disambiguator.
        if matches!(
            tcx.def_key(key).disambiguated_data.data,
            DefPathData::LifetimeNs(_)
        ) {
            return DefKind::LifetimeParam;
        }
        (original.def_kind)(tcx, key)
    })
}

fn generated_def_span(tcx: TyCtxt<'_>, key: LocalDefId) -> RustcSpan {
    with_generated_and_original(tcx, |generated, _original| {
        if let Some(span) = generated.def_span(tcx, key) {
            return span;
        }

        DUMMY_SP
        // (original.def_span)(tcx, key)
    })
}

fn generated_def_ident_span(tcx: TyCtxt<'_>, key: LocalDefId) -> Option<RustcSpan> {
    with_generated_and_original(tcx, |generated, _original| {
        if let Some(span) = generated.def_ident_span(tcx, key) {
            return Some(span);
        }

        generated.def_span(tcx, key)
    })
}

fn generated_visibility(tcx: TyCtxt<'_>, key: LocalDefId) -> ty::Visibility<RustcDefId> {
    let state = GENERATE_STATE
        .get()
        .cloned()
        .expect("generate state missing");
    let guard = state.state.try_lock().unwrap();
    let defined_crate = match &guard.defined_crate {
        DefinedCrateState::Stage1(info) | DefinedCrateState::Stage2(info, _, _, ()) => info,
        _ => return ty::Visibility::Public,
    };
    for item in &defined_crate.items {
        if my_def_id_to_rustc_def_id(tcx, item.def_id()).as_local() == Some(key) {
            return match item.visibility {
                Visibility::Public => ty::Visibility::Public,
                _ => ty::Visibility::Restricted(RustcDefId::local(
                    rustc_span::def_id::DefIndex::from_usize(0),
                )),
            };
        }
    }
    ty::Visibility::Public
}

fn generated_reachable_set(
    tcx: TyCtxt<'_>,
    (): (),
) -> rustc_data_structures::unord::UnordSet<LocalDefId> {
    with_generated_and_original(tcx, |generated, original| {
        let mut reachable = rustc_data_structures::unord::UnordSet::default();
        let DefinedCrateState::Stage2(defined_crate, _, _, ()) = generated else {
            return (original.reachable_set)(tcx, ());
        };
        for item in &defined_crate.items {
            let Some(local_def_id) = my_def_id_to_rustc_def_id(tcx, item.def_id()).as_local()
            else {
                continue;
            };
            reachable.insert(local_def_id);
        }
        reachable
    })
}

fn generated_mir_built<S: CrateGeneratorState>(
    tcx: TyCtxt<'_>,
    def_id: LocalDefId,
) -> &Steal<rustc_middle::mir::Body<'_>> {
    let (is_generated, original) = {
        let state = GENERATE_STATE
            .get()
            .cloned()
            .expect("generate state missing");
        let guard = state.state.try_lock().unwrap();
        let original = guard.original.expect("original providers missing");
        (guard.defined_crate.contains_key(tcx, &def_id), original)
    };
    if !is_generated {
        return (original.mir_built)(tcx, def_id);
    }

    let key = rustc_def_to_my_def(tcx, def_id.to_def_id());

    let (mir, block_scopes) = {
        let mut guard = MIR_STATE.get().unwrap().try_lock().unwrap();
        let MirState(state, context) = &mut *guard;
        let state = state.downcast_mut::<S>().unwrap();

        state.emit_mir(
            crate::HirStructureCtx {
                tcx,
                inner: context.clone(),
            },
            key,
        )
    };

    let body = build_mir_body(tcx, &mir, def_id, &block_scopes);

    unsafe { std::mem::transmute(leak(Steal::new(body))) }
}

fn generated_mir_borrowck<'tcx>(
    tcx: TyCtxt<'tcx>,
    def_id: LocalDefId,
) -> BorrowckProvidedValue<'tcx> {
    let (is_generated, original) = {
        let state = GENERATE_STATE
            .get()
            .cloned()
            .expect("generate state missing");
        let guard = state.state.try_lock().unwrap();
        let original = guard.original.expect("original providers missing");
        (guard.defined_crate.contains_key(tcx, &def_id), original)
    };

    if is_generated {
        let opaque_types: FxIndexMap<LocalDefId, ty::DefinitionSiteHiddenType<'tcx>> =
            FxIndexMap::default();
        Ok(tcx.arena.alloc(opaque_types))
    } else {
        (original.mir_borrowck)(tcx, def_id)
    }
}

fn random_fingerprint() -> Fingerprint {
    Fingerprint::new::<u64, u64>(rand::random(), rand::random())
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

fn make_c_variadic_param(item_allocator: &mut HirItemAllocator) -> hir::Param<'static> {
    let pat_hir_id = item_allocator.new_item();
    let pat = hir::Pat {
        hir_id: pat_hir_id,
        kind: hir::PatKind::Binding(
            hir::BindingMode(hir::ByRef::No, hir::Mutability::Mut),
            pat_hir_id,
            Ident::from_str("__co2_c_varargs"),
            None,
        ),
        span: DUMMY_SP,
        default_binding_modes: false,
    };
    let param_hir_id = item_allocator.new_item();
    let pat = leak(pat);
    item_allocator.set_node(
        pat_hir_id.local_id,
        hir::Node::Pat(pat),
        param_hir_id.local_id,
    );
    hir::Param {
        hir_id: param_hir_id,
        pat,
        ty_span: DUMMY_SP,
        span: DUMMY_SP,
    }
}

fn input_ident(input: &crate::FunctionInput) -> Ident {
    Ident::from_str(input.name.as_deref().unwrap_or("_"))
}

fn make_named_param(
    item_allocator: &mut HirItemAllocator,
    name: Option<&str>,
) -> hir::Param<'static> {
    let pat_hir_id = item_allocator.new_item();
    let ident = Ident::from_str(name.unwrap_or("_"));
    let pat = hir::Pat {
        hir_id: pat_hir_id,
        kind: hir::PatKind::Binding(
            hir::BindingMode(hir::ByRef::No, hir::Mutability::Not),
            pat_hir_id,
            ident,
            None,
        ),
        span: DUMMY_SP,
        default_binding_modes: false,
    };
    let param_hir_id = item_allocator.new_item();
    let pat = leak(pat);
    item_allocator.set_node(
        pat_hir_id.local_id,
        hir::Node::Pat(pat),
        param_hir_id.local_id,
    );
    hir::Param {
        hir_id: param_hir_id,
        pat,
        ty_span: DUMMY_SP,
        span: DUMMY_SP,
    }
}

fn make_self_param(item_allocator: &mut HirItemAllocator) -> hir::Param<'static> {
    let pat_hir_id = item_allocator.new_item();
    let pat = hir::Pat {
        hir_id: pat_hir_id,
        kind: hir::PatKind::Binding(
            hir::BindingMode(hir::ByRef::No, hir::Mutability::Not),
            pat_hir_id,
            Ident::from_str("self"),
            None,
        ),
        span: DUMMY_SP,
        default_binding_modes: false,
    };
    let param_hir_id = item_allocator.new_item();
    let pat = leak(pat);
    item_allocator.set_node(
        pat_hir_id.local_id,
        hir::Node::Pat(pat),
        param_hir_id.local_id,
    );
    hir::Param {
        hir_id: param_hir_id,
        pat,
        ty_span: DUMMY_SP,
        span: DUMMY_SP,
    }
}

fn build_mir_body<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &MirBody,
    owner: LocalDefId,
    block_scopes: &[u32],
) -> rustc_middle::mir::Body<'static> {
    let max_vdi_scope = body
        .var_debug_info
        .iter()
        .map(|info| info.source_info.scope)
        .max()
        .unwrap_or(0);
    let max_block_scope = block_scopes.iter().copied().max().unwrap_or(0);
    let max_scope = max_vdi_scope.max(max_block_scope) as usize;
    let source_scopes: IndexVec<
        rustc_middle::mir::SourceScope,
        rustc_middle::mir::SourceScopeData,
    > = (0..=max_scope)
        .map(|i| rustc_middle::mir::SourceScopeData {
            span: rustc_public::rustc_internal::internal(tcx, body.span),
            parent_scope: if i == 0 {
                None
            } else {
                Some(rustc_middle::mir::SourceScope::from_usize(0))
            },
            inlined: None,
            inlined_parent_scope: None,
            local_data: rustc_middle::mir::ClearCrossCrate::Set(
                rustc_middle::mir::SourceScopeLocalData {
                    lint_root: HirId::make_owner(owner),
                },
            ),
        })
        .collect();
    let source_scope = rustc_middle::mir::SourceScope::from_usize(0);

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
                span: rustc_public::rustc_internal::internal(tcx, local.span),
                scope: source_scope,
            },
        })
        .collect();

    let mut blocks = Vec::new();
    for (block_idx, block) in body.blocks.iter().enumerate() {
        let block_scope = block_scopes
            .get(block_idx)
            .copied()
            .map(|s| rustc_middle::mir::SourceScope::from_usize(s as usize))
            .unwrap_or(source_scope);
        let mut statements = Vec::new();
        for stmt in &block.statements {
            match &stmt.kind {
                MirStatementKind::Assign(place, rvalue) => {
                    let place = mir_place_to_rustc(tcx, place);
                    let rvalue = mir_rvalue_to_rustc(tcx, rvalue);
                    statements.push(rustc_middle::mir::Statement::new(
                        rustc_middle::mir::SourceInfo {
                            span: rustc_public::rustc_internal::internal(tcx, stmt.span),
                            scope: block_scope,
                        },
                        rustc_middle::mir::StatementKind::Assign(Box::new((place, rvalue))),
                    ));
                }
                _ => todo!(),
            }
        }

        let terminator = rustc_middle::mir::Terminator {
            source_info: rustc_middle::mir::SourceInfo {
                span: rustc_public::rustc_internal::internal(tcx, block.terminator.span),
                scope: block_scope,
            },
            kind: match &block.terminator.kind {
                MirTerminatorKind::Goto { target } => rustc_middle::mir::TerminatorKind::Goto {
                    target: rustc_middle::mir::BasicBlock::from_usize(*target),
                },
                MirTerminatorKind::SwitchInt { discr, targets } => {
                    let discr = mir_operand_to_rustc(tcx, discr);
                    let targets = rustc_middle::mir::SwitchTargets::new(
                        targets
                            .branches()
                            .map(|(v, bb)| (v, rustc_middle::mir::BasicBlock::from_usize(bb))),
                        rustc_middle::mir::BasicBlock::from_usize(targets.otherwise()),
                    );
                    rustc_middle::mir::TerminatorKind::SwitchInt { discr, targets }
                }
                MirTerminatorKind::Return => rustc_middle::mir::TerminatorKind::Return,
                MirTerminatorKind::Call {
                    func,
                    args,
                    destination,
                    target,
                    unwind: _,
                } => {
                    let func = mir_operand_to_rustc(tcx, func);
                    let args: Box<[rustc_span::Spanned<rustc_middle::mir::Operand<'tcx>>]> = args
                        .iter()
                        .map(|arg| rustc_span::Spanned {
                            node: mir_operand_to_rustc(tcx, arg),
                            span: rustc_public::rustc_internal::internal(
                                tcx,
                                block.terminator.span,
                            ),
                        })
                        .collect::<Vec<_>>()
                        .into_boxed_slice();
                    let destination = mir_place_to_rustc(tcx, destination);
                    let target = target.map(rustc_middle::mir::BasicBlock::from_usize);
                    rustc_middle::mir::TerminatorKind::Call {
                        func,
                        args,
                        destination,
                        target,
                        unwind: rustc_middle::mir::UnwindAction::Continue,
                        call_source: rustc_middle::mir::CallSource::Normal,
                        fn_span: rustc_public::rustc_internal::internal(tcx, block.terminator.span),
                    }
                }
                _ => todo!(),
            },
        };

        blocks.push(rustc_middle::mir::BasicBlockData::new_stmts(
            statements,
            Some(terminator),
            false,
        ));
    }

    let basic_blocks = IndexVec::from_iter(blocks);
    let local_decls = IndexVec::from_iter(locals);

    let var_debug_info: Vec<rustc_middle::mir::VarDebugInfo<'tcx>> = body
        .var_debug_info
        .iter()
        .map(|info| {
            let value = match &info.value {
                rustc_public::mir::VarDebugInfoContents::Place(place) => {
                    rustc_middle::mir::VarDebugInfoContents::Place(mir_place_to_rustc(tcx, place))
                }
                rustc_public::mir::VarDebugInfoContents::Const(_) => {
                    todo!("VarDebugInfoContents::Const conversion not yet implemented")
                }
            };
            rustc_middle::mir::VarDebugInfo {
                name: rustc_span::Symbol::intern(&info.name),
                source_info: rustc_middle::mir::SourceInfo {
                    span: rustc_public::rustc_internal::internal(tcx, info.source_info.span),
                    scope: rustc_middle::mir::SourceScope::from_usize(
                        info.source_info.scope as usize,
                    ),
                },
                composite: None,
                value,
                argument_index: info.argument_index,
            }
        })
        .collect();

    let body = rustc_middle::mir::Body::new(
        rustc_middle::mir::MirSource::item(owner.to_def_id()),
        basic_blocks,
        source_scopes,
        local_decls,
        IndexVec::new(),
        body.arg_locals().len(),
        var_debug_info,
        rustc_public::rustc_internal::internal(tcx, body.span),
        None,
        None,
    );

    unsafe {
        std::mem::transmute::<rustc_middle::mir::Body<'tcx>, rustc_middle::mir::Body<'static>>(body)
    }
}

fn mir_ty_to_rustc<'tcx>(tcx: TyCtxt<'tcx>, ty: &MirTy) -> ty::Ty<'tcx> {
    internal(tcx, ty)
}

fn mir_region_to_rustc<'tcx>(tcx: TyCtxt<'tcx>, ty: &rustc_public::ty::Region) -> ty::Region<'tcx> {
    internal(tcx, ty)
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
        MirRvalue::Use(op, _) => rustc_middle::mir::Rvalue::Use(
            mir_operand_to_rustc(tcx, op),
            rustc_middle::mir::WithRetag::Yes,
        ),
        MirRvalue::ThreadLocalRef(item) => {
            let def_id = internal(tcx, *item);
            // Skip thread-local checks for Const DefKinds - they shouldn't have MIR bodies
            let def_kind = tcx.def_kind(def_id);
            if matches!(def_kind, DefKind::Const { is_type_const: _ }) {
                // For Const items, just emit a placeholder - the actual value is in the AnonConst
                let ty = tcx.type_of(def_id).instantiate_identity();
                let ptr_ty = rustc_middle::ty::Ty::new_mut_ptr(tcx, ty.skip_normalization());
                let const_ = rustc_middle::mir::Const::Val(
                    ConstValue::Scalar(Scalar::from_pointer(
                        Pointer::new(
                            CtfeProvenance::from(rustc_middle::mir::interpret::AllocId(
                                std::num::NonZeroU64::MIN,
                            )),
                            rustc_abi::Size::ZERO,
                        ),
                        &tcx,
                    )),
                    ptr_ty,
                );
                let op = rustc_middle::mir::Operand::Constant(Box::new(
                    rustc_middle::mir::ConstOperand {
                        span: DUMMY_SP,
                        user_ty: None,
                        const_,
                    },
                ));
                rustc_middle::mir::Rvalue::Use(op, rustc_middle::mir::WithRetag::Yes)
            } else if tcx.is_thread_local_static(def_id) {
                rustc_middle::mir::Rvalue::ThreadLocalRef(def_id)
            } else {
                let alloc_id = tcx.reserve_and_set_static_alloc(def_id);
                let ptr = Pointer::new(CtfeProvenance::from(alloc_id), rustc_abi::Size::ZERO);
                let scalar = Scalar::from_pointer(ptr, &tcx);
                let ty = tcx.type_of(def_id).instantiate_identity();
                let ptr_ty = rustc_middle::ty::Ty::new_mut_ptr(tcx, ty.skip_normalization());
                let const_ = rustc_middle::mir::Const::Val(ConstValue::Scalar(scalar), ptr_ty);
                let op = rustc_middle::mir::Operand::Constant(Box::new(
                    rustc_middle::mir::ConstOperand {
                        span: DUMMY_SP,
                        user_ty: None,
                        const_,
                    },
                ));
                rustc_middle::mir::Rvalue::Use(op, rustc_middle::mir::WithRetag::Yes)
            }
        }
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
        MirRvalue::UnaryOp(operation, operand) => rustc_middle::mir::Rvalue::UnaryOp(
            mir_un_op_to_rustc(*operation),
            mir_operand_to_rustc(tcx, operand),
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

fn mir_un_op_to_rustc(op: rustc_public::mir::UnOp) -> rustc_middle::mir::UnOp {
    match op {
        rustc_public::mir::UnOp::Not => rustc_middle::mir::UnOp::Not,
        rustc_public::mir::UnOp::Neg => rustc_middle::mir::UnOp::Neg,
        rustc_public::mir::UnOp::PtrMetadata => rustc_middle::mir::UnOp::PtrMetadata,
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
                let konst = internal(tcx, konst.clone());
                ty::GenericArg::from(konst)
            }
        };
        rustc_args.push(rustc_arg);
    }
    tcx.mk_args(&rustc_args)
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
    rustc_middle::mir::ConstOperand {
        span: internal(tcx, konst.span),
        user_ty: None,
        const_: internal(tcx, konst.const_.clone()),
    }
}

fn make_owner_info_with_attrs(
    nodes: hir::OwnerNodes<'static>,
    attrs: Option<Vec<hir::Attribute>>,
) -> hir::OwnerInfo<'static> {
    let mut map = rustc_data_structures::sorted_map::SortedMap::new();
    if let Some(attrs) = attrs {
        let attrs: &'static [hir::Attribute] = Box::leak(attrs.into_boxed_slice());
        map.insert(ItemLocalId::new(0), attrs);
    }
    hir::OwnerInfo {
        nodes,
        parenting: LocalDefIdMap::default(),
        attrs: hir::AttributeMap {
            map,
            define_opaque: None,
            opt_hash: Some(Fingerprint::ZERO),
        },
        trait_map: ItemLocalMap::default(),
        delayed_lints: Steal::new(Vec::new().into_boxed_slice()),
    }
}

fn make_owner_info(nodes: hir::OwnerNodes<'static>) -> hir::OwnerInfo<'static> {
    make_owner_info_with_attrs(nodes, None)
}

fn make_def_path(
    tcx: TyCtxt<'_>,
    owner: LocalDefId,
    def_id: RustcDefId,
    kind: DefKind,
) -> hir::Path<'static> {
    let ident = Ident::from_str(tcx.item_name(def_id).as_str());
    let segment = hir::PathSegment::new(ident, HirId::make_owner(owner), Res::Def(kind, def_id));
    let segments = leak(vec![segment].into_boxed_slice());
    hir::Path {
        span: DUMMY_SP,
        res: Res::Def(kind, def_id),
        segments,
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

fn make_prim_ty(owner: LocalDefId, prim: hir::PrimTy, span: RustcSpan) -> hir::Ty<'static> {
    let ident = Ident::from_str(prim.name_str());
    let segment = hir::PathSegment::new(ident, HirId::make_owner(owner), Res::PrimTy(prim));
    let segments = leak(vec![segment].into_boxed_slice());
    let path = leak(hir::Path {
        span,
        res: Res::PrimTy(prim),
        segments,
    });
    hir::Ty {
        hir_id: HirId::make_owner(owner),
        span,
        kind: hir::TyKind::Path(hir::QPath::Resolved(None, path)),
    }
}

fn make_array_ty(
    tcx: TyCtxt<'_>,
    owner: LocalDefId,
    pointee: &'static hir::Ty<'static>,
    len: HirTyConst,
    span: RustcSpan,
) -> hir::Ty<'static> {
    hir::Ty {
        hir_id: HirId::make_owner(owner),
        span,
        kind: hir::TyKind::Array(
            pointee,
            leak(hir::ConstArg {
                hir_id: HirId::make_owner(owner),
                span,
                kind: match len {
                    HirTyConst::Literal(len) => hir::ConstArgKind::Literal {
                        lit: rustc_ast::LitKind::Int(
                            Pu128(len as u128),
                            rustc_ast::LitIntType::Unsuffixed,
                        ),
                        negated: false,
                    },
                    HirTyConst::ConstDef(def_id) => hir::ConstArgKind::Path(make_def_id_qpath(
                        tcx,
                        owner,
                        my_def_id_to_rustc_def_id(tcx, def_id),
                    )),
                },
            }),
        ),
    }
}

fn make_ptr_ty(
    owner: LocalDefId,
    pointee: &'static hir::Ty<'static>,
    mutability: hir::Mutability,
    span: RustcSpan,
) -> hir::Ty<'static> {
    hir::Ty {
        hir_id: HirId::make_owner(owner),
        span,
        kind: hir::TyKind::Ptr(hir::MutTy {
            ty: pointee,
            mutbl: mutability,
        }),
    }
}

struct HirItemAllocator {
    owner: LocalDefId,
    nodes: IndexVec<ItemLocalId, Option<hir::ParentedNode<'static>>>,
    bodies: rustc_data_structures::sorted_map::SortedMap<ItemLocalId, &'static hir::Body<'static>>,
}

impl HirItemAllocator {
    fn new(owner: LocalDefId) -> Self {
        HirItemAllocator {
            owner,
            nodes: IndexVec::from([None]),
            bodies: rustc_data_structures::sorted_map::SortedMap::new(),
        }
    }

    fn new_item(&mut self) -> hir::HirId {
        let local_id = self.nodes.push(None);
        HirId {
            owner: OwnerId { def_id: self.owner },
            local_id,
        }
    }

    fn into_owner_nodes(self) -> hir::OwnerNodes<'static> {
        let nodes: IndexVec<ItemLocalId, hir::ParentedNode<'static>> =
            self.nodes.into_iter().map(|node| node.unwrap()).collect();

        hir::OwnerNodes {
            opt_hash_including_bodies: Some(random_fingerprint()),
            nodes,
            bodies: self.bodies,
        }
    }

    fn insert_body(&mut self, key: ItemLocalId, body: &'static hir::Body<'static>) {
        self.bodies.insert(key, body);
    }

    fn set_node(
        &mut self,
        id: hir::ItemLocalId,
        node: rustc_hir::Node<'static>,
        parent: hir::ItemLocalId,
    ) {
        self.nodes[id] = Some(hir::ParentedNode { parent, node });
    }

    fn set_root_node(&mut self, node: rustc_hir::Node<'static>) {
        self.set_node(hir::ItemLocalId::ZERO, node, hir::ItemLocalId::INVALID);
    }
}

fn make_lifetime(
    tcx: TyCtxt<'_>,
    lifetime: &crate::HirLifetime,
    item_allocator: &mut HirItemAllocator,
) -> &'static hir::Lifetime {
    let l = leak(hir::Lifetime {
        hir_id: item_allocator.new_item(),
        ident: Ident::dummy(),
        kind: match lifetime {
            crate::HirLifetime::Static => hir::LifetimeKind::Static,
            crate::HirLifetime::Param(def_id) => {
                let def = my_def_id_to_rustc_def_id(tcx, *def_id).expect_local();
                hir::LifetimeKind::Param(def)
            }
        },
        source: hir::LifetimeSource::Reference,
        syntax: hir::LifetimeSyntax::Implicit,
    });
    item_allocator.set_node(l.hir_id.local_id, hir::Node::Lifetime(l), ItemLocalId::ZERO);
    l
}

/// Build a `Generics` with one `GenericParam` per lifetime DefId in `lifetimes`.
/// Each param is registered as a `Node::GenericParam` in `item_allocator`.
fn build_fn_generics(
    tcx: TyCtxt<'_>,
    lifetimes: &[DefId],
    item_allocator: &mut HirItemAllocator,
) -> &'static hir::Generics<'static> {
    if lifetimes.is_empty() {
        return hir::Generics::empty();
    }
    let params: Vec<hir::GenericParam<'static>> = lifetimes
        .iter()
        .map(|&lt_def_id| {
            let local_def_id = my_def_id_to_rustc_def_id(tcx, lt_def_id).expect_local();
            let name = tcx.item_name(local_def_id.to_def_id());
            let hir_id = item_allocator.new_item();

            hir::GenericParam {
                hir_id,
                def_id: local_def_id,
                name: hir::ParamName::Plain(Ident::with_dummy_span(name)),
                span: DUMMY_SP,
                pure_wrt_drop: false,
                kind: hir::GenericParamKind::Lifetime {
                    kind: hir::LifetimeParamKind::Explicit,
                },
                colon_span: None,
                source: hir::GenericParamSource::Generics,
            }
        })
        .collect();
    let params: &'static [hir::GenericParam<'static>] = leak(params.into_boxed_slice());
    for param in params {
        item_allocator.set_node(
            param.hir_id.local_id,
            hir::Node::GenericParam(param),
            ItemLocalId::ZERO,
        );
    }
    let generics = hir::Generics {
        params,
        predicates: &[],
        has_where_clause_predicates: false,
        where_clause_span: DUMMY_SP,
        span: DUMMY_SP,
    };
    leak(generics)
}

fn make_tuple_ty(
    tcx: TyCtxt<'_>,
    owner: LocalDefId,
    elems: &[HirTy],
    item_allocator: &mut HirItemAllocator,
    span: RustcSpan,
) -> hir::Ty<'static> {
    let empty: &'static [hir::Ty<'static>] = leak(
        elems
            .iter()
            .map(|ty| hir_ty_to_rustc(tcx, owner, ty, item_allocator))
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    );
    hir::Ty {
        hir_id: HirId::make_owner(owner),
        span,
        kind: hir::TyKind::Tup(empty),
    }
}

fn make_def_id_qpath(
    tcx: TyCtxt<'_>,
    owner: LocalDefId,
    def_id: RustcDefId,
) -> hir::QPath<'static> {
    let kind = tcx.def_kind(def_id);
    let ident = Ident::from_str(tcx.item_name(def_id).as_str());
    let segment = hir::PathSegment::new(ident, HirId::make_owner(owner), Res::Def(kind, def_id));
    let segments = leak(vec![segment].into_boxed_slice());
    let path = leak(hir::Path {
        span: DUMMY_SP,
        res: Res::Def(kind, def_id),
        segments,
    });
    hir::QPath::Resolved(None, path)
}

fn make_adt_ty(
    tcx: TyCtxt<'_>,
    owner: LocalDefId,
    adt: DefId,
    args: &[crate::HirGenericArg],
    item_allocator: &mut HirItemAllocator,
    span: RustcSpan,
) -> hir::Ty<'static> {
    let def_id = my_def_id_to_rustc_def_id(tcx, adt);
    let kind = tcx.def_kind(def_id);
    let ident = Ident::from_str(tcx.item_name(def_id).as_str());
    let mut segment =
        hir::PathSegment::new(ident, HirId::make_owner(owner), Res::Def(kind, def_id));
    if !args.is_empty() {
        let mut hir_args = Vec::with_capacity(args.len());
        for arg in args {
            match arg {
                crate::HirGenericArg::Ty(ty) => {
                    let ty = leak(hir_ty_to_rustc(tcx, owner, ty, item_allocator));
                    let ambig = ty
                        .try_as_ambig_ty()
                        .expect("generic type argument unexpectedly inferred");
                    hir_args.push(hir::GenericArg::Type(ambig));
                }
                crate::HirGenericArg::Lifetime(l) => {
                    hir_args.push(hir::GenericArg::Lifetime(make_lifetime(
                        tcx,
                        l,
                        item_allocator,
                    )));
                }
            }
        }
        let generic_args = leak(hir::GenericArgs {
            args: leak(hir_args.into_boxed_slice()),
            constraints: &[],
            parenthesized: hir::GenericArgsParentheses::No,
            span_ext: span,
        });
        segment.args = Some(generic_args);
        segment.infer_args = false;
    }
    let segments = leak(vec![segment].into_boxed_slice());
    let path = leak(hir::Path {
        span,
        res: Res::Def(kind, def_id),
        segments,
    });
    hir::Ty {
        hir_id: HirId::make_owner(owner),
        span,
        kind: hir::TyKind::Path(hir::QPath::Resolved(None, path)),
    }
}

fn hir_ty_to_rustc(
    tcx: TyCtxt<'_>,
    owner: LocalDefId,
    ty: &HirTy,
    item_allocator: &mut HirItemAllocator,
) -> hir::Ty<'static> {
    let span = internal(tcx, ty.span);
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
            make_prim_ty(owner, hir::PrimTy::Int(int_ty), span)
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
            make_prim_ty(owner, hir::PrimTy::Uint(int_ty), span)
        }
        HirTyKind::Bool => make_prim_ty(owner, hir::PrimTy::Bool, span),
        HirTyKind::Char => make_prim_ty(owner, hir::PrimTy::Char, span),
        HirTyKind::Str => make_prim_ty(owner, hir::PrimTy::Str, span),
        HirTyKind::Float(float_ty) => {
            let float_ty = match float_ty {
                rustc_public::ty::FloatTy::F16 => FloatTy::F16,
                rustc_public::ty::FloatTy::F32 => FloatTy::F32,
                rustc_public::ty::FloatTy::F64 => FloatTy::F64,
                rustc_public::ty::FloatTy::F128 => FloatTy::F128,
            };
            make_prim_ty(owner, hir::PrimTy::Float(float_ty), span)
        }
        HirTyKind::RawPtr(mutability, to) => {
            let pointee = leak(hir_ty_to_rustc(tcx, owner, to, item_allocator));
            make_ptr_ty(
                owner,
                pointee,
                match mutability {
                    rustc_public::mir::Mutability::Not => hir::Mutability::Not,
                    rustc_public::mir::Mutability::Mut => hir::Mutability::Mut,
                },
                span,
            )
        }
        HirTyKind::Array(len, to) => {
            let pointee = leak(hir_ty_to_rustc(tcx, owner, to, item_allocator));
            make_array_ty(tcx, owner, pointee, *len, span)
        }
        HirTyKind::Ref(mutability, lifetime, to) => {
            let pointee = leak(hir_ty_to_rustc(tcx, owner, to, item_allocator));
            let lifetime = make_lifetime(tcx, lifetime, item_allocator);
            hir::Ty {
                hir_id: HirId::make_owner(owner),
                span,
                kind: hir::TyKind::Ref(
                    lifetime,
                    hir::MutTy {
                        ty: pointee,
                        mutbl: match mutability {
                            rustc_public::mir::Mutability::Not => hir::Mutability::Not,
                            rustc_public::mir::Mutability::Mut => hir::Mutability::Mut,
                        },
                    },
                ),
            }
        }
        HirTyKind::Adt(adt, args) => make_adt_ty(tcx, owner, *adt, args, item_allocator, span),
        HirTyKind::Tuple(elems) => make_tuple_ty(tcx, owner, elems, item_allocator, span),
        HirTyKind::FnPtr(sig) => {
            let hir_id = item_allocator.new_item();
            let fn_decl = leak(hir::FnDecl {
                inputs: leak(
                    sig.inputs
                        .iter()
                        .map(|input| hir_ty_to_rustc(tcx, owner, &input.ty, item_allocator))
                        .collect::<Vec<_>>(),
                ),
                output: hir::FnRetTy::Return(leak(hir_ty_to_rustc(
                    tcx,
                    owner,
                    &sig.output,
                    item_allocator,
                ))),
                fn_decl_kind: hir::FnDeclFlags::default()
                    .set_c_variadic(sig.c_variadic)
                    .set_implicit_self(hir::ImplicitSelfKind::None)
                    .set_lifetime_elision_allowed(true),
            });
            let param_idents = leak(
                sig.inputs
                    .iter()
                    .map(|input| Some(input_ident(input)))
                    .collect::<Vec<_>>(),
            );
            let safety = if sig.is_unsafe {
                hir::Safety::Unsafe
            } else {
                hir::Safety::Safe
            };
            let abi = match sig.abi {
                FunctionAbi::Rust => ExternAbi::Rust,
                FunctionAbi::C => ExternAbi::C { unwind: false },
            };
            let fn_ptr = leak(hir::FnPtrTy {
                safety,
                abi,
                generic_params: &[],
                decl: fn_decl,
                param_idents,
            });
            let ty = leak(hir::Ty {
                hir_id,
                kind: hir::TyKind::FnPtr(fn_ptr),
                span,
            });
            item_allocator.set_node(hir_id.local_id, hir::Node::Ty(ty), ItemLocalId::ZERO);
            *ty
        }
        HirTyKind::Never => {
            let hir_id = item_allocator.new_item();
            let ty = leak(hir::Ty {
                kind: hir::TyKind::Never,
                span,
                hir_id,
            });
            item_allocator.set_node(hir_id.local_id, hir::Node::Ty(ty), ItemLocalId::ZERO);
            *ty
        }
    }
}

fn leak<T>(value: T) -> &'static T {
    Box::leak(Box::new(value))
}
