use std::{cell::RefCell, collections::HashMap, rc::Rc};

use co2_ast::{
    Declaration, DeclarationSpecifier, Declarator, DoTransform as _, EnumSpecifier, Expression,
    InitDeclarator, RustPath, Spanned, StatelessResolver, StructOrUnionSpecifier, TypeQueryResult,
    TypeResolver,
};
use co2_parser::parse_expression_tokens;
use rustc_public_generative::{DefData, FileId, HirStructureCtx, rustc_public::DefId};
use rustc_public_generative::HirTy;

use crate::{
    Resolver,
    struct_manager::StructManager,
    ty::{CTy, PrimitiveTy},
};

#[derive(Default, Debug, Clone)]
pub struct StructAndEnumData {
    pub struct_tags: im::HashMap<String, DefId>,
}

#[derive(Debug)]
pub struct LocalResolverBase {
    pub resolver: Resolver,
    pub local_counter: usize,
    pub fake_defs_counter: usize,
    pub pending_typedefs: Vec<(
        DefId,
        String,
        Vec<co2_ast::Spanned<DeclarationSpecifier<LocalResolver>>>,
        Declarator<LocalResolver>,
        co2_ast::Span,
    )>,
    pub pending_static: Vec<(
        DefId,
        String,
        Vec<co2_ast::Spanned<DeclarationSpecifier<LocalResolver>>>,
        InitDeclarator<LocalResolver>,
        co2_ast::Span,
    )>,
    pub pending_array_len_consts:
        Vec<(DefId, String, co2_ast::Spanned<Expression<LocalResolver>>, co2_ast::Span)>,
    pub array_len_consts: HashMap<(usize, usize), RegisteredArrayLenConst>,
    pub array_len_const_exprs: HashMap<DefId, co2_ast::Spanned<Expression<LocalResolver>>>,
    pub hir_ctx: &'static HirStructureCtx<'static>,
    pub file_id: FileId,
    pub source_name: String,
    pub source: &'static str,
    pub(crate) struct_manager: StructManager,
    pub(crate) unrepresentable_typedefs: HashMap<String, CTy>,
    pub(crate) typedef_tys: HashMap<DefId, HirTy>,
    pub(crate) global_struct_tags: Rc<RefCell<StructAndEnumData>>,
    pub(crate) global_locals: Rc<RefCell<im::HashMap<String, (DefOrLocal, TypeQueryResult)>>>,
}

#[derive(Debug, Clone)]
pub struct RegisteredArrayLenConst {
    pub def_id: DefId,
    pub expr: co2_ast::Spanned<Expression<LocalResolver>>,
}

#[derive(Debug, Clone)]
pub struct RegisteredSubscription {
    pub raw: co2_ast::LazySubscription,
    pub array_len_const: Option<DefId>,
}

impl LocalResolverBase {
    pub fn emit_fake_def(&mut self, def_data: fn(String) -> DefData) -> (DefId, String) {
        let fake_name = format!("__co2_fake_def_{}", self.fake_defs_counter);
        self.fake_defs_counter += 1;
        let def_id = self.hir_ctx.allocate_def_id(
            self.hir_ctx.root_crate_def_id(),
            def_data(fake_name.clone()),
        );
        (def_id, fake_name)
    }

    pub fn take_pending_array_len_consts(
        &mut self,
    ) -> Vec<(DefId, String, co2_ast::Spanned<Expression<LocalResolver>>, co2_ast::Span)> {
        std::mem::take(&mut self.pending_array_len_consts)
    }

    pub fn lookup_array_len_const(
        &self,
        span: co2_ast::Span,
    ) -> Option<RegisteredArrayLenConst> {
        self.array_len_consts
            .get(&(span.start, span.end))
            .cloned()
    }

    pub fn lookup_array_len_const_expr(
        &self,
        def_id: DefId,
    ) -> Option<co2_ast::Spanned<Expression<LocalResolver>>> {
        self.array_len_const_exprs.get(&def_id).cloned()
    }
}

pub fn eval_registered_array_len_const(
    resolver: &LocalResolver,
    def_id: DefId,
) -> Result<usize, String> {
    let mut base = resolver.base.borrow_mut();
    let expr = base
        .lookup_array_len_const_expr(def_id)
        .ok_or_else(|| "missing registered array size constant expression".to_owned())?;
    base.eval_array_len_expr(&expr)
}

#[derive(Debug, Clone)]
pub struct LocalResolver {
    pub(crate) base: Rc<RefCell<LocalResolverBase>>,
    pub(crate) locals: Rc<RefCell<im::HashMap<String, (DefOrLocal, TypeQueryResult)>>>,
    pub(crate) struct_tags: Rc<RefCell<StructAndEnumData>>,
}

impl LocalResolver {
    pub fn new(base: Rc<RefCell<LocalResolverBase>>) -> Self {
        let struct_tags = base.borrow().global_struct_tags.clone();
        let locals = base.borrow().global_locals.clone();
        LocalResolver {
            struct_tags,
            base,
            locals,
        }
    }

    fn localize(&mut self) {
        let struct_tags = self.struct_tags.borrow().clone();
        self.struct_tags = Rc::new(RefCell::new(struct_tags));
        let locals = self.locals.borrow().clone();
        self.locals = Rc::new(RefCell::new(locals));
    }

    pub fn add_local(&mut self, name: String) -> usize {
        let (id, name) = self.register_ident(name);
        self.locals
            .borrow_mut()
            .insert(name, (DefOrLocal::Local(id as u32), TypeQueryResult::Expr));
        id
    }

    fn register_array_len_const(
        &self,
        subscription: Spanned<co2_ast::LazySubscription>,
    ) -> Option<RegisteredArrayLenConst> {
        if subscription.0.is_unsized() || subscription.0.constant_len().is_some() {
            return None;
        }
        let key = (subscription.1.start, subscription.1.end);
        if let Some(existing) = self.base.borrow().array_len_consts.get(&key).cloned() {
            return Some(existing);
        }
        let tokens = subscription
            .0
            .tokens
            .get(1..subscription.0.tokens.len().saturating_sub(1))?;
        let expr = parse_expression_tokens(
            tokens,
            self.base.borrow().source_name.clone(),
            self.base.borrow().source,
            self.clone(),
        );
        let mut base = self.base.borrow_mut();
        let (def_id, fake_name) = base.emit_fake_def(DefData::ValueNs);
        let registered = RegisteredArrayLenConst { def_id, expr: expr.clone() };
        base.pending_array_len_consts
            .push((def_id, fake_name, expr, subscription.1));
        base.array_len_const_exprs.insert(def_id, registered.expr.clone());
        base.array_len_consts.insert(key, registered.clone());
        Some(registered)
    }
}

#[derive(Debug, Clone)]
pub enum DefOrLocal {
    Def(DefId),
    Local(u32),
    Prim(PrimitiveTy),
    UnrepresentableType(CTy),
}

impl co2_ast::TypeResolver for LocalResolver {
    type ResolvedRustPath = DefOrLocal;
    type DeclarationIdent = (usize, String);
    type StructOrUnionIdentifier = DefId;
    type EnumIdentifier = ();
    type EnumeratorIdentifier = (DefId, String, Option<Spanned<Expression<Self>>>);
    type SubscriptionIdentifier = RegisteredSubscription;

    fn classify_path(
        &self,
        path: &co2_ast::RustPath,
    ) -> Option<(TypeQueryResult, Self::ResolvedRustPath)> {
        let path = path.to_pretty();
        let base = self.base.borrow();
        if let Some(prim) = PrimitiveTy::parse(&path) {
            return Some((TypeQueryResult::Type, DefOrLocal::Prim(prim)));
        }
        if let Some(ty) = self.base.borrow().unrepresentable_typedefs.get(&path) {
            return Some((
                TypeQueryResult::Type,
                DefOrLocal::UnrepresentableType(ty.clone()),
            ));
        }
        let (def, class) = self.locals.borrow().get(&path).cloned().or_else(|| {
            let (def_id, class) = base.resolver.resolve(&path).ok()?;
            Some((DefOrLocal::Def(def_id), class))
        })?;
        Some((class, def))
    }

    fn register_ident(&self, name: String) -> Self::DeclarationIdent {
        let mut base = self.base.borrow_mut();
        let id = base.local_counter;
        base.local_counter += 1;
        (id, name)
    }

    fn declare_ident_as_local(&self, ident: &(usize, String)) -> Self {
        let mut next = self.clone();
        next.localize();
        next.locals.borrow_mut().insert(
            ident.1.clone(),
            (DefOrLocal::Local(ident.0 as u32), TypeQueryResult::Expr),
        );
        next
    }

    fn start_new_scope(&self) -> Self {
        let mut next = self.clone();
        next.localize();
        next
    }

    fn register_decl(&self, decl: &Declaration<Self>) -> Self {
        let mut next = self.clone();
        next.localize();

        match decl {
            Declaration::FunctionDefinition { .. } => next,
            Declaration::Declaration {
                declaration_specifiers,
                declarators,
            } => {
                let is_typedef = declaration_specifiers.iter().any(|d| d.0.is_typedef());
                let is_static = declaration_specifiers.iter().any(|d| d.0.is_static());
                if is_typedef && is_static {
                    todo!("Emit good error");
                }
                for decl in declarators {
                    let Some(name) = decl.0.declarator.0.ident() else {
                        continue;
                    };
                    if is_typedef {
                        let mut base = next.base.borrow_mut();
                        let (def_id, fake_name) =
                            base.emit_fake_def(rustc_public_generative::DefData::TypeNs);
                        base.pending_typedefs.push((
                            def_id,
                            fake_name,
                            declaration_specifiers.clone(),
                            decl.0.declarator.0.clone(),
                            decl.1,
                        ));
                        next.locals
                            .borrow_mut()
                            .insert(name.1, (DefOrLocal::Def(def_id), TypeQueryResult::Type));
                    } else if is_static {
                        let mut base = next.base.borrow_mut();
                        let (def_id, fake_name) =
                            base.emit_fake_def(rustc_public_generative::DefData::ValueNs);
                        base.pending_static.push((
                            def_id,
                            fake_name,
                            declaration_specifiers.clone(),
                            decl.0.clone(),
                            decl.1,
                        ));
                        next.locals
                            .borrow_mut()
                            .insert(name.1, (DefOrLocal::Def(def_id), TypeQueryResult::Expr));
                    } else if decl.0.declarator.0.is_function() {
                        // TODO: detect if we need to emit an extern function here.
                    } else {
                        next.locals.borrow_mut().insert(
                            name.1,
                            (DefOrLocal::Local(name.0 as u32), TypeQueryResult::Expr),
                        );
                    }
                }
                next
            }
        }
    }

    fn register_struct_or_union_specifier(
        &self,
        kind: co2_ast::StructOrUnionKind,
        (specifier, span): co2_ast::Spanned<co2_ast::StructOrUnionSpecifier<Self>>,
    ) -> Self::StructOrUnionIdentifier {
        self.lower_struct_specifier(kind, specifier, span)
    }

    fn register_enumerator(
        &self,
        (specifier, span): co2_ast::Spanned<co2_ast::Enumerator<Self>>,
    ) -> Self::EnumeratorIdentifier {
        self.collect_enumerator(specifier, span)
    }

    fn register_enum_specifier(
        &self,
        (specifier, span): co2_ast::Spanned<co2_ast::EnumSpecifier<Self>>,
    ) -> Self::EnumIdentifier {
        self.collect_enum_constants(specifier, span);
    }

    fn register_subscription(
        &self,
        subscription: Spanned<co2_ast::LazySubscription>,
    ) -> Self::SubscriptionIdentifier {
        RegisteredSubscription {
            raw: subscription.0.clone(),
            array_len_const: self
                .register_array_len_const(subscription)
                .map(|registered| registered.def_id),
        }
    }
}

impl co2_ast::Transformable<StatelessResolver> for LocalResolver {
    fn transform_decl_ident(a: &String) -> (usize, String) {
        (0, a.clone())
    }

    fn transform_struct_or_union_specifier(
        &self,
        kind: co2_ast::StructOrUnionKind,
        specifier: &Spanned<StructOrUnionSpecifier<StatelessResolver>>,
    ) -> Spanned<Self::StructOrUnionIdentifier> {
        (
            self.register_struct_or_union_specifier(kind, specifier.transform(self)),
            specifier.1,
        )
    }

    fn transform_path(&self, (path, span): &Spanned<RustPath>) -> Spanned<Self::ResolvedRustPath> {
        let Some(r) = self.classify_path(path) else {
            self.base
                .borrow()
                .terminate_with_error(*span, "Unresolved name");
        };
        (r.1, *span)
    }

    fn transform_subscription(
        &self,
        subscription: &Spanned<<StatelessResolver as TypeResolver>::SubscriptionIdentifier>,
    ) -> Spanned<Self::SubscriptionIdentifier> {
        (self.register_subscription(subscription.clone()), subscription.1)
    }

    fn transform_enumerator(
        &self,
        specifier: &Spanned<<StatelessResolver as TypeResolver>::EnumeratorIdentifier>,
    ) -> Spanned<Self::EnumeratorIdentifier> {
        let span = specifier.1;
        (
            self.collect_enumerator(specifier.0.transform(self), span),
            span,
        )
    }

    fn transform_enum_specifier(
        &self,
        specifier: &Spanned<EnumSpecifier<StatelessResolver>>,
    ) -> Spanned<Self::EnumIdentifier> {
        let span = specifier.1;
        (
            self.register_enum_specifier(specifier.transform(self)),
            span,
        )
    }
}
