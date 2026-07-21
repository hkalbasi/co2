use std::collections::HashMap;

use co2_ast::{CompoundStatement, Spanned};
use co2_crate_sig::{LocalResolver, WellknownDefs};
use la_arena::{Arena, Idx};
use rustc_public_generative::rustc_public::ty::TyConst;
use rustc_public_generative::rustc_public::{
    CrateItem, DefId,
    ty::{
        FnDef, FnSig, GenericArgKind, GenericArgs, Region, RegionKind, RigidTy, Span as RustSpan,
        Ty,
    },
};

use crate::initializer_tree::{InitializerTree, eval_const_int};
use crate::resolver::{HirCtx, ResolvedValue};
use crate::{HirExpr, HirExprKind, HirStmt};

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
}

impl HirBody {
    pub fn new_dummy(ty: Ty, span: RustSpan) -> Self {
        let mut locals = Arena::new();
        locals.alloc(HirLocal {
            name: "_ret".to_owned(),
            ty,
            span,
            read_only: false,
        });
        Self {
            locals,
            labels: Arena::new(),
            params: vec![],
            c_variadic_local: None,
            stmts: vec![],
            span,
        }
    }
}

pub fn lower_function_body(
    tokens: Spanned<CompoundStatement<LocalResolver>>,
    def: FnDef,
    param_names: &[(usize, String, RustSpan)],
    hir_ctx: &mut HirCtx<'_>,
) -> HirBody {
    hir_ctx.lower_function_body(tokens, def, param_names)
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
    let mut local_map: HashMap<usize, LocalId> = HashMap::new();
    locals.alloc(HirLocal {
        name: "_ret".to_owned(),
        ty: target_ty,
        span: body_span,
        read_only: false,
    });
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
    }
}

pub fn infer_array_len_from_initializer(
    (initializer, parser_span): Spanned<co2_ast::Initializer<LocalResolver>>,
    elem_ty: Ty,
    hir_ctx: &HirCtx<'_>,
) -> u64 {
    let mut locals = Arena::new();
    let mut local_map: HashMap<usize, LocalId> = HashMap::new();
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
    local_map: &mut HashMap<usize, LocalId>,
) -> u64 {
    let fake_ty = Ty::from_rigid_kind(RigidTy::Array(
        elem_ty,
        TyConst::try_from_target_usize(567_567).unwrap(),
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
    let mut local_map: HashMap<usize, LocalId> = HashMap::new();
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
    ) -> HirBody {
        self.reset_labels();
        self.lower_compound_statement(parsed, &def.fn_sig().skip_binder(), param_names)
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
    wellknown_defs: &WellknownDefs,
) -> HirBody {
    let sig = def.fn_sig().skip_binder();
    let ret_ty = sig.output();

    let mut locals = Arena::new();
    let mut c_variadic_local = None;

    locals.alloc(HirLocal {
        name: "_ret".to_owned(),
        ty: ret_ty,
        span,
        read_only: false,
    });

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
        c_variadic_local = Some(id);
    }

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
        c_variadic_local,
        stmts: vec![HirStmt::Return(Some(call), span)],
        span,
    }
}

impl HirCtx<'_> {
    fn lower_compound_statement(
        &mut self,
        (compound, parser_span): Spanned<CompoundStatement<LocalResolver>>,
        sig: &FnSig,
        param_names: &[(usize, String, RustSpan)],
    ) -> HirBody {
        let body_span = self.to_rust_span(parser_span);
        let mut locals = Arena::new();
        let mut params = Vec::new();
        let mut local_map: HashMap<usize, LocalId> = HashMap::new();

        locals.alloc(HirLocal {
            name: "_ret".to_owned(),
            ty: sig.output(),
            span: body_span,
            read_only: false,
        });

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
            let id = locals.alloc(HirLocal {
                name: "__co2_c_vararg".to_owned(),
                ty: Ty::from_rigid_kind(RigidTy::Adt(
                    self.wellknown_defs.valist,
                    GenericArgs(vec![GenericArgKind::Lifetime(Region {
                        kind: RegionKind::ReErased,
                    })]),
                )),
                span: body_span,
                read_only: false,
            });
            params.push(id);
            self.c_variadic_local = Some(id);
        }

        let mut stmts = Vec::new();
        self.lower_compound_items(compound, &mut stmts, &mut locals, &mut local_map);
        let mut hoisted = self.take_hoisted_zeroed_decls();
        hoisted.extend(stmts);
        let stmts = hoisted;

        HirBody {
            locals,
            labels: self.take_labels(),
            params,
            c_variadic_local: self.c_variadic_local,
            stmts,
            span: body_span,
        }
    }
}
