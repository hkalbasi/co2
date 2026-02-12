use std::collections::HashMap;

use chumsky::{Parser as _, input::Input as _};
use la_arena::Arena;

use crate::{
    Span, Spanned,
    diagnostic::{raise_error, report_error, unwind_stack_after_report},
    hir::{Block, HirBody, HirCtxInterface, Local, LocalData},
    parser::{self, CompoundStatement, LazyCompoundStatement, compound_statement},
    print_errors_and_terminate,
};

struct HirLowering<'c, C: HirCtxInterface> {
    ctx: &'c C,
    local_resolver: HashMap<String, Local<C>>,
    body: HirBody<C>,
}

impl<'c, C: HirCtxInterface> HirLowering<'c, C> {
    fn new(ctx: &'c C, span: Span) -> Self {
        Self {
            ctx,
            local_resolver: HashMap::new(),
            body: HirBody {
                locals: Arena::new(),
                root: Block {
                    stmts: vec![],
                    span,
                },
            },
        }
    }

    fn lower_block(&mut self, (ast, span): Spanned<CompoundStatement>) -> Block<C> {
        let prev_scope = self.open_new_scope();
        let mut stmts = vec![];
        for (ast_stmt, span) in ast.statements {
            match ast_stmt {
                parser::StatementOrDeclaration::Declaration(decl) => match decl.0 {
                    parser::Declaration::FunctionDefinition { .. } => {
                        raise_error(decl.1, "Nested declarations are not allowed");
                    }
                    parser::Declaration::Declaration {
                        declaration_specifiers,
                        declarators,
                    } => todo!(),
                },
                parser::StatementOrDeclaration::Statement(_) => todo!(),
            }
        }
        self.restore_scope(prev_scope);
        Block { stmts, span }
    }

    fn lower(&mut self, ast: LazyCompoundStatement, src: &'static str) {
        let (stmt, errors) = compound_statement()
            .parse(
                ast.tokens
                    .0
                    .leak()
                    .map((src.len()..src.len()).into(), |(t, s)| (t, s)),
            )
            .into_output_errors();
        if !errors.is_empty() {
            for error in errors {
                report_error(error);
            }
            print_errors_and_terminate("gav.txt".to_owned(), src, vec![]);
        }
    }

    fn open_new_scope(&self) -> HashMap<String, Local<C>> {
        self.local_resolver.clone()
    }

    fn restore_scope(&mut self, prev_scope: HashMap<String, Local<C>>) {
        self.local_resolver = prev_scope;
    }
}

impl<C: HirCtxInterface> HirBody<C> {
    pub fn lower(ast: LazyCompoundStatement, ctx: &C, src: &'static str) -> HirBody<C> {
        let mut lowering = HirLowering::new(ctx, ast.tokens.1);
        lowering.lower(ast, src);
        lowering.body
    }
}
