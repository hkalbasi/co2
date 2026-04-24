use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use rustc_abi::ExternAbi;
use rustc_ast::token::Token;
use rustc_ast::tokenstream::{DelimSpan, TokenStream, TokenTree};
use rustc_ast::{Attribute, FloatTy, IntTy, UintTy};
use rustc_data_structures::fingerprint::Fingerprint;
use rustc_data_structures::fx::FxHashMap;
use rustc_data_structures::packed::Pu128;
use rustc_data_structures::steal::Steal;
use rustc_data_structures::thin_vec::ThinVec;
use rustc_hir as hir;
use rustc_hir::def::{CtorKind, DefKind, Res};
use rustc_hir::def_id::{CRATE_DEF_ID, DefId as RustcDefId, LocalDefId, LocalDefIdMap};
use rustc_hir::definitions::{DefPathData, Definitions, DisambiguatorState};
use rustc_hir::lang_items::LangItem;
use rustc_hir::{HirId, ItemLocalId, ItemLocalMap, OwnerId};
use rustc_index::{Idx, IndexVec};
use rustc_lint::Level;
use rustc_middle::mir::interpret::{CtfeProvenance, Pointer, Scalar};
use rustc_middle::mir::{BorrowKind, ConstValue};
use rustc_middle::query::Providers as QueryProviders;
use rustc_middle::ty::{self, TyCtxt};
use rustc_middle::util::Providers as UtilProviders;
use rustc_public::rustc_internal::internal;
use rustc_session::config::EntryFnType;
use rustc_span::symbol::{Ident, Symbol};
use rustc_span::{BytePos, DUMMY_SP, Span as RustcSpan, SyntaxContext};
use rustc_trait_selection::infer::{InferCtxtExt as _, TyCtxtInferExt as _};

use crate::hir_structure::{AdtRepr, FunctionAbi, FunctionSignature, StructField};
use crate::hir_ty::HirTyConst;
pub use crate::hir_ty::{HirTy, HirTyKind};
use crate::{
    CrateGeneratorState, DependencyConstValue, DependencyCrate, DependencyFunction, DependencyInfo,
    DependencyTrait, DependencyType, DependencyValue, DependencyValueKind, FileId,
};
use rustc_public::DefId;
use rustc_public::mir::{
    Body as MirBody, ConstOperand as MirConst, Mutability as MirMutability, Operand as MirOperand,
    Place as MirPlace, ProjectionElem as MirProjection, Rvalue as MirRvalue,
    StatementKind as MirStatementKind, TerminatorKind as MirTerminatorKind,
};
use rustc_public::ty::{
    AdtDef, FnDef, GenericArgKind, GenericArgs, RigidTy, Span as PublicSpan, Ty as MirTy,
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
                let real = source_map
                    .path_mapping()
                    .to_real_filename(source_map.working_dir(), file.path.as_path());
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
                    } => match kind {
                        crate::hir_structure::HirAdtKind::Struct { fields } => {
                            result.push(ItemSignatureInfo {
                                id: id.0,
                                kind: ItemSignatureKind::Struct(fields.clone()),
                                span: *span,
                            })
                        }
                        crate::hir_structure::HirAdtKind::Union { fields } => {
                            result.push(ItemSignatureInfo {
                                id: id.0,
                                kind: ItemSignatureKind::Union(fields.clone()),
                                span: *span,
                            })
                        }
                    },
                    crate::HirModuleItem::TypeDef {
                        id,
                        span,
                        ty,
                        name: _,
                    } => {
                        result.push(ItemSignatureInfo {
                            id: *id,
                            kind: ItemSignatureKind::TypeDef(ty.clone()),
                            span: *span,
                        });
                    }
                    crate::HirModuleItem::Const {
                        id,
                        span,
                        ty,
                        rhs,
                        name: _,
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
                        name: _,
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

        let crate_def = CRATE_DEF_ID;
        let parent_local = |my_def_id: rustc_public::DefId| {
            self.items
                .iter()
                .find(|item| item.def_id() == my_def_id)
                .and_then(|item| item.parent)
                .and_then(|parent| my_def_id_to_rustc_def_id(tcx, parent).as_local())
                .unwrap_or(crate_def)
        };
        let is_mod_item = |kind: DefinedItemKind| {
            matches!(
                kind,
                DefinedItemKind::Function { .. }
                    | DefinedItemKind::Struct(_, _)
                    | DefinedItemKind::Union(_, _)
                    | DefinedItemKind::TypeDef(_)
                    | DefinedItemKind::Static(_)
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

            let fn_sig = generate_sig(tcx, def_id, &foreign, &mut item_allocator);

            let foreign_item = hir::ForeignItem {
                ident: Ident::from_str(name),
                kind: hir::ForeignItemKind::Fn(
                    fn_sig,
                    leak(vec![None; 0].into_boxed_slice()),
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
                    .into_iter()
                    .map(|field| {
                        let field_def_id = my_def_id_to_rustc_def_id(tcx, field.id).expect_local();

                        let hir_id = item_allocator.new_item();
                        let hir_field_def = hir::FieldDef {
                            span: internal(tcx, field.span),
                            vis_span: internal(tcx, field.span),
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
                vis_span: internal(tcx, span),
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
        for my_def_id in signatures.iter().filter_map(|item| match item.kind {
            ItemSignatureKind::Module => Some(item.id),
            _ => None,
        }) {
            let name = &self
                .items
                .iter()
                .find(|item| item.def_id() == my_def_id)
                .unwrap()
                .name;
            let def_id = my_def_id_to_rustc_def_id(tcx, my_def_id).expect_local();
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
                            inner_span: DUMMY_SP,
                            inject_use_span: DUMMY_SP,
                        },
                        item_ids: leak(child_item_ids.into_boxed_slice()),
                    }),
                ),
                span: DUMMY_SP,
                vis_span: DUMMY_SP,
                has_delayed_lints: false,
                eii: false,
            };
            module_items_hir.push((def_id, leak(item)));
        }

        let mut impl_items_hir = Vec::new();
        let mut impl_item_fns_hir = Vec::new();
        for (my_def_id, self_ty, trait_def, items) in
            signatures.iter().filter_map(|item| match &item.kind {
                ItemSignatureKind::Impl {
                    self_ty,
                    trait_def,
                    items,
                } => Some((item.id, self_ty, *trait_def, items)),
                _ => None,
            })
        {
            let def_id = my_def_id_to_rustc_def_id(tcx, my_def_id).expect_local();
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
                span: DUMMY_SP,
                vis_span: DUMMY_SP,
                has_delayed_lints: false,
                eii: false,
            };
            impl_items_hir.push((def_id, leak(impl_item)));

            for item in items {
                let item_def_id = my_def_id_to_rustc_def_id(tcx, item.id).expect_local();
                let mut item_allocator = HirItemAllocator::new(item_def_id);

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
                        let params = if matches!(self_kind, crate::hir_structure::HirSelfKind::None)
                        {
                            Vec::new()
                        } else {
                            vec![make_self_param(&mut item_allocator)]
                        };
                        let loop_expr = leak(hir::Block {
                            stmts: &[],
                            expr: None,
                            hir_id: item_allocator.new_item(),
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
                            hir_id: body_hir_id,
                            kind: body_kind,
                            span: DUMMY_SP,
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
                    span: DUMMY_SP,
                    has_delayed_lints: false,
                };
                item_allocator.set_root_node(hir::Node::ImplItem(leak(impl_item)));
                impl_item_fns_hir.push((item_def_id, item_allocator, def_id));
            }
        }

        let mut items_hir = Vec::new();
        for (my_def_id, alias_ty) in signatures.iter().filter_map(|item| match &item.kind {
            ItemSignatureKind::TypeDef(ty) => Some((item.id, ty)),
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
            let item = hir::Item {
                owner_id: OwnerId { def_id },
                kind: hir::ItemKind::TyAlias(
                    Ident::from_str(name),
                    hir::Generics::empty(),
                    leak(hir_ty_to_rustc(tcx, def_id, alias_ty, &mut item_allocator)),
                ),
                span: DUMMY_SP,
                vis_span: DUMMY_SP,
                has_delayed_lints: false,
                eii: false,
            };
            item_allocator.set_root_node(hir::Node::Item(leak(item)));
            items_hir.push((def_id, item_allocator));
        }

        for (my_def_id, function) in signatures.iter().filter_map(|item| match &item.kind {
            ItemSignatureKind::Function(sig) => Some((item.id, sig)),
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

            let fn_sig = generate_sig(tcx, def_id, &function, &mut item_allocator);
            let loop_expr = leak(hir::Block {
                stmts: &[],
                expr: None,
                hir_id: item_allocator.new_item(),
                rules: rustc_hir::BlockCheckMode::DefaultBlock,
                span: DUMMY_SP,
                targeted_by_break: false,
            });
            let body_kind =
                hir::ExprKind::Loop(loop_expr, None, rustc_hir::LoopSource::Loop, DUMMY_SP);
            let body_expr = leak(hir::Expr {
                hir_id: body_hir_id,
                kind: body_kind,
                span: DUMMY_SP,
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
            let mut params = vec![];

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
                span: DUMMY_SP,
                vis_span: DUMMY_SP,
                has_delayed_lints: false,
                eii: false,
            };
            item_allocator.set_root_node(hir::Node::Item(leak(item)));
            items_hir.push((def_id, item_allocator));
        }

        for (my_def_id, static_ty, mutable) in
            signatures.iter().filter_map(|item| match &item.kind {
                ItemSignatureKind::Static { ty, mutable } => Some((item.id, ty, *mutable)),
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

            let loop_expr = leak(hir::Block {
                stmts: &[],
                expr: None,
                hir_id: item_allocator.new_item(),
                rules: rustc_hir::BlockCheckMode::DefaultBlock,
                span: DUMMY_SP,
                targeted_by_break: false,
            });
            let body_kind =
                hir::ExprKind::Loop(loop_expr, None, rustc_hir::LoopSource::Loop, DUMMY_SP);
            let body_expr = leak(hir::Expr {
                hir_id: body_hir_id,
                kind: body_kind,
                span: DUMMY_SP,
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
                span: DUMMY_SP,
                vis_span: DUMMY_SP,
                has_delayed_lints: false,
                eii: false,
            };
            item_allocator.set_root_node(hir::Node::Item(leak(item)));
            items_hir.push((def_id, item_allocator));
        }

        for (my_def_id, static_ty, mutable) in
            signatures.iter().filter_map(|item| match &item.kind {
                ItemSignatureKind::ForeignStatic { ty, mutable } => Some((item.id, ty, *mutable)),
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
                span: DUMMY_SP,
                vis_span: DUMMY_SP,
                has_delayed_lints: false,
            };
            item_allocator.set_root_node(hir::Node::ForeignItem(leak(foreign_item)));
            foreign_items_hir.push((def_id, item_allocator));
        }

        for (my_def_id, const_ty, rhs) in signatures.iter().filter_map(|item| match &item.kind {
            ItemSignatureKind::Const { ty, rhs } => Some((item.id, ty, *rhs)),
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

            let anon_const_def_id = my_def_id_to_rustc_def_id(tcx, rhs).expect_local();
            let anon_const_hir_id = item_allocator.new_item();

            let loop_expr = leak(hir::Block {
                stmts: &[],
                expr: None,
                hir_id: item_allocator.new_item(),
                rules: rustc_hir::BlockCheckMode::DefaultBlock,
                span: DUMMY_SP,
                targeted_by_break: false,
            });
            let body_kind =
                hir::ExprKind::Loop(loop_expr, None, rustc_hir::LoopSource::Loop, DUMMY_SP);
            let body_expr = leak(hir::Expr {
                hir_id: body_hir_id,
                kind: body_kind,
                span: DUMMY_SP,
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
                span: DUMMY_SP,
            });
            insert_non_owner(
                &mut owners,
                anon_const_def_id,
                hir::MaybeOwner::NonOwner(anon_const.hir_id),
            );
            let const_arg = leak(hir::ConstArg {
                hir_id: item_allocator.new_item(),
                span: DUMMY_SP,
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
                span: DUMMY_SP,
                vis_span: DUMMY_SP,
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

        RESULT.set(owners.clone()).unwrap();

        owners
    }

    fn from_hir_structure<'tcx>(
        tcx: TyCtxt<'tcx>,
        hir_structure: &crate::HirStructure,
    ) -> (Self, LocalDefId) {
        fn collect_module<'tcx>(
            tcx: TyCtxt<'tcx>,
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
                            name: "".to_owned(),
                            kind: DefinedItemKind::AnonConst(rhs),
                            span: DUMMY_SP,
                            parent: Some(id),
                        });
                        DefinedItemKind::Const(id)
                    }
                    crate::HirModuleItem::Static { id, .. } => DefinedItemKind::Static(id),
                    crate::HirModuleItem::Adt {
                        name: _,
                        id,
                        kind,
                        span: _,
                        repr,
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
                                        span: internal(tcx, field.span),
                                        parent: Some(id.0),
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
                            span: DUMMY_SP,
                            parent,
                        });
                        for item in impl_items {
                            match &item.kind {
                                crate::hir_structure::HirImplItemKind::Fn { .. } => {
                                    items.push(DefinedItemInfo {
                                        name: item.name.clone(),
                                        kind: DefinedItemKind::ImplItemFn(item.id),
                                        span: DUMMY_SP,
                                        parent: Some(id),
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
                        span,
                    } => {
                        items.push(DefinedItemInfo {
                            name,
                            kind: DefinedItemKind::Module(id),
                            span: internal(tcx, span),
                            parent,
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
                            name: "".to_owned(),
                            kind: DefinedItemKind::ForeignMod(foreign_mod_id),
                            span: DUMMY_SP,
                            parent,
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
                                    span: DUMMY_SP,
                                    parent: Some(foreign_mod_id),
                                }),
                                crate::hir_structure::ForeignModItem::ForeignStatic {
                                    name,
                                    id,
                                    mutable: _,
                                    ty: _,
                                    span: _,
                                } => items.push(DefinedItemInfo {
                                    name,
                                    kind: DefinedItemKind::Static(id),
                                    span: DUMMY_SP,
                                    parent: Some(foreign_mod_id),
                                }),
                            }
                        }
                        continue;
                    }
                };
                items.push(DefinedItemInfo {
                    name: hir_item.name().unwrap().to_owned(),
                    kind,
                    span: hir_item
                        .span()
                        .map(|s| internal(tcx, s))
                        .unwrap_or(DUMMY_SP),
                    parent,
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
            Self { items },
            the_foreign_def.expect("missing foreign mod"),
        )
    }
}

fn generate_sig<'tcx>(
    tcx: TyCtxt<'tcx>,
    owner: LocalDefId,
    function: &FunctionSignature,
    item_allocator: &mut HirItemAllocator,
) -> rustc_hir::FnSig<'static> {
    let fn_decl = leak(hir::FnDecl {
        inputs: leak(
            function
                .inputs
                .iter()
                .map(|ty| hir_ty_to_rustc(tcx, owner, ty, item_allocator))
                .collect::<Vec<_>>(),
        ),
        output: hir::FnRetTy::Return(leak(hir_ty_to_rustc(
            tcx,
            owner,
            &function.output,
            item_allocator,
        ))),
        c_variadic: function.c_variadic,
        implicit_self: hir::ImplicitSelfKind::None,
        lifetime_elision_allowed: true,
    });

    let fn_sig = hir::FnSig {
        header: hir::FnHeader {
            safety: match function.is_unsafe {
                true => hir::HeaderSafety::Normal(hir::Safety::Unsafe),
                false => hir::HeaderSafety::Normal(hir::Safety::Safe),
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
    };
    fn_sig
}

fn generate_sig_with_self<'tcx>(
    tcx: TyCtxt<'tcx>,
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
            .map(|ty| hir_ty_to_rustc(tcx, owner, ty, item_allocator)),
    );
    let fn_decl = leak(hir::FnDecl {
        inputs: leak(inputs),
        output: hir::FnRetTy::Return(leak(hir_ty_to_rustc(
            tcx,
            owner,
            &sig.output,
            item_allocator,
        ))),
        c_variadic: sig.c_variadic,
        implicit_self,
        lifetime_elision_allowed: true,
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

    fn hir_crate<'tcx>(&self, tcx: TyCtxt<'tcx>, _: ()) -> rustc_hir::Crate<'tcx> {
        let DefinedCrateState::Stage2(defined_crate, signatures, foreign_def_id, _) = self else {
            panic!("hir_crate query in stage {}", self.stage_id());
        };
        let owners = defined_crate.owners(tcx, signatures, *foreign_def_id);
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
            | DefinedCrateState::Stage2(defined_crate_info, _, _, _) => defined_crate_info
                .items
                .iter()
                .any(|item| my_def_id_to_rustc_def_id(tcx, item.def_id()).as_local() == Some(*key)),
        }
    }

    fn entry_fn<'tcx>(&self, tcx: TyCtxt<'tcx>, _: ()) -> Option<(RustcDefId, EntryFnType)> {
        let entry_fn = match self {
            DefinedCrateState::Stage0 => panic!("Can't eval entry_fn at stage0"),
            DefinedCrateState::Stage1(defined_crate_info)
            | DefinedCrateState::Stage2(defined_crate_info, _, _, _) => defined_crate_info
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

    fn def_kind<'tcx>(&self, tcx: TyCtxt<'tcx>, key: LocalDefId) -> Option<DefKind> {
        let DefinedCrateState::Stage2(items, _, _, _) = self else {
            return None;
        };
        let key = rustc_def_to_my_def(tcx, key.to_def_id());
        let kind = items.items.iter().find(|item| item.def_id() == key)?.kind;
        Some(match kind {
            DefinedItemKind::ForeignMod(_) => DefKind::ForeignMod,
            DefinedItemKind::Module(_) => DefKind::Mod,
            DefinedItemKind::Function { .. } => DefKind::Fn,
            DefinedItemKind::ForeignFunction(_) => DefKind::Fn,
            DefinedItemKind::Const(_) => DefKind::Const,
            DefinedItemKind::AnonConst(_) => DefKind::AnonConst,
            DefinedItemKind::Static(_) => DefKind::Static {
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

    fn def_span<'tcx>(&self, tcx: TyCtxt<'tcx>, key: LocalDefId) -> Option<RustcSpan> {
        let DefinedCrateState::Stage2(items, _, _, _) = self else {
            return None;
        };
        let key = rustc_def_to_my_def(tcx, key.to_def_id());
        Some(items.items.iter().find(|item| item.def_id() == key)?.span)
    }

    fn to_stage1(&mut self, defined_crate: DefinedCrateInfo) {
        let DefinedCrateState::Stage0 = self else {
            panic!("Moving to stage1 from stage {}", self.stage_id());
        };
        *self = DefinedCrateState::Stage1(defined_crate);
    }

    fn to_stage2<S: CrateGeneratorState>(
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
    pub span: RustcSpan,
    pub parent: Option<DefId>,
}

impl DefinedItemInfo {
    pub fn def_id(&self) -> rustc_public::DefId {
        match self.kind {
            DefinedItemKind::Function { fn_def, .. } | DefinedItemKind::ForeignFunction(fn_def) => {
                fn_def.0
            }
            DefinedItemKind::Struct(adt_def, _) | DefinedItemKind::Union(adt_def, _) => adt_def.0,
            DefinedItemKind::ForeignMod(def_id)
            | DefinedItemKind::Module(def_id)
            | DefinedItemKind::TypeDef(def_id)
            | DefinedItemKind::Static(def_id)
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
    Const(DefId),
    AnonConst(DefId),
    Static(DefId),
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

/// Run rustc_driver but emit a synthetic crate described by three callbacks.
///
/// Phase 1 (`define_items`) declares items and allocates their definitions.
/// Phase 2 (`define_signatures`) defines function signatures using allocated definitions.
/// Phase 3 (`emit_mir`) emits MIR bodies for generated local functions.
pub fn generate<S: CrateGeneratorState>() {
    generate_with_args::<S>(std::env::args().collect())
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
    context: Context,
    gate: Arc<GenerateGate>,
    phantom: PhantomData<S>,
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
    context: Option<Context>,
}

struct GenerateGate {
    state: Mutex<GenerateState>,
}

#[derive(Copy, Clone)]
struct OriginalProviders {
    hir_crate: for<'tcx> fn(TyCtxt<'tcx>, ()) -> hir::Crate<'tcx>,
    resolutions: for<'tcx> fn(TyCtxt<'tcx>, ()) -> &'tcx rustc_middle::ty::ResolverGlobalCtxt,
    def_kind: for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> DefKind,
    // def_span: for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> RustcSpan,
    // def_ident_span: for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> Option<RustcSpan>,
    reachable_set:
        for<'tcx> fn(TyCtxt<'tcx>, ()) -> rustc_data_structures::unord::UnordSet<LocalDefId>,
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

struct MirState(Box<dyn Any>, Context);

// TODO: these are very wrong
unsafe impl Sync for MirState {}
unsafe impl Send for MirState {}
unsafe impl Sync for GenerateGate {}
unsafe impl Send for GenerateGate {}

fn with_generated_and_original<'tcx, R>(
    _tcx: TyCtxt<'tcx>,
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

pub fn root_crate_def_id<'tcx>(tcx: TyCtxt<'tcx>) -> DefId {
    rustc_def_to_my_def(tcx, CRATE_DEF_ID.to_def_id())
}

#[allow(invalid_reference_casting)]
pub fn allocate_def_id<'tcx>(
    tcx: TyCtxt<'tcx>,
    parent: rustc_public::DefId,
    kind: crate::DefData,
) -> rustc_public::DefId {
    let defs_guard = tcx.definitions_untracked();
    let defs_mut = unsafe { &mut *(&*defs_guard as *const Definitions as *mut Definitions) };
    let parent = my_def_id_to_rustc_def_id(tcx, parent).expect_local();
    let data = match &kind {
        crate::DefData::ForeignMod => DefPathData::ForeignMod,
        crate::DefData::Module(name) => DefPathData::TypeNs(Symbol::intern(name)),
        crate::DefData::ValueNs(name) => DefPathData::ValueNs(Symbol::intern(name)),
        crate::DefData::TypeNs(name) => DefPathData::TypeNs(Symbol::intern(name)),
        crate::DefData::LifetimeNs(name) => DefPathData::LifetimeNs(Symbol::intern(name)),
        crate::DefData::Impl => DefPathData::Impl,
        crate::DefData::AnonConst => DefPathData::AnonConst,
    };
    let mut disamb = match kind {
        crate::DefData::Impl => {
            static IMPL_DISAMB: AtomicU32 = AtomicU32::new(1);
            let idx = IMPL_DISAMB.fetch_add(1, Ordering::Relaxed);
            DisambiguatorState::with(CRATE_DEF_ID, DefPathData::Impl, idx)
        }
        _ => DisambiguatorState::with(CRATE_DEF_ID, DefPathData::ValueNs(Symbol::intern("gen")), 1),
    };
    let def_id = defs_mut.create_def(parent, data, &mut disamb);
    rustc_def_to_my_def(tcx, def_id.to_def_id())
}

thread_local! {
    static CACHE_TO: RefCell<HashMap<DefId, RustcDefId>> = RefCell::new(HashMap::new());
    static CACHE_FROM: RefCell<HashMap<RustcDefId, DefId>> = RefCell::new(HashMap::new());
}

fn my_def_id_to_rustc_def_id<'tcx>(tcx: TyCtxt<'tcx>, def_id: DefId) -> RustcDefId {
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

fn rustc_def_to_my_def<'tcx>(_tcx: TyCtxt<'tcx>, def_id: RustcDefId) -> DefId {
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
            context: Context::new(),
            gate: Arc::new(GenerateGate {
                state: Mutex::new(GenerateState::default()),
            }),
            phantom: PhantomData,
        }
    }
}

impl<S: CrateGeneratorState> rustc_driver::Callbacks for GenerateCallbacks<S> {
    fn config(&mut self, config: &mut rustc_interface::Config) {
        if std::env::var("GEN_DEBUG").is_ok() {
            eprintln!("callbacks.config");
        }
        let _ = GENERATE_STATE.set(self.gate.clone());

        config.opts.lint_opts.extend([
            ("unused".to_owned(), Level::Allow),
            ("nonstandard_style".to_owned(), Level::Allow),
            ("arithmetic_overflow".to_owned(), Level::Warn),
        ]);
        config.override_queries = Some(override_queries::<S>);

        if let Some(gate) = GENERATE_STATE.get() {
            let mut guard = gate.state.try_lock().unwrap();
            if std::env::var("GEN_DEBUG").is_ok() {
                eprintln!("callbacks.config: storing callback");
            }
            guard.context = Some(self.context.clone());
        }
    }

    fn after_crate_root_parsing(
        &mut self,
        _compiler: &rustc_interface::interface::Compiler,
        krate: &mut rustc_ast::Crate,
    ) -> rustc_driver::Compilation {
        krate.attrs.push(Attribute {
            kind: rustc_ast::AttrKind::Normal(Box::new(rustc_ast::NormalAttr {
                item: rustc_ast::AttrItem {
                    unsafety: rustc_ast::Safety::Default,
                    path: rustc_ast::Path {
                        span: DUMMY_SP,
                        segments: [rustc_ast::PathSegment {
                            ident: Ident::from_str("feature"),
                            id: rustc_ast::NodeId::from_usize(666666),
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
                                        Symbol::intern("c_variadic"),
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
            id: rustc_span::AttrId::from_usize(666),
            style: rustc_ast::AttrStyle::Inner,
            span: DUMMY_SP,
        });
        rustc_driver::Compilation::Continue
    }

    fn after_expansion<'tcx>(
        &mut self,
        _compiler: &rustc_interface::interface::Compiler,
        tcx: TyCtxt<'tcx>,
    ) -> rustc_driver::Compilation {
        _ = rustc_public::rustc_internal::run(tcx, || {
            let gate = GENERATE_STATE.get().unwrap();
            let context = {
                let guard = gate.state.try_lock().unwrap();
                let context = guard.context.clone().unwrap();
                context
            };

            let (state, hir_structure) = S::hir_structure(crate::HirStructureCtx {
                tcx,
                inner: context.clone(),
            });

            let (defined_crate, foreign_mod_def) =
                DefinedCrateInfo::from_hir_structure(tcx, &hir_structure);

            {
                let mut guard = gate.state.try_lock().unwrap();
                guard.defined_crate.to_stage1(defined_crate.clone());
            };
            let sigs = ItemSignatureInfo::from_hir_structure(&hir_structure);
            {
                let mut guard = gate.state.try_lock().unwrap();
                guard.defined_crate.to_stage2(
                    sigs.clone(),
                    foreign_mod_def,
                    state,
                    context.clone(),
                );
            }
            defined_crate.owners(tcx, &sigs, foreign_mod_def);

            _ = tcx.hir_crate(());
        });
        rustc_driver::Compilation::Continue
    }
}

pub fn collect_dependency_info<'tcx>(tcx: rustc_middle::ty::TyCtxt<'tcx>) -> DependencyInfo {
    let mut info = DependencyInfo::default();

    for &krate in tcx.crates(()).iter() {
        let name = tcx.crate_name(krate).to_string();
        let disambiguator = tcx.crate_hash(krate).to_hex();
        info.crates.push(DependencyCrate {
            name,
            disambiguator,
        });
    }

    for &cnum in tcx.crates(()).iter() {
        let num_defs = tcx.num_extern_def_ids(cnum);
        for idx in 0..num_defs {
            let def_id = RustcDefId {
                krate: cnum,
                index: rustc_span::def_id::DefIndex::from_usize(idx),
            };
            collect_dependency_def(tcx, def_id, &mut info);
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
        collect_dependency_def(tcx, def_id, &mut info);
    }

    info
}

fn collect_dependency_def<'tcx>(tcx: TyCtxt<'tcx>, def_id: RustcDefId, info: &mut DependencyInfo) {
    let kind = tcx.def_kind(def_id);

    if matches!(
        kind,
        DefKind::Fn | DefKind::AssocFn | DefKind::Ctor(_, CtorKind::Fn)
    ) {
        let hash = tcx.def_path_hash(def_id);
        let (hi, lo): (u64, u64) =
            unsafe { std::mem::transmute::<Fingerprint, (u64, u64)>(hash.0) };
        info.functions.push(DependencyFunction {
            path: tcx.def_path_str(def_id),
            def_path_hash_hi: hi,
            def_path_hash_lo: lo,
            fn_def: stable_fn_from_def_id(tcx, def_id),
        });
    }

    if matches!(kind, DefKind::Const | DefKind::Static { .. }) {
        let hash = tcx.def_path_hash(def_id);
        let (hi, lo): (u64, u64) =
            unsafe { std::mem::transmute::<Fingerprint, (u64, u64)>(hash.0) };
        info.values.push(DependencyValue {
            kind: match kind {
                DefKind::Const => DependencyValueKind::ConstDef(rustc_def_to_my_def(tcx, def_id)),
                DefKind::Static { .. } => {
                    DependencyValueKind::Def(rustc_def_to_my_def(tcx, def_id))
                }
                _ => unreachable!(),
            },
            path: tcx.def_path_str(def_id),
            def_path_hash_hi: hi,
            def_path_hash_lo: lo,
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

    if matches!(kind, DefKind::Trait) {
        let hash = tcx.def_path_hash(def_id);
        let (hi, lo): (u64, u64) =
            unsafe { std::mem::transmute::<Fingerprint, (u64, u64)>(hash.0) };
        info.traits.push(DependencyTrait {
            def_id: rustc_def_to_my_def(tcx, def_id),
            path: tcx.def_path_str(def_id),
            def_path_hash_hi: hi,
            def_path_hash_lo: lo,
        });
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

fn dependency_const_value<'tcx>(
    tcx: TyCtxt<'tcx>,
    def_id: RustcDefId,
) -> Option<DependencyConstValue> {
    let kind = tcx.def_kind(def_id);
    if !matches!(kind, DefKind::Const) {
        return None;
    }

    let value =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| tcx.const_eval_poly(def_id)))
            .ok()?
            .ok()?;
    let scalar = value.try_to_scalar()?;
    let ty = tcx.type_of(def_id).instantiate_identity();

    match ty.kind() {
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

pub(crate) fn dependency_const_value_for_def_id<'tcx>(
    tcx: TyCtxt<'tcx>,
    def_id: DefId,
) -> Option<DependencyConstValue> {
    dependency_const_value(tcx, my_def_id_to_rustc_def_id(tcx, def_id))
}

pub(crate) fn type_implements_trait<'tcx>(
    tcx: TyCtxt<'tcx>,
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

pub(crate) fn type_is_copy<'tcx>(tcx: TyCtxt<'tcx>, owner: DefId, ty: MirTy) -> bool {
    let copy_trait = rustc_def_to_my_def(tcx, tcx.require_lang_item(LangItem::Copy, DUMMY_SP));
    type_implements_trait(tcx, owner, ty, copy_trait)
}

pub(crate) fn normalize_ty_for_owner<'tcx>(tcx: TyCtxt<'tcx>, owner: DefId, ty: MirTy) -> MirTy {
    let owner = my_def_id_to_rustc_def_id(tcx, owner);
    let ty = mir_ty_to_rustc(tcx, &ty);
    let typing_env =
        ty::TypingEnv::non_body_analysis(tcx, owner).with_post_analysis_normalized(tcx);
    tcx.try_normalize_erasing_regions(typing_env, ty)
        .map(rustc_public::rustc_internal::stable)
        .unwrap_or_else(|_| rustc_public::rustc_internal::stable(ty))
}

pub(crate) fn normalize_ty_for_owner_with_self<'tcx>(
    tcx: TyCtxt<'tcx>,
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
    tcx.try_normalize_erasing_regions(typing_env, ty)
        .map(rustc_public::rustc_internal::stable)
        .unwrap_or_else(|_| rustc_public::rustc_internal::stable(ty))
}

pub(crate) fn normalize_ty_defaults<'tcx>(tcx: TyCtxt<'tcx>, ty: MirTy) -> MirTy {
    rustc_public::rustc_internal::stable(normalize_ty_defaults_to_rustc(tcx, ty))
}

fn normalize_ty_defaults_to_rustc<'tcx>(tcx: TyCtxt<'tcx>, ty: MirTy) -> ty::Ty<'tcx> {
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
        param
            .default_value(tcx)
            .map(|default| default.instantiate(tcx, current))
            .unwrap_or_else(|| tcx.mk_param_from_def(param))
    })
}

fn override_queries<S: CrateGeneratorState>(
    _sess: &rustc_session::Session,
    providers: &mut UtilProviders,
) {
    if std::env::var("GEN_DEBUG").is_ok() {
        eprintln!("override_queries");
    }
    if let Some(gate) = GENERATE_STATE.get() {
        override_providers::<S>(&mut providers.queries, gate.clone());
    } else if std::env::var("GEN_DEBUG").is_ok() {
        eprintln!("override_queries: no state");
    }
}

fn override_providers<S: CrateGeneratorState>(
    providers: &mut QueryProviders,
    gate: Arc<GenerateGate>,
) {
    let mut guard = gate.state.try_lock().unwrap();
    if guard.original.is_none() {
        guard.original = Some(OriginalProviders {
            hir_crate: providers.hir_crate,
            resolutions: providers.resolutions,
            def_kind: providers.def_kind,
            // def_span: providers.def_span,
            // def_ident_span: providers.def_ident_span,
            reachable_set: providers.reachable_set,
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
    // Leave hir_crate_items/hir_module_items to the original providers.
    // providers.local_def_id_to_hir_id = generated_local_def_id_to_hir_id;
    // providers.opt_hir_owner_nodes = generated_opt_hir_owner_nodes;
    // providers.hir_owner_parent_q = generated_hir_owner_parent_q;
    providers.hir_attr_map = generated_hir_attr_map;
    providers.opt_ast_lowering_delayed_lints = generated_opt_ast_lowering_delayed_lints;
    providers.entry_fn = generated_entry_fn;
    providers.def_kind = generated_def_kind;
    providers.def_span = generated_def_span;
    providers.def_ident_span = generated_def_ident_span;
    providers.visibility = generated_visibility;
    providers.reachable_set = generated_reachable_set;
    // providers.impl_parent = generated_impl_parent;
    // providers.specialization_graph_of = generated_specialization_graph_of;
    // providers.all_local_trait_impls = generated_all_local_trait_impls;
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

fn generated_resolutions<'tcx>(
    tcx: TyCtxt<'tcx>,
    (): (),
) -> &'tcx rustc_middle::ty::ResolverGlobalCtxt {
    // Avoid holding the generate mutex while calling the original provider.
    let (original, items, trait_impl_pairs) = {
        let state = GENERATE_STATE
            .get()
            .cloned()
            .expect("generate state missing");
        let guard = state.state.try_lock().unwrap();
        let original = guard.original.expect("original providers missing");
        let items = match &guard.defined_crate {
            DefinedCrateState::Stage2(defined_crate, _, _, _) => defined_crate.items.clone(),
            _ => Vec::new(),
        };
        // Collect (trait_def_id, impl_local_def_id) for generated trait impls so we
        // can register them in resolutions().trait_impls.  This is what feeds
        // local_trait_impls / all_local_trait_impls / specialization_graph_of, so
        // the generated impls become visible to those queries without overriding them.
        let trait_impl_pairs: Vec<(DefId, DefId)> = match &guard.defined_crate {
            DefinedCrateState::Stage2(_, signatures, _, _) => signatures
                .iter()
                .filter_map(|item| match &item.kind {
                    ItemSignatureKind::Impl {
                        trait_def: Some(trait_def),
                        ..
                    } => Some((*trait_def, item.id)),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        (original, items, trait_impl_pairs)
    };

    let r = (original.resolutions)(tcx, ());
    if items.is_empty() {
        return r;
    }

    let mut module_children: HashMap<LocalDefId, Vec<rustc_middle::metadata::ModChild>> =
        HashMap::new();
    for item in &items {
        let Some(local_def_id) = my_def_id_to_rustc_def_id(tcx, item.def_id()).as_local() else {
            continue;
        };
        let res = match item.kind {
            DefinedItemKind::Function { .. } => Res::Def(DefKind::Fn, local_def_id.to_def_id()),
            DefinedItemKind::Struct(_, _) => Res::Def(DefKind::Struct, local_def_id.to_def_id()),
            DefinedItemKind::Union(_, _) => Res::Def(DefKind::Union, local_def_id.to_def_id()),
            DefinedItemKind::TypeDef(_) => Res::Def(DefKind::TyAlias, local_def_id.to_def_id()),
            DefinedItemKind::Module(_) => Res::Def(DefKind::Mod, local_def_id.to_def_id()),
            DefinedItemKind::Static(_) => Res::Def(
                DefKind::Static {
                    safety: rustc_hir::Safety::Safe,
                    mutability: ty::Mutability::Mut,
                    nested: false,
                },
                local_def_id.to_def_id(),
            ),
            DefinedItemKind::ForeignFunction(_) => Res::Def(DefKind::Fn, local_def_id.to_def_id()),
            DefinedItemKind::Impl { .. } | DefinedItemKind::ImplItemFn(_) => continue,
            _ => continue,
        };
        let child = rustc_middle::metadata::ModChild {
            ident: Ident::from_str(&item.name),
            res,
            vis: ty::Visibility::Public,
            reexport_chain: Default::default(),
        };
        let parent = item
            .parent
            .and_then(|parent| my_def_id_to_rustc_def_id(tcx, parent).as_local())
            .unwrap_or(CRATE_DEF_ID);
        module_children.entry(parent).or_default().push(child);
    }

    unsafe {
        let r_ptr = r as *const rustc_middle::ty::ResolverGlobalCtxt
            as *mut rustc_middle::ty::ResolverGlobalCtxt;
        for (module, children) in module_children {
            (*r_ptr).module_children.insert(module, children);
        }
        (*r_ptr)
            .effective_visibilities
            .public_at_level(CRATE_DEF_ID);
        for item in &items {
            let Some(local_def_id) = my_def_id_to_rustc_def_id(tcx, item.def_id()).as_local()
            else {
                continue;
            };
            (*r_ptr)
                .effective_visibilities
                .public_at_level(local_def_id);
        }
        // Register generated trait impls into resolutions().trait_impls so that
        // local_trait_impls / all_local_trait_impls / specialization_graph_of all
        // see them without any additional query overrides.
        for (trait_def, impl_def) in &trait_impl_pairs {
            let trait_rustc = my_def_id_to_rustc_def_id(tcx, *trait_def);
            let impl_local = my_def_id_to_rustc_def_id(tcx, *impl_def)
                .as_local()
                .expect("generated impl must be local");
            (*r_ptr)
                .trait_impls
                .entry(trait_rustc)
                .or_default()
                .push(impl_local);
        }
    }

    r
}

fn generated_hir_attr_map<'tcx>(tcx: TyCtxt<'tcx>, key: OwnerId) -> &'tcx hir::AttributeMap<'tcx> {
    with_generated_and_original(tcx, |generated, _original| {
        let DefinedCrateState::Stage2(items, _, _, _) = generated else {
            return hir::AttributeMap::EMPTY;
        };
        let key = key.to_def_id();
        if key.is_crate_root() {
            return hir::AttributeMap::EMPTY;
        }
        let key = rustc_def_to_my_def(tcx, key);
        let Some(info) = items.items.iter().find(|item| item.def_id() == key) else {
            return hir::AttributeMap::EMPTY;
        };
        let Some(attrs) = generated_item_attrs(info.kind) else {
            return hir::AttributeMap::EMPTY;
        };
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

fn generated_item_attrs(kind: DefinedItemKind) -> Option<Vec<hir::Attribute>> {
    let attr = match kind {
        DefinedItemKind::Function {
            abi: FunctionAbi::C,
            no_mangle: true,
            ..
        } => hir::attrs::AttributeKind::NoMangle(DUMMY_SP),
        DefinedItemKind::Struct(_, repr) | DefinedItemKind::Union(_, repr) => {
            let repr = match repr {
                AdtRepr::Rust => hir::attrs::ReprAttr::ReprRust,
                AdtRepr::C => hir::attrs::ReprAttr::ReprC,
            };
            hir::attrs::AttributeKind::Repr {
                reprs: ThinVec::from_iter([(repr, DUMMY_SP)]),
                first_span: DUMMY_SP,
            }
        }
        _ => return None,
    };
    Some(vec![hir::Attribute::Parsed(attr)])
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
    with_generated_and_original(tcx, |generated, _| generated.entry_fn(tcx, key))
}

fn generated_def_kind<'tcx>(tcx: TyCtxt<'tcx>, key: LocalDefId) -> DefKind {
    if std::env::var("GEN_DEBUG").is_ok() {
        eprintln!("generated_def_kind {:?}", key);
    }
    with_generated_and_original(tcx, |generated, original| {
        if let Some(kind) = generated.def_kind(tcx, key) {
            return kind;
        }
        // Lifetime params allocated inside generated items won't be in DefinedItemInfo,
        // but their DefKey will have LifetimeNs disambiguator.
        use rustc_hir::definitions::DefPathData;
        if matches!(
            tcx.def_key(key).disambiguated_data.data,
            DefPathData::LifetimeNs(_)
        ) {
            return DefKind::LifetimeParam;
        }
        (original.def_kind)(tcx, key)
    })
}

fn generated_def_span<'tcx>(tcx: TyCtxt<'tcx>, key: LocalDefId) -> RustcSpan {
    with_generated_and_original(tcx, |generated, _original| {
        if let Some(span) = generated.def_span(tcx, key) {
            return span;
        }

        DUMMY_SP
        // (original.def_span)(tcx, key)
    })
}

fn generated_def_ident_span<'tcx>(tcx: TyCtxt<'tcx>, key: LocalDefId) -> Option<RustcSpan> {
    with_generated_and_original(tcx, |generated, _original| {
        if let Some(span) = generated.def_span(tcx, key) {
            return Some(span);
        }

        None
        // (original.def_ident_span)(tcx, key)
    })
}

fn generated_visibility<'tcx>(_tcx: TyCtxt<'tcx>, _key: LocalDefId) -> ty::Visibility<RustcDefId> {
    ty::Visibility::Public
}

fn generated_reachable_set<'tcx>(
    tcx: TyCtxt<'tcx>,
    (): (),
) -> rustc_data_structures::unord::UnordSet<LocalDefId> {
    with_generated_and_original(tcx, |generated, original| {
        let mut reachable = (original.reachable_set)(tcx, ());
        let DefinedCrateState::Stage2(defined_crate, _, _, _) = generated else {
            return reachable;
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

fn generated_mir_built<'tcx, S: CrateGeneratorState>(
    tcx: TyCtxt<'tcx>,
    def_id: LocalDefId,
) -> &'tcx Steal<rustc_middle::mir::Body<'tcx>> {
    let key = rustc_def_to_my_def(tcx, def_id.to_def_id());

    let mir = {
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

    let body = build_mir_body(tcx, &mir, def_id);

    unsafe { std::mem::transmute(leak(Steal::new(body))) }
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
) -> rustc_middle::mir::Body<'static> {
    let source_scope = rustc_middle::mir::SourceScope::from_usize(0);
    let source_scopes = IndexVec::from_iter([rustc_middle::mir::SourceScopeData {
        span: rustc_public::rustc_internal::internal(tcx, body.span),
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
                span: rustc_public::rustc_internal::internal(tcx, local.span),
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
                            span: rustc_public::rustc_internal::internal(tcx, stmt.span),
                            scope: source_scope,
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
                scope: source_scope,
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
                    let args: Box<
                        [rustc_span::source_map::Spanned<rustc_middle::mir::Operand<'tcx>>],
                    > = args
                        .iter()
                        .map(|arg| rustc_span::source_map::Spanned {
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
    let body = rustc_middle::mir::Body::new(
        rustc_middle::mir::MirSource::item(owner.to_def_id()),
        basic_blocks,
        source_scopes,
        local_decls,
        IndexVec::new(),
        body.arg_locals().len(),
        Vec::new(),
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
        MirRvalue::Use(op) => rustc_middle::mir::Rvalue::Use(mir_operand_to_rustc(tcx, op)),
        MirRvalue::ThreadLocalRef(item) => {
            let def_id = internal(tcx, *item);
            if tcx.is_thread_local_static(def_id) {
                rustc_middle::mir::Rvalue::ThreadLocalRef(def_id)
            } else {
                let alloc_id = tcx.reserve_and_set_static_alloc(def_id);
                let ptr = Pointer::new(CtfeProvenance::from(alloc_id), rustc_abi::Size::ZERO);
                let scalar = Scalar::from_pointer(ptr, &tcx);
                let ty = tcx.type_of(def_id).instantiate_identity();
                let ptr_ty = rustc_middle::ty::Ty::new_mut_ptr(tcx, ty);
                let const_ = rustc_middle::mir::Const::Val(ConstValue::Scalar(scalar), ptr_ty);
                let op = rustc_middle::mir::Operand::Constant(Box::new(
                    rustc_middle::mir::ConstOperand {
                        span: DUMMY_SP,
                        user_ty: None,
                        const_,
                    },
                ));
                rustc_middle::mir::Rvalue::Use(op)
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
        delayed_lints: hir::lints::DelayedLints {
            lints: Vec::new().into_boxed_slice(),
            opt_hash: Some(Fingerprint::ZERO),
        },
    }
}

fn make_owner_info(nodes: hir::OwnerNodes<'static>) -> hir::OwnerInfo<'static> {
    make_owner_info_with_attrs(nodes, None)
}

fn make_def_path<'tcx>(
    tcx: TyCtxt<'tcx>,
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

fn make_array_ty<'tcx>(
    tcx: TyCtxt<'tcx>,
    owner: LocalDefId,
    pointee: &'static hir::Ty<'static>,
    len: HirTyConst,
) -> hir::Ty<'static> {
    hir::Ty {
        hir_id: HirId::make_owner(owner),
        span: DUMMY_SP,
        kind: hir::TyKind::Array(
            pointee,
            leak(hir::ConstArg {
                hir_id: HirId::make_owner(owner),
                span: DUMMY_SP,
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

fn make_lifetime<'tcx>(
    tcx: TyCtxt<'tcx>,
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
fn build_fn_generics<'tcx>(
    tcx: TyCtxt<'tcx>,
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
            let param = hir::GenericParam {
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
            };
            param
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

fn make_unit_ty(owner: LocalDefId) -> hir::Ty<'static> {
    let empty: &'static [hir::Ty<'static>] = leak(Vec::new().into_boxed_slice());
    hir::Ty {
        hir_id: HirId::make_owner(owner),
        span: DUMMY_SP,
        kind: hir::TyKind::Tup(empty),
    }
}

fn make_def_id_qpath<'tcx>(
    tcx: TyCtxt<'tcx>,
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

fn make_adt_ty<'tcx>(
    tcx: TyCtxt<'tcx>,
    owner: LocalDefId,
    adt: DefId,
    args: &[crate::HirGenericArg],
    item_allocator: &mut HirItemAllocator,
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
            };
        }
        let generic_args = leak(hir::GenericArgs {
            args: leak(hir_args.into_boxed_slice()),
            constraints: &[],
            parenthesized: hir::GenericArgsParentheses::No,
            span_ext: DUMMY_SP,
        });
        segment.args = Some(generic_args);
        segment.infer_args = false;
    }
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

fn hir_ty_to_rustc<'tcx>(
    tcx: TyCtxt<'tcx>,
    owner: LocalDefId,
    ty: &HirTy,
    item_allocator: &mut HirItemAllocator,
) -> hir::Ty<'static> {
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
        HirTyKind::Bool => make_prim_ty(owner, hir::PrimTy::Bool),
        HirTyKind::Char => make_prim_ty(owner, hir::PrimTy::Char),
        HirTyKind::Float(float_ty) => {
            let float_ty = match float_ty {
                rustc_public::ty::FloatTy::F16 => FloatTy::F16,
                rustc_public::ty::FloatTy::F32 => FloatTy::F32,
                rustc_public::ty::FloatTy::F64 => FloatTy::F64,
                rustc_public::ty::FloatTy::F128 => FloatTy::F128,
            };
            make_prim_ty(owner, hir::PrimTy::Float(float_ty))
        }
        HirTyKind::RawPtr(mutability, to) => {
            let pointee = leak(hir_ty_to_rustc(tcx, owner, &to, item_allocator));
            make_ptr_ty(
                owner,
                pointee,
                match mutability {
                    rustc_public::mir::Mutability::Not => hir::Mutability::Not,
                    rustc_public::mir::Mutability::Mut => hir::Mutability::Mut,
                },
            )
        }
        HirTyKind::Array(len, to) => {
            let pointee = leak(hir_ty_to_rustc(tcx, owner, &to, item_allocator));
            make_array_ty(tcx, owner, pointee, *len)
        }
        HirTyKind::Ref(mutability, lifetime, to) => {
            let pointee = leak(hir_ty_to_rustc(tcx, owner, &to, item_allocator));
            let lifetime = make_lifetime(tcx, lifetime, item_allocator);
            hir::Ty {
                hir_id: HirId::make_owner(owner),
                span: DUMMY_SP,
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
        HirTyKind::Adt(adt, args) => make_adt_ty(tcx, owner, *adt, args, item_allocator),
        HirTyKind::Tuple(elems) => {
            if elems.is_empty() {
                make_unit_ty(owner)
            } else {
                todo!()
            }
        }
        HirTyKind::FnPtr(sig) => {
            let hir_id = item_allocator.new_item();
            let fn_decl = leak(hir::FnDecl {
                inputs: leak(
                    sig.inputs
                        .iter()
                        .map(|ty| hir_ty_to_rustc(tcx, owner, ty, item_allocator))
                        .collect::<Vec<_>>(),
                ),
                output: hir::FnRetTy::Return(leak(hir_ty_to_rustc(
                    tcx,
                    owner,
                    &sig.output,
                    item_allocator,
                ))),
                c_variadic: sig.c_variadic,
                implicit_self: hir::ImplicitSelfKind::None,
                lifetime_elision_allowed: true,
            });
            let param_idents = leak(vec![None; sig.inputs.len()]);
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
                span: DUMMY_SP,
            });
            item_allocator.set_node(hir_id.local_id, hir::Node::Ty(ty), ItemLocalId::ZERO);
            *ty
        }
        HirTyKind::Never => {
            let hir_id = item_allocator.new_item();
            let ty = leak(hir::Ty {
                kind: hir::TyKind::Never,
                span: DUMMY_SP,
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
