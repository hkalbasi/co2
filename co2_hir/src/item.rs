use std::collections::HashMap;

use co2_parser::{
    CompoundStatement, Declaration, RustPath, Spanned, Token, TypeQueryResult,
    TypeResolver as ParserTypeResolver, parse_compound_statement,
};
use la_arena::{Arena, Idx};
use rustc_public_generative::rustc_public::ty::{FnDef, FnSig, Span as RustSpan, Ty};

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
    pub stmts: Vec<HirStmt>,
    pub span: RustSpan,
}

pub fn lower_function_body<R>(
    tokens: &[Spanned<Token>],
    source_name: &str,
    source: &'static str,
    def: FnDef,
    param_names: &[String],
    prelude_decls: &[Declaration],
    hir_ctx: &HirCtx<'_, R>,
) -> Result<HirBody, String> {
    hir_ctx.lower_function_body(tokens, source_name, source, def, param_names, prelude_decls)
}

impl<R> HirCtx<'_, R> {
    pub fn lower_function_body(
        &self,
        tokens: &[Spanned<Token>],
        source_name: &str,
        source: &'static str,
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
                let path = path.to_pretty();
                if self.hir_ctx.resolve_type(&path).is_some() {
                    TypeQueryResult::Type
                } else {
                    TypeQueryResult::Expr
                }
            }
        }

        let parse_resolver = BodyParseResolver { hir_ctx: self };
        let parsed = parse_compound_statement(tokens, source_name.to_owned(), source, &parse_resolver)
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
