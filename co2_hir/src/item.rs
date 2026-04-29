use std::collections::HashMap;

use co2_ast::{CompoundStatement, Spanned};
use co2_crate_sig::LocalResolver;
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
use crate::{HirStmt, resolver::HirCtx};

#[derive(Clone, Debug)]
pub struct HirLocal {
    pub name: String,
    pub ty: Ty,
    pub span: RustSpan,
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
    param_names: &[(usize, String)],
    hir_ctx: &mut HirCtx<'_>,
) -> Result<HirBody, String> {
    hir_ctx.lower_function_body(tokens, def, param_names)
}

pub fn lower_static_body(
    (initializer, parser_span): Spanned<co2_ast::Initializer<LocalResolver>>,
    def: DefId,
    hir_ctx: &HirCtx<'_>,
) -> Result<HirBody, String> {
    lower_static_body_for_ty((initializer, parser_span), CrateItem(def).ty(), hir_ctx)
}

pub fn lower_static_body_for_ty(
    (initializer, parser_span): Spanned<co2_ast::Initializer<LocalResolver>>,
    target_ty: Ty,
    hir_ctx: &HirCtx<'_>,
) -> Result<HirBody, String> {
    let body_span = hir_ctx.to_rust_span(parser_span);
    let mut locals = Arena::new();
    let mut local_map: HashMap<usize, LocalId> = HashMap::new();
    locals.alloc(HirLocal {
        name: "_ret".to_owned(),
        ty: target_ty,
        span: body_span,
    });
    let tree = hir_ctx.lower_to_initializer_tree(
        target_ty,
        (initializer, parser_span),
        &mut locals,
        &mut local_map,
    );
    let init_expr = hir_ctx.initializer_tree_to_expr(&tree, target_ty, parser_span);
    Ok(HirBody {
        locals,
        labels: hir_ctx.take_labels(),
        params: vec![],
        c_variadic_local: None,
        stmts: vec![HirStmt::Return(Some(init_expr), body_span)],
        span: body_span,
    })
}

pub fn infer_array_len_from_initializer(
    (initializer, parser_span): Spanned<co2_ast::Initializer<LocalResolver>>,
    elem_ty: Ty,
    hir_ctx: &HirCtx<'_>,
) -> Result<u64, String> {
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
) -> Result<u64, String> {
    let fake_ty = Ty::from_rigid_kind(RigidTy::Array(
        elem_ty,
        TyConst::try_from_target_usize(567_567).unwrap(),
    ));
    let tree =
        hir_ctx.lower_to_initializer_tree(fake_ty, (initializer, parser_span), locals, local_map);
    let InitializerTree::Middle { children } = tree else {
        return Err("invalid initializer for unsized array".to_owned());
    };
    Ok(children.len() as u64)
}

pub fn eval_usize_initializer(
    (initializer, parser_span): Spanned<co2_ast::Initializer<LocalResolver>>,
    hir_ctx: &HirCtx<'_>,
) -> Result<u64, String> {
    let target_ty = Ty::usize_ty();
    let mut locals = Arena::new();
    let mut local_map: HashMap<usize, LocalId> = HashMap::new();
    let tree = hir_ctx.lower_to_initializer_tree(
        target_ty,
        (initializer, parser_span),
        &mut locals,
        &mut local_map,
    );
    let expr = hir_ctx.initializer_tree_to_expr(&tree, target_ty, parser_span);
    let value = eval_const_int(&expr)?;
    u64::try_from(value).map_err(|_| format!("expected non-negative usize constant, got {value}"))
}

impl HirCtx<'_> {
    pub(crate) fn lower_function_body(
        &mut self,
        parsed: Spanned<CompoundStatement<LocalResolver>>,
        def: FnDef,
        param_names: &[(usize, String)],
    ) -> Result<HirBody, String> {
        self.reset_labels();
        self.lower_compound_statement(parsed, &def.fn_sig().skip_binder(), &param_names)
    }

    fn lower_compound_statement(
        &mut self,
        (compound, parser_span): Spanned<CompoundStatement<LocalResolver>>,
        sig: &FnSig,
        param_names: &[(usize, String)],
    ) -> Result<HirBody, String> {
        let body_span = self.to_rust_span(parser_span);
        let mut locals = Arena::new();
        let mut params = Vec::new();
        let mut local_map: HashMap<usize, LocalId> = HashMap::new();

        locals.alloc(HirLocal {
            name: "_ret".to_owned(),
            ty: sig.output(),
            span: body_span,
        });

        for (idx, ty) in sig.inputs().iter().enumerate() {
            let name = &param_names[idx];
            let id = locals.alloc(HirLocal {
                name: name.1.clone(),
                ty: *ty,
                span: body_span,
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
            });
            params.push(id);
            self.c_variadic_local = Some(id);
        }

        let mut stmts = Vec::new();
        self.lower_compound_items(compound, &mut stmts, &mut locals, &mut local_map)?;

        Ok(HirBody {
            locals,
            labels: self.take_labels(),
            params,
            c_variadic_local: self.c_variadic_local,
            stmts,
            span: body_span,
        })
    }
}
