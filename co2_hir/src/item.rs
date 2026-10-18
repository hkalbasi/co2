use rustc_data_structures::fx::FxHashMap;

use co2_ast::{CompoundStatement, Spanned};
use co2_crate_sig::{LocalResolver, WellknownDefs};
use la_arena::{Arena, Idx};
use rustc_public_generative::rustc_public::ty::TyConst;
use rustc_public_generative::rustc_public::{
    CrateItem, DefId,
    ty::{
        FnDef, FnSig, GenericArgKind, GenericArgs, Region, RegionKind, RigidTy, Span as RustSpan,
        Ty, TyKind,
    },
};

use crate::initializer_tree::{ARRAY_LEN_PROBE, InitializerTree, eval_const_int};
use crate::resolver::{HirCtx, ResolvedValue, VlaShape};
use crate::{HirDecl, HirExpr, HirExprKind, HirStmt};

#[derive(Clone, Debug)]
pub struct HirLocal {
    pub name: String,
    pub ty: Ty,
    pub span: RustSpan,
    pub read_only: bool,
}

pub type LocalId = Idx<HirLocal>;

#[derive(Clone, Debug)]
pub struct HirLabel {
    pub name: Option<String>,
}

pub type LabelId = Idx<HirLabel>;

#[derive(Clone, Debug)]
pub struct HirBody {
    pub locals: Arena<HirLocal>,
    pub labels: Arena<HirLabel>,
    pub params: Vec<LocalId>,
    pub c_variadic_local: Option<LocalId>,
    pub stmts: Vec<HirStmt>,
    pub span: RustSpan,
    /// GNU `__attribute__((naked))`: every asm template concatenates into
    /// a single diverging `naked_asm!`, no return value is produced.
    pub naked: bool,
}

impl HirBody {
    pub fn new_dummy(ty: Ty, span: RustSpan) -> Self {
        let mut locals = Arena::new();
        alloc_ret_local(&mut locals, ty, span);
        Self {
            locals,
            labels: Arena::new(),
            params: vec![],
            c_variadic_local: None,
            stmts: vec![],
            span,
            naked: false,
        }
    }
}

/// Allocate the `_ret` place shared by every function/static body preamble.
fn alloc_ret_local(locals: &mut Arena<HirLocal>, ty: Ty, span: RustSpan) {
    locals.alloc(HirLocal {
        name: "_ret".to_owned(),
        ty,
        span,
        read_only: false,
    });
}

/// Allocate the `__co2_c_vararg` local shared by every variadic body preamble.
fn alloc_c_variadic_local(
    locals: &mut Arena<HirLocal>,
    span: RustSpan,
    wellknown_defs: &WellknownDefs,
) -> LocalId {
    locals.alloc(HirLocal {
        name: "__co2_c_vararg".to_owned(),
        ty: Ty::from_rigid_kind(RigidTy::Adt(
            wellknown_defs.valist,
            GenericArgs(vec![GenericArgKind::Lifetime(Region {
                kind: RegionKind::ReErased,
            })]),
        )),
        span,
        read_only: false,
    })
}

pub fn lower_function_body(
    tokens: Spanned<CompoundStatement<LocalResolver>>,
    def: FnDef,
    param_names: &[(usize, String, RustSpan)],
    vla_params: &[(usize, Vec<co2_crate_sig::VlaParamDim>)],
    hir_ctx: &mut HirCtx<'_>,
) -> HirBody {
    hir_ctx.lower_function_body(tokens, def, param_names, vla_params)
}

/// Naked-function entry point (GNU `__attribute__((naked))`): the body
/// must be basic asm only; every template concatenates into a single
/// diverging `naked_asm!`.
pub fn lower_naked_function_body(
    tokens: Spanned<CompoundStatement<LocalResolver>>,
    def: FnDef,
    param_names: &[(usize, String, RustSpan)],
    hir_ctx: &mut HirCtx<'_>,
) -> HirBody {
    hir_ctx.lower_naked_function_body(tokens, def, param_names)
}

pub fn lower_static_body(
    (initializer, parser_span): Spanned<co2_ast::Initializer<LocalResolver>>,
    def: DefId,
    hir_ctx: &HirCtx<'_>,
) -> HirBody {
    lower_static_body_for_ty((initializer, parser_span), CrateItem(def).ty(), hir_ctx)
}

pub fn lower_static_body_for_ty(
    (initializer, parser_span): Spanned<co2_ast::Initializer<LocalResolver>>,
    target_ty: Ty,
    hir_ctx: &HirCtx<'_>,
) -> HirBody {
    let body_span = hir_ctx.to_rust_span(parser_span);
    let mut locals = Arena::new();
    let mut local_map: FxHashMap<usize, LocalId> = FxHashMap::default();
    alloc_ret_local(&mut locals, target_ty, body_span);
    let init_expr = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if let co2_ast::Initializer::Expr(expr) = &initializer
            && let Err(err) = hir_ctx.eval_const_expr_in_scope(expr, &mut locals, &mut local_map)
            // TODO: This is a dirty hack, should be fixed when we have proper const eval.
            // Integer const-eval legitimately can't represent float scalar initializers;
            // those are handled by the normal initializer-tree lowering below.
            && !err.1.starts_with("unsupported")
            && !err.1.starts_with("cannot use floats in const expressions")
        {
            hir_ctx.terminate_with_spanned_error(err);
        }
        let tree = hir_ctx.lower_to_initializer_tree(
            target_ty,
            (initializer, parser_span),
            &mut locals,
            &mut local_map,
            false,
        );
        hir_ctx.initializer_tree_to_expr(&tree, target_ty, parser_span)
    })) {
        Ok(init_expr) => init_expr,
        Err(payload) => {
            if co2_ast::is_diagnostic_abort(payload.as_ref()) {
                hir_ctx.zeroed_expr(target_ty, body_span)
            } else {
                std::panic::resume_unwind(payload);
            }
        }
    };
    HirBody {
        locals,
        labels: hir_ctx.take_labels(),
        params: vec![],
        c_variadic_local: None,
        stmts: vec![HirStmt::Return(Some(init_expr), body_span)],
        span: body_span,
        naked: false,
    }
}

pub fn infer_array_len_from_initializer(
    (initializer, parser_span): Spanned<co2_ast::Initializer<LocalResolver>>,
    elem_ty: Ty,
    hir_ctx: &HirCtx<'_>,
) -> u64 {
    let mut locals = Arena::new();
    let mut local_map: FxHashMap<usize, LocalId> = FxHashMap::default();
    infer_array_len_from_initializer_in_scope(
        (initializer, parser_span),
        elem_ty,
        hir_ctx,
        &mut locals,
        &mut local_map,
    )
}

pub(crate) fn infer_array_len_from_initializer_in_scope(
    (initializer, parser_span): Spanned<co2_ast::Initializer<LocalResolver>>,
    elem_ty: Ty,
    hir_ctx: &HirCtx<'_>,
    locals: &mut Arena<HirLocal>,
    local_map: &mut FxHashMap<usize, LocalId>,
) -> u64 {
    let fake_ty = Ty::from_rigid_kind(RigidTy::Array(
        elem_ty,
        TyConst::try_from_target_usize(ARRAY_LEN_PROBE).unwrap(),
    ));
    let tree = hir_ctx.lower_to_initializer_tree(
        fake_ty,
        (initializer, parser_span),
        locals,
        local_map,
        true,
    );
    let InitializerTree::Middle { children } = tree else {
        hir_ctx.terminate_with_error(parser_span, "invalid initializer for unsized array");
    };
    children.len() as u64
}

pub fn eval_usize_initializer(
    (initializer, parser_span): Spanned<co2_ast::Initializer<LocalResolver>>,
    hir_ctx: &HirCtx<'_>,
) -> u64 {
    let target_ty = Ty::usize_ty();
    let mut locals = Arena::new();
    let mut local_map: FxHashMap<usize, LocalId> = FxHashMap::default();
    let tree = hir_ctx.lower_to_initializer_tree(
        target_ty,
        (initializer, parser_span),
        &mut locals,
        &mut local_map,
        false,
    );
    let expr = hir_ctx.initializer_tree_to_expr(&tree, target_ty, parser_span);
    let Some(value) = eval_const_int(&expr) else {
        hir_ctx.terminate_with_error(parser_span, "expected integer constant initializer");
    };
    u64::try_from(value).unwrap_or_else(|_| {
        hir_ctx.terminate_with_error(
            parser_span,
            &format!("expected non-negative usize constant, got {value}"),
        )
    })
}

impl HirCtx<'_> {
    pub(crate) fn lower_function_body(
        &mut self,
        parsed: Spanned<CompoundStatement<LocalResolver>>,
        def: FnDef,
        param_names: &[(usize, String, RustSpan)],
        vla_params: &[(usize, Vec<co2_crate_sig::VlaParamDim>)],
    ) -> HirBody {
        self.reset_labels();
        self.lower_compound_statement(
            parsed,
            &def.fn_sig().skip_binder(),
            param_names,
            vla_params,
        )
    }

    pub(crate) fn lower_naked_function_body(
        &mut self,
        parsed: Spanned<CompoundStatement<LocalResolver>>,
        def: FnDef,
        param_names: &[(usize, String, RustSpan)],
    ) -> HirBody {
        self.reset_labels();
        self.lower_naked_compound_statement(parsed, &def.fn_sig().skip_binder(), param_names)
    }
}

/// Builds the body of a weak-alias forwarder: `return target(args...)`.
/// The forwarder simply forwards every one of its parameters to `target`
/// and returns whatever `target` returns, matching the GNU
/// `__attribute__((alias("target")))` semantics for functions.
pub fn build_forwarding_fn_body(
    def: FnDef,
    target: FnDef,
    param_names: &[(usize, String, RustSpan)],
    span: RustSpan,
) -> HirBody {
    let sig = def.fn_sig().skip_binder();
    let ret_ty = sig.output();

    let mut locals = Arena::new();
    alloc_ret_local(&mut locals, ret_ty, span);

    let mut params = Vec::new();
    let mut arg_exprs = Vec::new();
    for (idx, (_, name, param_span)) in param_names.iter().enumerate() {
        let ty = sig.inputs()[idx];
        let id = locals.alloc(HirLocal {
            name: name.clone(),
            ty,
            span: *param_span,
            read_only: false,
        });
        params.push(id);
        arg_exprs.push(HirExpr {
            kind: HirExprKind::Local(id),
            ty,
            span,
        });
    }

    let resolved = ResolvedValue::Fn(target, Vec::new());
    let func_ty = resolved.ty();
    let call = HirExpr {
        kind: HirExprKind::Call {
            func: Box::new(HirExpr {
                kind: HirExprKind::Path(resolved),
                ty: func_ty,
                span,
            }),
            args: arg_exprs,
        },
        ty: ret_ty,
        span,
    };

    HirBody {
        locals,
        labels: Arena::new(),
        params,
        c_variadic_local: None,
        stmts: vec![HirStmt::Return(Some(call), span)],
        span,
        naked: false,
    }
}

impl HirCtx<'_> {
    /// Shared `_ret`/param/variadic preamble for both body kinds.
    fn alloc_fn_preamble(
        &mut self,
        sig: &FnSig,
        param_names: &[(usize, String, RustSpan)],
        vla_params: &[(usize, Vec<co2_crate_sig::VlaParamDim>)],
        parser_span: co2_ast::Span,
    ) -> (
        RustSpan,
        Arena<HirLocal>,
        Vec<LocalId>,
        FxHashMap<usize, LocalId>,
        Vec<HirStmt>,
    ) {
        let body_span = self.to_rust_span(parser_span);
        *self.alloca_storage.borrow_mut() = None;
        *self.vla.borrow_mut() = Default::default();
        *self.pending_vla.borrow_mut() = None;
        *self.vla_typedefs.borrow_mut() = FxHashMap::default();
        let mut locals = Arena::new();
        let mut params = Vec::new();
        let mut local_map: FxHashMap<usize, LocalId> = FxHashMap::default();
        let mut param_stmts: Vec<HirStmt> = Vec::new();
        let usize_ty = Ty::usize_ty();

        alloc_ret_local(&mut locals, sig.output(), body_span);

        for (idx, ty) in sig.inputs().iter().enumerate() {
            let name = &param_names[idx];
            let param_span = name.2;
            let id = locals.alloc(HirLocal {
                name: name.1.clone(),
                ty: *ty,
                span: param_span,
                read_only: false,
            });
            params.push(id);
            local_map.insert(name.0, id);
        }
        if sig.c_variadic {
            let id = alloc_c_variadic_local(&mut locals, body_span, &self.wellknown_defs);
            params.push(id);
            self.c_variadic_local = Some(id);
        }

        for (idx, dims) in vla_params {
            let Some(param_ty) = sig.inputs().get(*idx) else {
                continue;
            };
            let TyKind::RigidTy(RigidTy::RawPtr(elem_ty, _)) = param_ty.kind() else {
                continue;
            };
            let mut dim_locals = Vec::with_capacity(dims.len());
            let mut resolved = true;
            for dim in dims {
                let value = match dim {
                    co2_crate_sig::VlaParamDim::Name(name) => {
                        match param_names
                            .iter()
                            .find(|p| &p.1 == name)
                            .and_then(|p| local_map.get(&p.0))
                        {
                            Some(local) => {
                                let src_ty = locals[*local].ty;
                                let src = HirExpr {
                                    kind: HirExprKind::Local(*local),
                                    ty: src_ty,
                                    span: body_span,
                                };
                                if src_ty == usize_ty {
                                    src
                                } else {
                                    HirExpr {
                                        kind: HirExprKind::Cast(Box::new(src)),
                                        ty: usize_ty,
                                        span: body_span,
                                    }
                                }
                            }
                            None => {
                                resolved = false;
                                break;
                            }
                        }
                    }
                    co2_crate_sig::VlaParamDim::Const(n) => HirExpr {
                        kind: HirExprKind::ConstInt(*n as i128),
                        ty: usize_ty,
                        span: body_span,
                    },
                };
                let local = locals.alloc(HirLocal {
                    name: "__co2_vla_dim".to_owned(),
                    ty: usize_ty,
                    span: body_span,
                    read_only: false,
                });
                param_stmts.push(HirStmt::Decl(HirDecl {
                    local,
                    initializer: Some(value),
                    span: body_span,
                }));
                dim_locals.push(local);
            }
            if resolved {
                let shape = VlaShape {
                    elem_ty,
                    dims: dim_locals,
                };
                self.vla.borrow_mut().ptrs.insert(params[*idx], shape);
            }
        }

        (body_span, locals, params, local_map, param_stmts)
    }

    fn lower_compound_statement(
        &mut self,
        (compound, parser_span): Spanned<CompoundStatement<LocalResolver>>,
        sig: &FnSig,
        param_names: &[(usize, String, RustSpan)],
        vla_params: &[(usize, Vec<co2_crate_sig::VlaParamDim>)],
    ) -> HirBody {
        let (body_span, mut locals, params, mut local_map, mut stmts) =
            self.alloc_fn_preamble(sig, param_names, vla_params, parser_span);

        self.lower_compound_items(compound, &mut stmts, &mut locals, &mut local_map);
        if let Some((storage, init)) = self.take_alloca_storage_init(body_span) {
            // Free the backing store before every `return`, and on fall-through.
            self.insert_alloca_storage_drops(&mut stmts, storage, &mut locals, body_span);
            stmts.push(self.alloca_storage_drop(storage, body_span));
            stmts.insert(
                0,
                HirStmt::Decl(HirDecl {
                    local: storage,
                    initializer: Some(init),
                    span: body_span,
                }),
            );
        }

        HirBody {
            locals,
            labels: self.take_labels(),
            params,
            c_variadic_local: self.c_variadic_local,
            stmts,
            span: body_span,
            naked: false,
        }
    }

    fn lower_naked_compound_statement(
        &mut self,
        (compound, parser_span): Spanned<CompoundStatement<LocalResolver>>,
        sig: &FnSig,
        param_names: &[(usize, String, RustSpan)],
    ) -> HirBody {
        let (body_span, mut locals, params, mut local_map, mut stmts) =
            self.alloc_fn_preamble(sig, param_names, &[], parser_span);

        self.lower_naked_compound_items(compound, &mut stmts, &mut locals, &mut local_map);

        HirBody {
            locals,
            labels: self.take_labels(),
            params,
            c_variadic_local: self.c_variadic_local,
            stmts,
            span: body_span,
            naked: true,
        }
    }
}
