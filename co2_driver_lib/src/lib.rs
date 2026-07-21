#![feature(rustc_private)]

extern crate rustc_ast;
extern crate rustc_driver;
extern crate rustc_interface;
extern crate rustc_middle;

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

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
        BasicBlock, Body, ConstOperand, LocalDecl, Mutability, Operand, Rvalue, SourceInfo,
        Statement, StatementKind, Terminator, TerminatorKind, WithRetag,
    },
    ty::{
        AdtDef, FnDef, GenericArgKind, GenericArgs, MirConst, Region, RegionKind, RigidTy, Ty,
        TyKind,
    },
};
use rustc_public_generative::{self as rustc_gen, HirStructureCtx};

pub mod time_report;
mod types;

pub use time_report::PhaseTiming;
pub use types::CompileMode;

struct PendingCompile {
    mode: CompileMode,
    source_path: PathBuf,
    preprocessed: Arc<PreprocessedSource>,
}

pub struct Co2RustdocCallbacks {
    inner: rustc_gen::InterfaceCallbacks<Co2GeneratorState>,
}

struct Co2SourceMap {
    files: Arc<HashMap<co2_ast::FileId, (String, Arc<str>)>>,
}

fn pending_compile_cell() -> &'static Mutex<Option<PendingCompile>> {
    static CELL: OnceLock<Mutex<Option<PendingCompile>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(None))
}

static DUMP_MIR_ENABLED: AtomicBool = AtomicBool::new(false);

struct Co2GeneratorState {
    file_id: rustc_gen::FileId,
    file_ids: Arc<HashMap<co2_ast::FileId, rustc_gen::FileId>>,
    reverse_file_ids: Arc<HashMap<rustc_gen::FileId, co2_ast::FileId>>,
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
    fn force_no_main_attr() -> bool {
        pending_compile_cell()
            .try_lock()
            .unwrap()
            .as_ref()
            .is_some_and(|pending| pending.mode.no_main)
    }

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
            Box::leak(pending.preprocessed.raw_src.to_string().into_boxed_str());
        co2_ast::set_source_map(Arc::new(Co2SourceMap {
            files: Arc::new(source_files.clone()),
        }));

        let mut on_parse: Option<&mut dyn FnMut(std::time::Duration)> = None;
        let mut record_parse = |d: std::time::Duration| time_report::record_parse(d);
        let mut on_body: Option<&mut dyn FnMut(std::time::Duration)> = None;
        let mut record_body = |d: std::time::Duration| time_report::accumulate_body_parse(d);
        if time_report::timing_enabled() {
            on_parse = Some(&mut record_parse);
            on_body = Some(&mut record_body);
        }

        let (result, pending_mirs, wellknown_defs) = co2_crate_sig::lower_crate_sig(
            ctx,
            &pending.source_path,
            &source_name,
            src_static,
            file_id,
            &pending.preprocessed,
            &mut file_ids,
            &mut source_files,
            pending.mode.no_main,
            pending.mode.test,
            on_parse,
            on_body,
        );
        let file_ids = Arc::new(file_ids);
        co2_ast::set_source_map(Arc::new(Co2SourceMap {
            files: Arc::new(source_files),
        }));

        let reverse_file_ids: HashMap<rustc_gen::FileId, co2_ast::FileId> =
            file_ids.iter().map(|(&k, &v)| (v, k)).collect();
        let reverse_file_ids = Arc::new(reverse_file_ids);

        let state = Co2GeneratorState {
            file_id,
            file_ids,
            reverse_file_ids,
            pending_mirs,
            wellknown_defs,
        };

        (state, result)
    }

    fn emit_mir(&mut self, ctx: rustc_gen::HirStructureCtx, def: DefId) -> Body {
        let pending_mir = self.pending_mirs.remove(&def).unwrap();

        match pending_mir {
            MirOwnerInfo::CloneMethod(adt) => {
                build_clone_method_body(adt, ctx.span_in_file(self.file_id, 0, 0))
            }
            MirOwnerInfo::Const => build_zeroed_static_initializer_body(
                &self.wellknown_defs,
                CrateItem(def).ty(),
                ctx.span_in_file(self.file_id, 0, 0),
            ),
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
            MirOwnerInfo::ForwardingFn {
                def,
                target,
                param_names,
                resolver,
            } => {
                let span = ctx.span_in_file(self.file_id, 0, 0);
                let hir = co2_hir::build_forwarding_fn_body(
                    def,
                    target,
                    &param_names,
                    span,
                    &self.wellknown_defs,
                );
                let mir_start = Instant::now();
                let mir_result = co2_mir::build_mir_for_body(
                    &hir,
                    &ctx,
                    def.0,
                    self.file_id,
                    self.wellknown_defs,
                    Some(resolver),
                );
                time_report::accumulate_mir_lowering(mir_start.elapsed());
                return mir_result.body;
            }
            MirOwnerInfo::Fn {
                def,
                function_name,
                param_names,
                resolver,
                body,
            } => {
                let span_converter = |span: co2_ast::Span| self.map_co2_span(&ctx, span);
                let chumsky_span_converter =
                    |span: rustc_public_generative::rustc_public::ty::Span| {
                        self.map_rust_to_co2_span(&ctx, span)
                    };
                let mut hir_ctx = HirCtx::new(
                    self.wellknown_defs,
                    &span_converter,
                    &chumsky_span_converter,
                    Some(function_name),
                    def.fn_sig().skip_binder().output(),
                    resolver.clone(),
                );

                let hir_start = Instant::now();
                let hir = match std::panic::catch_unwind(AssertUnwindSafe(|| {
                    co2_hir::lower_function_body(body.clone(), def, &param_names, &mut hir_ctx)
                })) {
                    Ok(hir) => {
                        time_report::accumulate_hir_lowering(hir_start.elapsed());
                        hir
                    }
                    Err(payload) => {
                        if co2_ast::is_diagnostic_abort(payload.as_ref()) {
                            let hir = build_error_fn_body(
                                &self.wellknown_defs,
                                &def.fn_sig().skip_binder(),
                                ctx.span_in_file(self.file_id, 0, 0),
                            );
                            let mir_start = Instant::now();
                            let mir_result = co2_mir::build_mir_for_body(
                                &hir,
                                &ctx,
                                def.0,
                                self.file_id,
                                self.wellknown_defs,
                                Some(resolver),
                            );
                            time_report::accumulate_mir_lowering(mir_start.elapsed());
                            if DUMP_MIR_ENABLED.load(Ordering::Relaxed) {
                                let name = ctx.tcx.def_path_str(
                                    rustc_public_generative::rustc_public::rustc_internal::internal(
                                        ctx.tcx, def.0,
                                    ),
                                );
                                dump_mir_body(&mir_result.body, &name);
                            }
                            return mir_result.body;
                        }
                        std::panic::resume_unwind(payload);
                    }
                };

                let mir_start = Instant::now();
                let mir_result = co2_mir::build_mir_for_body(
                    &hir,
                    &ctx,
                    def.0,
                    self.file_id,
                    self.wellknown_defs,
                    Some(resolver),
                );
                time_report::accumulate_mir_lowering(mir_start.elapsed());
                if DUMP_MIR_ENABLED.load(Ordering::Relaxed) {
                    let name = ctx.tcx.def_path_str(
                        rustc_public_generative::rustc_public::rustc_internal::internal(
                            ctx.tcx, def.0,
                        ),
                    );
                    dump_mir_body(&mir_result.body, &name);
                }
                mir_result.body
            }
            MirOwnerInfo::FnBodyError { def, body_span } => {
                let hir = build_error_fn_body(
                    &self.wellknown_defs,
                    &def.fn_sig().skip_binder(),
                    self.map_co2_span(&ctx, body_span),
                );
                let mir_start = Instant::now();
                let mir_result = co2_mir::build_mir_for_body(
                    &hir,
                    &ctx,
                    def.0,
                    self.file_id,
                    self.wellknown_defs,
                    None,
                );
                time_report::accumulate_mir_lowering(mir_start.elapsed());
                if DUMP_MIR_ENABLED.load(Ordering::Relaxed) {
                    let name = ctx.tcx.def_path_str(
                        rustc_public_generative::rustc_public::rustc_internal::internal(
                            ctx.tcx, def.0,
                        ),
                    );
                    dump_mir_body(&mir_result.body, &name);
                }
                mir_result.body
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
        let file_id = self.file_ids[&span.data().context];
        ctx.span_in_file(file_id, span.data().start as u32, span.data().end as u32)
    }

    fn map_rust_to_co2_span(
        &self,
        ctx: &HirStructureCtx<'_>,
        span: rustc_public_generative::rustc_public::ty::Span,
    ) -> co2_ast::Span {
        let (file_id, lo, hi) = ctx.span_data(span);
        let co2_file_id = self.reverse_file_ids[&file_id];
        co2_ast::Span::from_parts(co2_file_id, lo as usize..hi as usize)
    }

    fn lower_explicit_static_mir(
        &mut self,
        ctx: &HirStructureCtx<'_>,
        def: DefId,
        resolver: LocalResolver,
        initializer: co2_ast::Spanned<Initializer<LocalResolver>>,
    ) -> Body {
        let span_converter = |span: co2_ast::Span| self.map_co2_span(ctx, span);
        let chumsky_span_converter = |span: rustc_public_generative::rustc_public::ty::Span| {
            self.map_rust_to_co2_span(ctx, span)
        };
        let hir_ctx = HirCtx::new(
            self.wellknown_defs,
            &span_converter,
            &chumsky_span_converter,
            None,
            CrateItem(def).ty(),
            resolver.clone(),
        );

        let mut target_ty = CrateItem(def).ty();
        if let TyKind::RigidTy(RigidTy::Array(elem_ty, len)) = target_ty.kind()
            && len.eval_target_usize().is_err()
        {
            let inferred_len =
                infer_array_len_from_initializer(initializer.clone(), elem_ty, &hir_ctx);
            target_ty = Ty::from_rigid_kind(RigidTy::Array(
                elem_ty,
                rustc_public_generative::rustc_public::ty::TyConst::try_from_target_usize(
                    inferred_len,
                )
                .expect("failed to materialize inferred array length"),
            ));
        }

        let hir_start = Instant::now();
        let hir = lower_static_body_for_ty(initializer, target_ty, &hir_ctx);
        time_report::accumulate_hir_lowering(hir_start.elapsed());

        let mir_start = Instant::now();
        let mir_result = co2_mir::build_mir_for_body(
            &hir,
            ctx,
            def,
            self.file_id,
            self.wellknown_defs,
            Some(resolver),
        );
        time_report::accumulate_mir_lowering(mir_start.elapsed());
        mir_result.body
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
        if let TyKind::RigidTy(RigidTy::Array(elem_ty, len)) = target_ty.kind()
            && len.eval_target_usize().is_err()
        {
            let len_span_converter = |span: co2_ast::Span| self.map_co2_span(ctx, span);
            let len_chumsky_converter = |span: rustc_public_generative::rustc_public::ty::Span| {
                self.map_rust_to_co2_span(ctx, span)
            };
            let len_hir_ctx = HirCtx::new(
                self.wellknown_defs,
                &len_span_converter,
                &len_chumsky_converter,
                None,
                Ty::usize_ty(),
                resolver.clone(),
            );
            let evaluated_len = eval_usize_initializer(array_len, &len_hir_ctx);
            target_ty = Ty::from_rigid_kind(RigidTy::Array(
                elem_ty,
                rustc_public_generative::rustc_public::ty::TyConst::try_from_target_usize(
                    evaluated_len,
                )
                .expect("failed to materialize static array length"),
            ));
        }
        let span_converter = |span: co2_ast::Span| self.map_co2_span(ctx, span);
        let chumsky_span_converter = |span: rustc_public_generative::rustc_public::ty::Span| {
            self.map_rust_to_co2_span(ctx, span)
        };
        let hir_ctx = HirCtx::new(
            self.wellknown_defs,
            &span_converter,
            &chumsky_span_converter,
            None,
            target_ty,
            resolver.clone(),
        );
        let hir_start = Instant::now();
        let hir = lower_static_body_for_ty(initializer, target_ty, &hir_ctx);
        time_report::accumulate_hir_lowering(hir_start.elapsed());

        let mir_start = Instant::now();
        let mir_result = co2_mir::build_mir_for_body(
            &hir,
            ctx,
            def,
            self.file_id,
            self.wellknown_defs,
            Some(resolver),
        );
        time_report::accumulate_mir_lowering(mir_start.elapsed());
        mir_result.body
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

        let mir_start = Instant::now();
        let mir_result =
            co2_mir::build_mir_for_body(&hir, ctx, prev, self.file_id, self.wellknown_defs, None);
        time_report::accumulate_mir_lowering(mir_start.elapsed());
        mir_result.body
    }
}

fn dump_mir_body(body: &Body, name: &str) {
    let dump_dir = Path::new("co2_mir_dump");
    let _ = fs::create_dir_all(dump_dir);
    let file_name = name.replace([':', ' ', '(', ')', '[', ']', '<', '>', '&', '*', '"'], "_");
    let path = dump_dir.join(format!("{file_name}.mir"));
    if let Ok(file) = fs::File::create(&path) {
        let mut w = std::io::BufWriter::new(file);
        let _ = write_body_pretty(&mut w, body, name);
    }
}

fn write_body_pretty(w: &mut impl std::io::Write, body: &Body, name: &str) -> std::io::Result<()> {
    write!(w, "fn {name}(")?;
    let mut sep = "";
    for (i, local) in body.arg_locals().iter().enumerate() {
        write!(w, "{sep}_{}: {}", i + 1, local.ty)?;
        sep = ", ";
    }
    writeln!(w, ") -> {} {{", body.ret_local().ty)?;

    let arg_count = body.arg_locals().len();
    for (i, local) in body.locals().iter().enumerate() {
        if i > arg_count {
            let m = if local.mutability == Mutability::Mut {
                "mut "
            } else {
                ""
            };
            writeln!(w, "    let {m}_{i}: {};", local.ty)?;
        }
    }

    for (i, block) in body.blocks.iter().enumerate() {
        writeln!(w, "    bb{i}: {{")?;
        for stmt in &block.statements {
            writeln!(w, "        {:?};", stmt.kind)?;
        }
        writeln!(w, "        {:?};", block.terminator.kind)?;
        writeln!(w, "    }}")?;
    }
    writeln!(w, "}}")?;
    Ok(())
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
            source_info: SourceInfo { span, scope: 0 },
        },
    };
    let return_block = BasicBlock {
        statements: vec![],
        terminator: Terminator {
            kind: TerminatorKind::Return,
            source_info: SourceInfo { span, scope: 0 },
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

    let projection = vec![rustc_public_generative::rustc_public::mir::ProjectionElem::Deref];
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
            Rvalue::Use(Operand::Copy(deref_place), WithRetag::Yes),
        ),
        source_info: SourceInfo { span, scope: 0 },
    }];

    let return_block = BasicBlock {
        statements,
        terminator: Terminator {
            kind: TerminatorKind::Return,
            source_info: SourceInfo { span, scope: 0 },
        },
    };
    Body::new(vec![return_block], locals, 1, vec![], None, span)
}

pub fn extract_feature_defines(rustc_args: &[String]) -> Vec<String> {
    let mut defines = Vec::new();
    let mut i = 0;
    while i < rustc_args.len() {
        if rustc_args[i] == "--cfg" {
            if let Some(val) = rustc_args.get(i + 1) {
                if let Some(feature) = val.strip_prefix("feature=\"") {
                    if let Some(feature_name) = feature.strip_suffix('"') {
                        let define = format!(
                            "CFG_FEATURE_{}",
                            feature_name.to_uppercase().replace('-', "_")
                        );
                        defines.push("-D".to_string());
                        defines.push(define);
                    }
                }
            }
            i += 2;
            continue;
        } else if let Some(val) = rustc_args[i].strip_prefix("--cfg=") {
            if let Some(feature) = val.strip_prefix("feature=\"") {
                if let Some(feature_name) = feature.strip_suffix('"') {
                    let define = format!(
                        "CFG_FEATURE_{}",
                        feature_name.to_uppercase().replace('-', "_")
                    );
                    defines.push("-D".to_string());
                    defines.push(define);
                }
            }
        }
        i += 1;
    }
    defines
}

pub fn compile_co2_file(mode: CompileMode, co2_file: &Path, rustc_args: Vec<String>) {
    let cpp_args = extract_feature_defines(&rustc_args);
    let preprocessed = Arc::new(co2_preprocessor::preprocess(co2_file, &cpp_args));
    compile_co2_source(mode, co2_file.to_path_buf(), preprocessed, rustc_args);
}

pub fn compile_co2_file_for_miri(
    co2_file: &Path,
    rustc_args: Vec<String>,
    after_analysis: Box<
        dyn for<'tcx> FnOnce(rustc_middle::ty::TyCtxt<'tcx>) -> rustc_driver::Compilation + Send,
    >,
) {
    let dump_mir =
        rustc_args.iter().any(|a| a.contains("dump-mir")) || std::env::var("CO2_DUMP_MIR").is_ok();
    DUMP_MIR_ENABLED.store(dump_mir, Ordering::Relaxed);
    let cpp_args = extract_feature_defines(&rustc_args);
    let preprocessed = Arc::new(co2_preprocessor::preprocess(co2_file, &cpp_args));
    install_pending_compile(CompileMode::RUST, co2_file.to_path_buf(), preprocessed);
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
    let dump_mir =
        rustc_args.iter().any(|a| a.contains("dump-mir")) || std::env::var("CO2_DUMP_MIR").is_ok();
    DUMP_MIR_ENABLED.store(dump_mir, Ordering::Relaxed);

    install_pending_compile(mode, source_path, preprocessed);

    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        if time_report::timing_enabled() {
            let after_hook: Box<
                dyn for<'tcx> FnOnce(rustc_middle::ty::TyCtxt<'tcx>) -> rustc_driver::Compilation
                    + Send,
            > = Box::new(|_tcx| {
                time_report::mark_codegen_start();
                rustc_driver::Compilation::Continue
            });
            rustc_gen::generate_with_args_and_after_analysis::<Co2GeneratorState>(
                rustc_args, after_hook,
            );
        } else {
            rustc_gen::generate_with_args::<Co2GeneratorState>(rustc_args);
        }
    }));

    match result {
        Ok(()) => {
            if co2_ast::diagnostics_were_emitted() {
                co2_ast::panic_with_diagnostic_abort();
            }
        }
        Err(payload) => {
            if co2_ast::is_diagnostic_abort(payload.as_ref()) {
                std::panic::resume_unwind(payload);
            }
            // String panics (e.g., `panic!("break co2!")`) are deliberate;
            // let them propagate as real panics (exit 101).
            if payload.downcast_ref::<String>().is_some()
                || payload.downcast_ref::<&str>().is_some()
            {
                std::panic::resume_unwind(payload);
            }
            // Non-string panics from codegen (e.g., LLVM duplicate symbol)
            // are compilation errors already reported via rustc's diagnostics.
            // Convert to a clean DiagnosticAbort (exit code 5).
            co2_ast::panic_with_diagnostic_abort();
        }
    }
}

impl Co2RustdocCallbacks {
    pub fn new(co2_file: &Path, rustc_args: &[String]) -> Self {
        let cpp_args = extract_feature_defines(rustc_args);
        let preprocessed = Arc::new(co2_preprocessor::preprocess(co2_file, &cpp_args));
        install_pending_compile(CompileMode::RUST, co2_file.to_path_buf(), preprocessed);
        Self {
            inner: rustc_gen::InterfaceCallbacks::new_without_original_owners(),
        }
    }

    pub fn config(&mut self, config: &mut rustc_interface::Config) {
        self.inner.config(config);
    }

    pub fn after_crate_root_parsing(&mut self, krate: &mut rustc_ast::Crate) {
        self.inner.after_crate_root_parsing(krate);
    }

    pub fn after_expansion(&mut self, tcx: rustc_middle::ty::TyCtxt<'_>) {
        self.inner.after_expansion(tcx);
    }
}

fn install_pending_compile(
    mode: CompileMode,
    source_path: PathBuf,
    preprocessed: Arc<PreprocessedSource>,
) {
    co2_ast::reset_diagnostic_state();
    *pending_compile_cell().try_lock().unwrap() = Some(PendingCompile {
        mode,
        source_path,
        preprocessed,
    });
}
