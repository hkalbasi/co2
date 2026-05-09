#![feature(rustc_private)]

extern crate rustc_driver;
extern crate rustc_middle;

use std::collections::{BTreeMap, HashMap};
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::{Mutex, OnceLock};

use co2_ast::Initializer;
use co2_crate_sig::{LocalResolver, MirOwnerInfo, WellknownDefs};
use co2_hir::{
    HirBody, HirCtx, HirExpr, HirExprKind, HirLocal, HirStmt, ResolvedValue,
    eval_usize_initializer, infer_array_len_from_initializer, lower_static_body_for_ty,
};
use co2_preprocessor::PreprocessedSource;
use la_arena::Arena;
use rustc_public_generative::rustc_public::ty::IntTy;
use rustc_public_generative::rustc_public::{
    CrateDefType, CrateItem, DefId,
    mir::{
        BasicBlock, Body, ConstOperand, LocalDecl, Mutability, Operand, Rvalue, Statement,
        StatementKind, Terminator, TerminatorKind,
    },
    ty::{
        AdtDef, FnDef, GenericArgKind, GenericArgs, MirConst, Region, RegionKind, RigidTy, Ty,
        TyKind,
    },
};
use rustc_public_generative::{self as rustc_gen, HirStructureCtx};

mod types;

pub use types::CompileMode;

struct PendingCompile {
    mode: CompileMode,
    source_path: PathBuf,
    preprocessed: Arc<PreprocessedSource>,
}

struct Co2SourceMap {
    files: Arc<HashMap<co2_ast::FileId, (String, Arc<str>)>>,
}

fn pending_compile_cell() -> &'static Mutex<Option<PendingCompile>> {
    static CELL: OnceLock<Mutex<Option<PendingCompile>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(None))
}
struct Co2GeneratorState {
    file_id: rustc_gen::FileId,
    file_ids: Arc<HashMap<co2_ast::FileId, rustc_gen::FileId>>,
    pending_mirs: HashMap<DefId, MirOwnerInfo>,
    wellknown_defs: WellknownDefs,
}

unsafe impl Send for Co2GeneratorState {}
unsafe impl Sync for Co2GeneratorState {}

impl co2_ast::SourceMap for Co2SourceMap {
    fn get_file_info(&self, id: co2_ast::FileId) -> Option<(String, Arc<str>)> {
        self.files.get(&id).cloned()
    }
}

impl rustc_gen::CrateGeneratorState for Co2GeneratorState {
    fn hir_structure(ctx: rustc_gen::HirStructureCtx) -> (Self, rustc_gen::HirStructure) {
        let pending = pending_compile_cell()
            .try_lock()
            .unwrap()
            .take()
            .expect("missing pending compile input");

        let mut file_ids = HashMap::with_capacity(pending.preprocessed.files().len());
        let mut source_files = HashMap::with_capacity(pending.preprocessed.files().len());
        for (co2_file_id, file) in pending.preprocessed.files() {
            file_ids.insert(
                *co2_file_id,
                ctx.add_custom_file(&file.path, file.source.as_ref()),
            );
            source_files.insert(
                *co2_file_id,
                (file.path.display().to_string(), file.source.clone()),
            );
        }
        let file_id = file_ids[&pending.preprocessed.main_file_idx];
        let source_name = pending.source_path.to_string_lossy().into_owned();
        let src_static: &'static str =
            Box::leak(pending.preprocessed.normalized.to_string().into_boxed_str());
        co2_ast::set_source_map(Arc::new(Co2SourceMap {
            files: Arc::new(source_files.clone()),
        }));

        let (result, pending_mirs, wellknown_defs) = co2_crate_sig::lower_crate_sig(
            ctx,
            pending.source_path.clone(),
            source_name.clone(),
            src_static,
            file_id,
            pending.preprocessed.clone(),
            &mut file_ids,
            &mut source_files,
            pending.mode.no_main,
        );
        let file_ids = Arc::new(file_ids);
        co2_ast::set_source_map(Arc::new(Co2SourceMap {
            files: Arc::new(source_files),
        }));

        // If co2 diagnostics were emitted during lowering, abort now before
        // passing the (possibly broken) HIR to rustc.  This ensures a clean
        // DiagnosticAbort (exit code 5) rather than a non-string rustc panic
        // (exit code 101) that would otherwise occur when rustc encounters
        // problems such as duplicate `no_mangle` symbols.
        if co2_ast::diagnostics_were_emitted() {
            co2_ast::panic_with_diagnostic_abort();
        }

        let state = Co2GeneratorState {
            wellknown_defs,
            file_id,
            file_ids,
            pending_mirs,
        };

        (state, result)
    }

    fn emit_mir(&mut self, ctx: rustc_gen::HirStructureCtx, def: DefId) -> Body {
        let pending_mir = self.pending_mirs.remove(&def).unwrap();

        match pending_mir {
            MirOwnerInfo::CloneMethod(adt) => {
                build_clone_method_body(adt, ctx.span_in_file(self.file_id, 0, 0))
            }
            MirOwnerInfo::Const => {
                // Const items don't have bodies; return a placeholder
                build_zeroed_static_initializer_body(
                    &self.wellknown_defs,
                    CrateItem(def).ty(),
                    ctx.span_in_file(self.file_id, 0, 0),
                )
            }
            MirOwnerInfo::EnumConstPrevPlus(prev, span) => {
                self.build_enum_prev_plus_body(prev, span, &ctx)
            }
            MirOwnerInfo::EnumConstExplicit {
                initializer,
                resolver,
            } => {
                let span = initializer.1;
                self.lower_explicit_static_mir(
                    &ctx,
                    def,
                    resolver,
                    (Initializer::Expr(initializer), span),
                )
            }
            MirOwnerInfo::Static {
                resolver,
                initializer,
            } => self.lower_explicit_static_mir(&ctx, def, resolver, initializer),
            MirOwnerInfo::StaticWithArrayLen {
                resolver,
                initializer,
                array_len,
            } => self.lower_explicit_static_mir_with_array_len(
                &ctx,
                def,
                resolver,
                initializer,
                array_len,
            ),
            MirOwnerInfo::StaticZeroed | MirOwnerInfo::EnumConstZeroed => {
                build_zeroed_static_initializer_body(
                    &self.wellknown_defs,
                    CrateItem(def).ty(),
                    ctx.span_in_file(self.file_id, 0, 0),
                )
            }
            MirOwnerInfo::Fn {
                def,
                function_name,
                param_names,
                resolver,
                body,
            } => {
                let span_converter = |span: co2_ast::Span| self.map_co2_span(&ctx, span);
                let mut hir_ctx = HirCtx::new(
                    self.wellknown_defs,
                    &span_converter,
                    Some(function_name),
                    def.fn_sig().skip_binder().output(),
                    resolver,
                );

                let hir = match std::panic::catch_unwind(AssertUnwindSafe(|| {
                    co2_hir::lower_function_body(body.clone(), def, &param_names, &mut hir_ctx)
                        .unwrap_or_else(|err| {
                            co2_ast::emit_errors_and_terminate(vec![co2_ast::Rich::custom(
                                body.1, err,
                            )])
                        })
                })) {
                    Ok(hir) => hir,
                    Err(payload) => {
                        if co2_ast::is_diagnostic_abort(payload.as_ref()) {
                            let hir = build_error_fn_body(
                                &self.wellknown_defs,
                                &def.fn_sig().skip_binder(),
                                ctx.span_in_file(self.file_id, 0, 0),
                            );
                            return co2_mir::build_mir_for_body(
                                &hir,
                                &ctx,
                                def.0,
                                self.file_id,
                                self.wellknown_defs,
                            );
                        }
                        std::panic::resume_unwind(payload);
                    }
                };

                co2_mir::build_mir_for_body(&hir, &ctx, def.0, self.file_id, self.wellknown_defs)
            }
            MirOwnerInfo::FnBodyError { def, body_span } => {
                let hir = build_error_fn_body(
                    &self.wellknown_defs,
                    &def.fn_sig().skip_binder(),
                    self.map_co2_span(&ctx, body_span),
                );
                co2_mir::build_mir_for_body(&hir, &ctx, def.0, self.file_id, self.wellknown_defs)
            }
        }
    }
}

impl Co2GeneratorState {
    fn map_co2_span(
        &self,
        ctx: &HirStructureCtx<'_>,
        span: co2_ast::Span,
    ) -> rustc_public_generative::rustc_public::ty::Span {
        let file_id = self.file_ids[&span.context];
        ctx.span_in_file(file_id, span.start as u32, span.end as u32)
    }

    fn lower_explicit_static_mir(
        &mut self,
        ctx: &HirStructureCtx<'_>,
        def: DefId,
        resolver: LocalResolver,
        initializer: co2_ast::Spanned<Initializer<LocalResolver>>,
    ) -> Body {
        let span_converter = |span: co2_ast::Span| self.map_co2_span(ctx, span);
        let hir_ctx = HirCtx::new(
            self.wellknown_defs,
            &span_converter,
            None,
            CrateItem(def).ty(),
            resolver,
        );

        let mut target_ty = CrateItem(def).ty();
        if let TyKind::RigidTy(RigidTy::Array(elem_ty, len)) = target_ty.kind() {
            if len.eval_target_usize().is_err() {
                let inferred_len =
                    infer_array_len_from_initializer(initializer.clone(), elem_ty, &hir_ctx)
                        .expect("failed to infer static array length");
                target_ty = Ty::from_rigid_kind(RigidTy::Array(
                    elem_ty,
                    rustc_public_generative::rustc_public::ty::TyConst::try_from_target_usize(
                        inferred_len,
                    )
                    .expect("failed to materialize inferred array length"),
                ));
            }
        }

        let hir = lower_static_body_for_ty(initializer, target_ty, &hir_ctx).unwrap();

        co2_mir::build_mir_for_body(&hir, ctx, def, self.file_id, self.wellknown_defs)
    }

    fn lower_explicit_static_mir_with_array_len(
        &mut self,
        ctx: &HirStructureCtx<'_>,
        def: DefId,
        resolver: LocalResolver,
        initializer: co2_ast::Spanned<Initializer<LocalResolver>>,
        array_len: co2_ast::Spanned<Initializer<LocalResolver>>,
    ) -> Body {
        let mut target_ty = CrateItem(def).ty();
        if let TyKind::RigidTy(RigidTy::Array(elem_ty, len)) = target_ty.kind() {
            if len.eval_target_usize().is_err() {
                let len_span_converter = |span: co2_ast::Span| self.map_co2_span(ctx, span);
                let len_hir_ctx = HirCtx::new(
                    self.wellknown_defs,
                    &len_span_converter,
                    None,
                    Ty::usize_ty(),
                    resolver.clone(),
                );
                let evaluated_len = eval_usize_initializer(array_len, &len_hir_ctx)
                    .expect("failed to evaluate static array length");
                target_ty = Ty::from_rigid_kind(RigidTy::Array(
                    elem_ty,
                    rustc_public_generative::rustc_public::ty::TyConst::try_from_target_usize(
                        evaluated_len,
                    )
                    .expect("failed to materialize static array length"),
                ));
            }
        }
        let span_converter = |span: co2_ast::Span| self.map_co2_span(ctx, span);
        let hir_ctx = HirCtx::new(
            self.wellknown_defs,
            &span_converter,
            None,
            target_ty,
            resolver,
        );
        let hir = lower_static_body_for_ty(initializer, target_ty, &hir_ctx).unwrap();
        co2_mir::build_mir_for_body(&hir, ctx, def, self.file_id, self.wellknown_defs)
    }

    fn build_enum_prev_plus_body(
        &self,
        prev: DefId,
        span: rustc_public_generative::rustc_public::ty::Span,
        ctx: &HirStructureCtx<'_>,
    ) -> Body {
        let i32_ty = Ty::signed_ty(IntTy::I32);
        let mut hir = HirBody::new_dummy(i32_ty, span);
        hir.stmts = vec![HirStmt::Return(
            Some(HirExpr {
                kind: co2_hir::HirExprKind::Binary {
                    op: co2_hir::HirBinOp::Add,
                    lhs: Box::new(HirExpr {
                        kind: co2_hir::HirExprKind::Path(ResolvedValue::Static(prev)),
                        ty: i32_ty,
                        span,
                    }),
                    rhs: Box::new(HirExpr {
                        kind: co2_hir::HirExprKind::ConstInt(1),
                        ty: i32_ty,
                        span,
                    }),
                },
                ty: i32_ty,
                span,
            }),
            span,
        )];

        co2_mir::build_mir_for_body(&hir, ctx, prev, self.file_id, self.wellknown_defs)
    }
}

fn fn_const_operand(
    fn_def: FnDef,
    generic_args: Vec<GenericArgKind>,
    span: rustc_public_generative::rustc_public::ty::Span,
) -> Operand {
    let fn_ty = Ty::from_rigid_kind(RigidTy::FnDef(fn_def, GenericArgs(generic_args)));
    let c = MirConst::try_new_zero_sized(fn_ty).expect("failed to build fn const");
    Operand::Constant(ConstOperand {
        span,
        user_ty: None,
        const_: c,
    })
}

fn infer_fn_generic_args_for_return(
    sig: &rustc_public_generative::rustc_public::ty::FnSig,
    ret_ty: Ty,
) -> Vec<GenericArgKind> {
    let mut by_index: BTreeMap<u32, Ty> = BTreeMap::new();
    collect_param_bindings(sig.output(), ret_ty, &mut by_index);
    by_index
        .into_values()
        .map(GenericArgKind::Type)
        .collect::<Vec<_>>()
}

fn collect_param_bindings(expected: Ty, actual: Ty, out: &mut BTreeMap<u32, Ty>) {
    match (expected.kind(), actual.kind()) {
        (TyKind::Param(param), _) => {
            out.entry(param.index).or_insert(actual);
        }
        (TyKind::RigidTy(RigidTy::Ref(_, expected_inner, _)), _) => {
            collect_param_bindings(expected_inner, actual, out);
        }
        (
            TyKind::RigidTy(RigidTy::Adt(expected_adt, expected_args)),
            TyKind::RigidTy(RigidTy::Adt(actual_adt, actual_args)),
        ) if expected_adt == actual_adt && expected_args.0.len() == actual_args.0.len() => {
            for (e, a) in expected_args.0.iter().zip(actual_args.0.iter()) {
                if let (GenericArgKind::Type(et), GenericArgKind::Type(at)) = (e, a) {
                    collect_param_bindings(*et, *at, out);
                }
            }
        }
        _ => {}
    }
}

fn build_zeroed_static_initializer_body(
    wellknown_defs: &WellknownDefs,
    ty: Ty,
    span: rustc_public_generative::rustc_public::ty::Span,
) -> Body {
    let zeroed_fn = wellknown_defs.zeroed;
    let sig = zeroed_fn
        .ty()
        .kind()
        .fn_sig()
        .expect("std::mem::zeroed has no signature")
        .skip_binder();
    let generic_args = infer_fn_generic_args_for_return(&sig, ty);
    let locals = vec![LocalDecl {
        ty,
        span,
        mutability: Mutability::Mut,
    }];
    let call_block = BasicBlock {
        statements: vec![],
        terminator: Terminator {
            kind: TerminatorKind::Call {
                func: fn_const_operand(zeroed_fn, generic_args, span),
                args: vec![],
                destination: rustc_public_generative::rustc_public::mir::Place {
                    local: 0,
                    projection: vec![],
                },
                target: Some(1),
                unwind: rustc_public_generative::rustc_public::mir::UnwindAction::Continue,
            },
            span,
        },
    };
    let return_block = BasicBlock {
        statements: vec![],
        terminator: Terminator {
            kind: TerminatorKind::Return,
            span,
        },
    };
    Body::new(
        vec![call_block, return_block],
        locals,
        0,
        vec![],
        None,
        span,
    )
}

fn build_error_fn_body(
    wellknown_defs: &WellknownDefs,
    sig: &rustc_public_generative::rustc_public::ty::FnSig,
    span: rustc_public_generative::rustc_public::ty::Span,
) -> HirBody {
    let mut locals = Arena::new();
    locals.alloc(HirLocal {
        name: "_ret".to_owned(),
        ty: sig.output(),
        span,
        read_only: false,
    });

    let mut params = Vec::new();
    for (idx, ty) in sig.inputs().iter().enumerate() {
        let id = locals.alloc(HirLocal {
            name: format!("_arg{idx}"),
            ty: *ty,
            span,
            read_only: false,
        });
        params.push(id);
    }

    let mut c_variadic_local = None;
    if sig.c_variadic {
        let id = locals.alloc(HirLocal {
            name: "__co2_c_vararg".to_owned(),
            ty: Ty::from_rigid_kind(RigidTy::Adt(
                wellknown_defs.valist,
                GenericArgs(vec![GenericArgKind::Lifetime(Region {
                    kind: RegionKind::ReErased,
                })]),
            )),
            span,
            read_only: false,
        });
        params.push(id);
        c_variadic_local = Some(id);
    }

    let stmts = if matches!(sig.output().kind(), TyKind::RigidTy(RigidTy::Never)) {
        vec![]
    } else {
        vec![HirStmt::Return(
            Some(HirExpr {
                kind: HirExprKind::Zeroed,
                ty: sig.output(),
                span,
            }),
            span,
        )]
    };

    HirBody {
        locals,
        labels: Arena::new(),
        params,
        c_variadic_local,
        stmts,
        span,
    }
}

fn build_clone_method_body(
    adt: AdtDef,
    span: rustc_public_generative::rustc_public::ty::Span,
) -> Body {
    let self_ty = CrateItem(adt.0).ty();
    let region = Region {
        kind: RegionKind::ReErased,
    };
    let arg_ty = Ty::new_ref(region, self_ty, Mutability::Not);

    let locals = vec![
        LocalDecl {
            ty: self_ty,
            span,
            mutability: Mutability::Mut,
        },
        LocalDecl {
            ty: arg_ty,
            span,
            mutability: Mutability::Not,
        },
    ];

    let mut projection = Vec::new();
    projection.push(rustc_public_generative::rustc_public::mir::ProjectionElem::Deref);
    let deref_place = rustc_public_generative::rustc_public::mir::Place {
        local: 1,
        projection,
    };
    let return_place = rustc_public_generative::rustc_public::mir::Place {
        local: 0,
        projection: vec![],
    };
    let statements = vec![Statement {
        kind: StatementKind::Assign(
            return_place.clone(),
            Rvalue::Use(Operand::Copy(deref_place)),
        ),
        span,
    }];

    let return_block = BasicBlock {
        statements,
        terminator: Terminator {
            kind: TerminatorKind::Return,
            span,
        },
    };
    Body::new(vec![return_block], locals, 1, vec![], None, span)
}

pub fn compile_co2_file(mode: CompileMode, co2_file: &Path, rustc_args: Vec<String>) {
    let preprocessed = Arc::new(co2_preprocessor::preprocess(co2_file, &Vec::new()));
    compile_co2_source(mode, co2_file.to_path_buf(), preprocessed, rustc_args);
}

pub fn compile_co2_file_for_miri(
    co2_file: &Path,
    rustc_args: Vec<String>,
    after_analysis: Box<
        dyn for<'tcx> FnOnce(rustc_middle::ty::TyCtxt<'tcx>) -> rustc_driver::Compilation + Send,
    >,
) {
    let preprocessed = Arc::new(co2_preprocessor::preprocess(co2_file, &Vec::new()));
    co2_ast::reset_diagnostic_state();
    *pending_compile_cell().try_lock().unwrap() = Some(PendingCompile {
        mode: CompileMode::RUST,
        source_path: co2_file.to_path_buf(),
        preprocessed,
    });
    rustc_gen::generate_with_args_and_after_analysis::<Co2GeneratorState>(
        rustc_args,
        after_analysis,
    );
    if co2_ast::diagnostics_were_emitted() {
        co2_ast::panic_with_diagnostic_abort();
    }
}

pub fn compile_co2_source(
    mode: CompileMode,
    source_path: PathBuf,
    preprocessed: Arc<PreprocessedSource>,
    rustc_args: Vec<String>,
) {
    co2_ast::reset_diagnostic_state();
    *pending_compile_cell().try_lock().unwrap() = Some(PendingCompile {
        mode,
        source_path,
        preprocessed,
    });

    rustc_gen::generate_with_args::<Co2GeneratorState>(rustc_args);
    if co2_ast::diagnostics_were_emitted() {
        co2_ast::panic_with_diagnostic_abort();
    }
}
