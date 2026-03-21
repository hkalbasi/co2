use std::collections::HashMap;

use co2_ast::{CompoundStatement, Spanned};
use co2_crate_sig::LocalResolver;
use la_arena::{Arena, Idx};
use rustc_public_generative::rustc_public::{
    CrateItem, DefId,
    ty::{
        FnDef, FnSig, GenericArgKind, GenericArgs, Region, RegionKind, RigidTy, Span as RustSpan,
        Ty,
    },
};

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
    let body_span = hir_ctx.to_rust_span(parser_span);
    let mut locals = Arena::new();
    let mut local_map: HashMap<usize, LocalId> = HashMap::new();
    locals.alloc(HirLocal {
        name: "_ret".to_owned(),
        ty: CrateItem(def).ty(),
        span: body_span,
    });
    let target_ty = CrateItem(def).ty();
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
