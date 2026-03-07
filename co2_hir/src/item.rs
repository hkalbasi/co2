use std::collections::HashMap;

use co2_ast::{
    CompoundStatement, Declaration, RustPath, Spanned, Token, TypeQueryResult,
    TypeResolver as ParserTypeResolver,
};
use co2_parser::parse_compound_statement;
use la_arena::{Arena, Idx};
use rustc_public_generative::rustc_public::{
    CrateItem, DefId,
    ty::{FnDef, FnSig, Span as RustSpan, Ty},
};

use crate::{HirStmt, primitive_type, resolver::HirCtx};

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
            stmts: vec![],
            span,
        }
    }
}

pub fn lower_function_body<R>(
    tokens: &[Spanned<Token>],
    def: FnDef,
    param_names: &[String],
    prelude_decls: &[Declaration],
    hir_ctx: &HirCtx<'_, R>,
) -> Result<HirBody, String> {
    hir_ctx.lower_function_body(tokens, def, param_names, prelude_decls)
}

pub fn lower_static_body<R>(
    (initializer, parser_span): Spanned<co2_ast::Initializer>,
    def: DefId,
    hir_ctx: &HirCtx<'_, R>,
) -> Result<HirBody, String> {
    let body_span = hir_ctx.to_rust_span(parser_span);
    let mut locals = Arena::new();
    let mut local_map: HashMap<String, LocalId> = HashMap::new();
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
        stmts: vec![HirStmt::Return(Some(init_expr), body_span)],
        span: body_span,
    })
}

impl<R> HirCtx<'_, R> {
    pub(crate) fn lower_function_body(
        &self,
        tokens: &[Spanned<Token>],
        def: FnDef,
        param_names: &[String],
        prelude_decls: &[Declaration],
    ) -> Result<HirBody, String> {
        self.reset_labels();
        struct BodyParseResolver<'a, R> {
            hir_ctx: &'a HirCtx<'a, R>,
        }
        impl<R> ParserTypeResolver for BodyParseResolver<'_, R> {
            fn classify_path(&self, path: &RustPath) -> TypeQueryResult {
                let (path, _) = path.decompose();
                let path = path.to_pretty();
                if let Some((_, r)) = self.hir_ctx.resolve(&path) {
                    r
                } else if primitive_type(&path).is_some() {
                    TypeQueryResult::Type
                } else {
                    TypeQueryResult::Expr
                }
            }
        }

        let parse_resolver = BodyParseResolver { hir_ctx: self };
        let parsed = parse_compound_statement(
            tokens,
            self.source_name.to_owned(),
            self.source,
            &parse_resolver,
        )
        .ok_or_else(|| "failed to parse function body".to_owned())?;
        self.lower_compound_statement(
            parsed,
            &def.fn_sig().skip_binder(),
            param_names,
            prelude_decls,
        )
    }

    fn lower_compound_statement(
        &self,
        (compound, parser_span): Spanned<CompoundStatement>,
        sig: &FnSig,
        param_names: &[String],
        prelude_decls: &[Declaration],
    ) -> Result<HirBody, String> {
        let body_span = self.to_rust_span(parser_span);
        let mut locals = Arena::new();
        let mut params = Vec::new();
        let mut local_map: HashMap<String, LocalId> = HashMap::new();

        locals.alloc(HirLocal {
            name: "_ret".to_owned(),
            ty: sig.output(),
            span: body_span,
        });

        for (idx, ty) in sig.inputs().iter().enumerate() {
            let name = param_names
                .get(idx)
                .cloned()
                .unwrap_or_else(|| format!("arg{idx}"));
            let id = locals.alloc(HirLocal {
                name: name.clone(),
                ty: *ty,
                span: body_span,
            });
            params.push(id);
            local_map.insert(name, id);
        }

        let mut stmts = Vec::new();
        for decl in prelude_decls {
            self.lower_decl(decl.clone(), &mut stmts, &mut locals, &mut local_map)?;
        }
        self.lower_compound_items(compound, &mut stmts, &mut locals, &mut local_map)?;

        Ok(HirBody {
            locals,
            labels: self.take_labels(),
            params,
            stmts,
            span: body_span,
        })
    }
}
