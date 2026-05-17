use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::Arc};

use co2_ast::{
    Declaration, DeclarationSpecifier, Declarator, DoTransform as _, EnumSpecifier, Expression,
    InitDeclarator, Initializer, RustPath, Spanned, StatelessResolver, StructOrUnionSpecifier,
    TypeQueryResult, TypeResolver,
};
use co2_parser::parse_expression_tokens;
use co2_preprocessor::PreprocessedSource;
use rustc_public_generative::HirTy;
use rustc_public_generative::{
    DefData, FileId, HirStructureCtx,
    rustc_public::{DefId, ty::Ty},
};

use crate::{
    Resolver,
    resolver::ResolvedExprPath,
    struct_manager::StructManager,
    ty::{CTy, PrimitiveTy},
};

fn expr_contains_label_address<R: TypeResolver>(expr: &Expression<R>) -> bool {
    match expr {
        Expression::LabelAddress(_) => true,
        Expression::Field(base, _)
        | Expression::Arrow(base, _)
        | Expression::UnaryOp(_, base)
        | Expression::Sizeof(base)
        | Expression::Alignof(base) => expr_contains_label_address(&base.0),
        Expression::Subscript(base, index) | Expression::BinOp(base, _, index) => {
            expr_contains_label_address(&base.0) || expr_contains_label_address(&index.0)
        }
        Expression::Call { func, params } => {
            expr_contains_label_address(&func.0)
                || params
                    .iter()
                    .any(|param| expr_contains_label_address(&param.0))
        }
        Expression::Update { expr, .. } | Expression::Cast { expr, .. } => {
            expr_contains_label_address(&expr.0)
        }
        Expression::AssignWithOp { lhs, rhs, .. } => {
            expr_contains_label_address(&lhs.0) || expr_contains_label_address(&rhs.0)
        }
        Expression::Conditional {
            cond,
            then_expr,
            else_expr,
        } => {
            expr_contains_label_address(&cond.0)
                || expr_contains_label_address(&then_expr.0)
                || expr_contains_label_address(&else_expr.0)
        }
        Expression::CompoundLiteral { initializer, .. } => {
            initializer_contains_label_address(&initializer.0)
        }
        Expression::GenericSelection {
            controlling,
            associations,
        } => {
            expr_contains_label_address(&controlling.0)
                || associations.iter().any(|assoc| match &assoc.0 {
                    co2_ast::GenericAssociation::Type { expr, .. }
                    | co2_ast::GenericAssociation::Default { expr } => {
                        expr_contains_label_address(&expr.0)
                    }
                })
        }
        Expression::VaStart { args, .. } | Expression::VaEnd { args } => {
            expr_contains_label_address(&args.0)
        }
        Expression::VaArg { args, .. } => expr_contains_label_address(&args.0),
        Expression::Identifier(_)
        | Expression::Empty
        | Expression::Constant(_)
        | Expression::SizeofType(_)
        | Expression::AlignofType(_)
        | Expression::Offsetof { .. }
        | Expression::GnuStatementExpr { .. }
        | Expression::BuiltinTypesCompatibleP { .. } => false,
    }
}

fn initializer_contains_label_address<R: TypeResolver>(
    initializer: &co2_ast::Initializer<R>,
) -> bool {
    match initializer {
        co2_ast::Initializer::Expr(expr) => expr_contains_label_address(&expr.0),
        co2_ast::Initializer::List(items) => items
            .iter()
            .any(|item| initializer_contains_label_address(&item.0.initializer.0)),
    }
}

#[derive(Default, Debug, Clone)]
pub struct StructAndEnumData {
    pub struct_tags: im::HashMap<String, DefId>,
}

pub struct LocalResolverBase {
    pub resolver: Resolver,
    pub local_counter: usize,
    pub fake_defs_counter: usize,
    pub local_tys: HashMap<u32, HirTy>,
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
    pub array_len_consts: HashMap<(usize, usize), RegisteredArrayLenConst>,
    pub array_len_const_exprs: HashMap<usize, co2_ast::Spanned<Expression<LocalResolver>>>,
    pub hir_ctx: &'static HirStructureCtx<'static>,
    pub file_id: FileId,
    pub preprocessed: Arc<PreprocessedSource>,
    pub file_ids: Arc<HashMap<co2_ast::FileId, FileId>>,
    pub(crate) struct_manager: StructManager,
    pub(crate) unrepresentable_typedefs: HashMap<String, CTy>,
    pub(crate) typedef_tys: HashMap<DefId, HirTy>,
    pub(crate) global_value_tys: HashMap<DefId, HirTy>,
    pub(crate) global_struct_tags: Rc<RefCell<StructAndEnumData>>,
    pub(crate) global_locals: Rc<RefCell<im::HashMap<String, (DefOrLocal, TypeQueryResult)>>>,
    pub(crate) enum_const_values: HashMap<DefId, i128>,
    pub(crate) constexpr_def_exprs: HashMap<DefId, co2_ast::Spanned<Expression<LocalResolver>>>,
    pub(crate) constexpr_local_exprs: HashMap<u32, co2_ast::Spanned<Expression<LocalResolver>>>,
}

impl std::fmt::Debug for LocalResolverBase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalResolverBase")
            .field("local_counter", &self.local_counter)
            .field("fake_defs_counter", &self.fake_defs_counter)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
pub struct RegisteredArrayLenConst {
    pub id: usize,
    pub expr: co2_ast::Spanned<Expression<LocalResolver>>,
}

#[derive(Debug, Clone)]
pub struct RegisteredSubscription {
    pub raw: co2_ast::LazySubscription,
    pub array_len_const: Option<usize>,
}

impl LocalResolverBase {
    pub fn emit_fake_def(&mut self, def_data: fn(String) -> DefData) -> (DefId, String) {
        let fake_name = format!("__co2_fake_def_{}", self.fake_defs_counter);
        self.fake_defs_counter += 1;
        let def_id = self.hir_ctx.allocate_def_id(
            self.hir_ctx.root_crate_def_id(),
            &def_data(fake_name.clone()),
        );
        (def_id, fake_name)
    }

    pub fn lookup_array_len_const(&self, span: co2_ast::Span) -> Option<RegisteredArrayLenConst> {
        self.array_len_consts.get(&(span.start, span.end)).cloned()
    }

    pub fn lookup_array_len_const_expr(
        &self,
        id: usize,
    ) -> Option<co2_ast::Spanned<Expression<LocalResolver>>> {
        self.array_len_const_exprs.get(&id).cloned()
    }

    pub fn set_local_ty(&mut self, local: u32, ty: HirTy) {
        self.local_tys.insert(local, ty);
    }
}

pub fn eval_registered_array_len_const(
    resolver: &LocalResolver,
    id: usize,
) -> Result<usize, String> {
    let mut base = resolver.base.borrow_mut();
    let expr = base
        .lookup_array_len_const_expr(id)
        .ok_or_else(|| "missing registered array size constant expression".to_owned())?;
    base.eval_array_len_expr(&expr)
}

pub fn eval_const_expr_as_usize(
    resolver: &LocalResolver,
    expr: &co2_ast::Spanned<Expression<LocalResolver>>,
) -> Result<usize, String> {
    resolver.base.borrow_mut().eval_array_len_expr(expr)
}

#[derive(Debug, Clone)]
pub struct LocalResolver {
    pub(crate) base: Rc<RefCell<LocalResolverBase>>,
    pub(crate) locals: Rc<RefCell<im::HashMap<String, (DefOrLocal, TypeQueryResult)>>>,
    pub(crate) struct_tags: Rc<RefCell<StructAndEnumData>>,
    pub(crate) module_path: Rc<Vec<String>>,
    pub(crate) current_owner: DefId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MethodResolutionKind {
    Inherent,
    Trait,
}

impl LocalResolver {
    pub fn new(base: Rc<RefCell<LocalResolverBase>>) -> Self {
        let struct_tags = base.borrow().global_struct_tags.clone();
        let locals = base.borrow().global_locals.clone();
        let current_owner = base.borrow().hir_ctx.root_crate_def_id();
        LocalResolver {
            struct_tags,
            base,
            locals,
            module_path: Rc::new(Vec::new()),
            current_owner,
        }
    }

    pub fn with_module_path(mut self, module_path: Vec<String>) -> Self {
        self.module_path = Rc::new(module_path);
        self
    }

    pub fn with_owner(mut self, owner: DefId) -> Self {
        self.current_owner = owner;
        self
    }

    pub fn current_owner(&self) -> DefId {
        self.current_owner
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

    pub fn set_local_ty(&self, local: u32, ty: HirTy) {
        self.base.borrow_mut().set_local_ty(local, ty);
    }

    pub fn dependency_const_value(
        &self,
        def_id: DefId,
    ) -> Option<rustc_public_generative::DependencyConstValue> {
        self.base.borrow().hir_ctx.dependency_const_value(def_id)
    }

    pub fn local_const_int_value(&self, def_id: DefId) -> Result<i128, String> {
        self.base.borrow_mut().eval_local_const(def_id)
    }

    pub fn has_local_const_value(&self, def_id: DefId) -> bool {
        self.base.borrow().has_local_const_value(def_id)
    }

    pub fn local_constexpr_int_value(&self, local: u32) -> Result<i128, String> {
        self.base.borrow_mut().eval_local_constexpr(local)
    }

    pub fn has_local_constexpr(&self, local: u32) -> bool {
        self.base.borrow().has_local_constexpr(local)
    }

    pub fn is_constexpr_def(&self, def_id: DefId) -> bool {
        self.base.borrow().is_constexpr_def(def_id)
    }

    /// Returns the underlying `HirTy` for the given `def_id` if it is a type alias
    /// (e.g. an incomplete/forward-declared C struct lowered as TypeDef → ForeignType).
    pub fn get_typedef_hir_ty(&self, def_id: DefId) -> Option<HirTy> {
        self.base.borrow().typedef_tys.get(&def_id).cloned()
    }

    pub fn peel_constexpr_typedef_hir(&self, ty: HirTy) -> HirTy {
        self.base.borrow().peel_constexpr_typedef(ty)
    }

    pub fn is_enum_def(&self, def_id: DefId) -> bool {
        self.base.borrow().is_enum_def(def_id)
    }

    pub fn validate_constexpr_decl(
        &self,
        specifiers: &[Spanned<DeclarationSpecifier<LocalResolver>>],
        declarator: &Declarator<LocalResolver>,
        ty: &crate::CTy,
        initializer: Option<&Spanned<Initializer<LocalResolver>>>,
    ) -> Result<(), String> {
        self.base
            .borrow_mut()
            .validate_constexpr_decl(specifiers, declarator, ty, initializer)
    }

    pub fn eval_const_expr(
        &self,
        expr: &Spanned<Expression<LocalResolver>>,
    ) -> Result<i128, String> {
        self.base.borrow_mut().eval_const_expr(expr)
    }

    pub fn normalize_ty_for_current_owner(&self, ty: Ty) -> Ty {
        self.base
            .borrow()
            .hir_ctx
            .normalize_ty_for_owner(self.current_owner, ty)
    }

    pub fn normalize_ty_for_current_owner_with_self(&self, ty: Ty, self_ty: Ty) -> Ty {
        self.base
            .borrow()
            .hir_ctx
            .normalize_ty_for_owner_with_self(self.current_owner, ty, self_ty)
    }

    pub fn resolve_method(
        &self,
        receiver_ty: Ty,
        method: &str,
    ) -> Result<Option<(DefId, TypeQueryResult, MethodResolutionKind)>, String> {
        if let Some(found) = self
            .base
            .borrow()
            .resolver
            .resolve_inherent_method(receiver_ty, method)?
        {
            let (method_def, class) = found;
            return Ok(Some((method_def, class, MethodResolutionKind::Inherent)));
        }

        let trait_candidates = self
            .base
            .borrow()
            .resolver
            .traits_in_scope_with_method(method);
        let hir_ctx = self.base.borrow().hir_ctx;
        let mut applicable = Vec::new();
        for (trait_name, trait_def, trait_path) in trait_candidates {
            if hir_ctx.type_implements_trait(self.current_owner, receiver_ty, trait_def)
                && let Some((method_def, class)) = self
                    .base
                    .borrow()
                    .resolver
                    .resolve_trait_method(&trait_path, method)
            {
                applicable.push((trait_name, method_def, class));
            }
        }

        match applicable.len() {
            0 => match receiver_ty.kind() {
                rustc_public_generative::rustc_public::ty::TyKind::RigidTy(
                    rustc_public_generative::rustc_public::ty::RigidTy::Ref(_, inner, _)
                    | rustc_public_generative::rustc_public::ty::RigidTy::RawPtr(inner, _),
                ) => self.resolve_method(inner, method),
                _ => Ok(None),
            },
            1 => {
                let (_, method_def, class) = applicable.pop().unwrap();
                Ok(Some((method_def, class, MethodResolutionKind::Trait)))
            }
            _ => Err(format!(
                "multiple traits in scope provide method `{method}` for receiver type {receiver_ty:?}: {}",
                applicable
                    .into_iter()
                    .map(|(trait_name, _, _)| trait_name)
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }

    fn register_array_len_const(
        &self,
        subscription: &Spanned<co2_ast::LazySubscription>,
    ) -> Option<RegisteredArrayLenConst> {
        if subscription.0.is_unsized()
            || subscription.0.is_unspecified_vla()
            || subscription.0.constant_len().is_some()
        {
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
        let tokens = tokens
            .iter()
            .skip_while(|(token, _)| {
                matches!(
                    token,
                    co2_ast::Token::Static
                        | co2_ast::Token::Const
                        | co2_ast::Token::Restrict
                        | co2_ast::Token::Volatile
                        | co2_ast::Token::Atomic
                )
            })
            .cloned()
            .collect::<Vec<_>>();
        // Extract source info into locals first so the Ref guards are dropped before
        // parse_expression_tokens runs. If the arguments were inline borrows, they would
        // live until the end of the statement, causing a RefCell panic when the parser
        // tries to borrow_mut (e.g. to register an anonymous struct in sizeof).
        let expr = parse_expression_tokens(&tokens, subscription.1, self.clone());
        let mut base = self.base.borrow_mut();
        let id = base.array_len_consts.len();
        let registered = RegisteredArrayLenConst {
            id,
            expr: expr.clone(),
        };
        base.array_len_const_exprs
            .insert(id, registered.expr.clone());
        base.array_len_consts.insert(key, registered.clone());
        Some(registered)
    }
}

#[derive(Debug, Clone)]
pub enum DefOrLocal {
    Def {
        def_id: DefId,
        generic_args: Vec<Spanned<co2_ast::RustTy<LocalResolver>>>,
    },
    Const(DefId),
    LocalConst(u32),
    AssocMethod {
        receiver: DefId,
        method: String,
        receiver_generic_args: Vec<Spanned<co2_ast::RustTy<LocalResolver>>>,
    },
    Local(u32),
    FuncName,
    Prim(PrimitiveTy),
    UnrepresentableType(CTy),
    InlineRustTy(Box<co2_ast::RustTy<LocalResolver>>),
}

impl co2_ast::TypeResolver for LocalResolver {
    type ResolvedRustPath = DefOrLocal;
    type DeclarationIdent = (usize, String);
    type StructOrUnionIdentifier = DefId;
    type EnumIdentifier = DefId;
    type EnumeratorIdentifier = (DefId, String, Option<Spanned<Expression<Self>>>);
    type SubscriptionIdentifier = RegisteredSubscription;

    fn classify_path(
        &self,
        path: &co2_ast::RustPath<StatelessResolver>,
    ) -> Option<(TypeQueryResult, Self::ResolvedRustPath)> {
        let stripped_path: co2_ast::RustPath<StatelessResolver> = co2_ast::RustPath {
            segments: path
                .segments
                .iter()
                .filter_map(|(segment, span)| match segment {
                    co2_ast::RustPathSegment::Ident(ident) => {
                        Some((co2_ast::RustPathSegment::Ident(ident.clone()), *span))
                    }
                    co2_ast::RustPathSegment::Generics(_) => None,
                })
                .collect(),
        };
        let path_pretty = stripped_path.to_pretty();
        let generic_args = match path.segments.last() {
            Some((co2_ast::RustPathSegment::Generics(args), _)) => args
                .iter()
                .map(|arg| arg.transform(self))
                .collect::<Vec<_>>(),
            _ => vec![],
        };
        if ["__func__", "__PRETTY_FUNCTION__", "__FUNCTION__"].contains(&&*path_pretty) {
            return Some((TypeQueryResult::Expr, DefOrLocal::FuncName));
        }
        let base = self.base.borrow();
        if let Some(ty) = self
            .base
            .borrow()
            .unrepresentable_typedefs
            .get(&path_pretty)
        {
            return Some((
                TypeQueryResult::Type,
                DefOrLocal::UnrepresentableType(ty.clone()),
            ));
        }
        let (def, class) = self
            .locals
            .borrow()
            .get(&path_pretty)
            .cloned()
            .or_else(|| {
                let Some((co2_ast::RustPathSegment::Ident(method), _)) =
                    stripped_path.segments.last()
                else {
                    return None;
                };
                if stripped_path.segments.len() < 2 {
                    return None;
                }
                let receiver_end = path.segments.iter().rposition(|(segment, _)| {
                    matches!(segment, co2_ast::RustPathSegment::Ident(_))
                })?;
                let receiver_path = co2_ast::RustPath {
                    segments: path.segments[..receiver_end].to_vec(),
                };
                let (receiver_class, receiver) = self.classify_path(&receiver_path)?;
                if receiver_class != TypeQueryResult::Type {
                    return None;
                }
                let DefOrLocal::Def {
                    def_id: receiver,
                    generic_args: receiver_generic_args,
                } = receiver
                else {
                    return None;
                };
                Some((
                    DefOrLocal::AssocMethod {
                        receiver,
                        method: method.clone(),
                        receiver_generic_args,
                    },
                    TypeQueryResult::Expr,
                ))
            })
            .or_else(|| {
                match base
                    .resolver
                    .resolve_relative_expr_path(&self.module_path, &path_pretty)
                    .ok()?
                {
                    ResolvedExprPath::Def(def_id, class) => Some((
                        DefOrLocal::Def {
                            def_id,
                            generic_args: generic_args.clone(),
                        },
                        class,
                    )),
                    ResolvedExprPath::Const(def_id) => {
                        Some((DefOrLocal::Const(def_id), TypeQueryResult::Expr))
                    }
                }
            })
            .or_else(|| {
                PrimitiveTy::parse(&path_pretty)
                    .map(|prim| (DefOrLocal::Prim(prim), TypeQueryResult::Type))
            })
            .or_else(|| {
                // Handle ::<T> syntax: a standalone generic type specifier with no leading ident.
                // The single generic arg is the type itself (e.g., ::<*mut i32> declares *mut i32).
                if path_pretty.is_empty()
                    && let [(ty, _)] = generic_args.as_slice()
                {
                    return Some((
                        DefOrLocal::InlineRustTy(Box::new(ty.clone())),
                        TypeQueryResult::Type,
                    ));
                }
                None
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
            Declaration::FunctionDefinition { .. } | Declaration::RustTypeAlias { .. } => next,
            Declaration::PragmaPack { action } => {
                next.base.borrow_mut().apply_pack_action(action);
                next
            }
            Declaration::Declaration {
                declaration_specifiers,
                declarators,
            } => {
                let is_typedef = declaration_specifiers.iter().any(|d| d.0.is_typedef());
                let is_static = declaration_specifiers.iter().any(|d| d.0.is_static());
                let is_constexpr = declaration_specifiers.iter().any(|d| d.0.is_constexpr());
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
                        next.locals.borrow_mut().insert(
                            name.1,
                            (
                                DefOrLocal::Def {
                                    def_id,
                                    generic_args: vec![],
                                },
                                TypeQueryResult::Type,
                            ),
                        );
                    } else if is_static
                        && !decl
                            .0
                            .initializer
                            .as_ref()
                            .is_some_and(|init| initializer_contains_label_address(&init.0))
                    {
                        let mut base = next.base.borrow_mut();
                        let (def_id, fake_name) =
                            base.emit_fake_def(rustc_public_generative::DefData::ValueNs);
                        if is_constexpr
                            && let Some((co2_ast::Initializer::Expr(expr), _span)) =
                                decl.0.initializer.clone()
                        {
                            base.constexpr_def_exprs.insert(def_id, expr);
                        }
                        base.pending_static.push((
                            def_id,
                            fake_name,
                            declaration_specifiers.clone(),
                            decl.0.clone(),
                            decl.1,
                        ));
                        next.locals.borrow_mut().insert(
                            name.1,
                            (
                                DefOrLocal::Def {
                                    def_id,
                                    generic_args: vec![],
                                },
                                TypeQueryResult::Expr,
                            ),
                        );
                    } else if decl.0.declarator.0.is_function() {
                        // TODO: detect if we need to emit an extern function here.
                    } else {
                        if is_constexpr
                            && let Some((co2_ast::Initializer::Expr(expr), _span)) =
                                decl.0.initializer.clone()
                        {
                            next.base
                                .borrow_mut()
                                .constexpr_local_exprs
                                .insert(name.0 as u32, expr);
                        }
                        next.locals.borrow_mut().insert(
                            name.1,
                            (
                                if is_constexpr {
                                    DefOrLocal::LocalConst(name.0 as u32)
                                } else {
                                    DefOrLocal::Local(name.0 as u32)
                                },
                                TypeQueryResult::Expr,
                            ),
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
        self.collect_enum_constants(specifier, span)
    }

    fn register_subscription(
        &self,
        subscription: Spanned<co2_ast::LazySubscription>,
    ) -> Self::SubscriptionIdentifier {
        RegisteredSubscription {
            raw: subscription.0.clone(),
            array_len_const: self
                .register_array_len_const(&subscription)
                .map(|registered| registered.id),
        }
    }

    fn rust_style_syntax_enabled(&self) -> bool {
        !self.locals.borrow().contains_key("fn")
            && self
                .base
                .borrow()
                .resolver
                .resolve_in_current(["fn"])
                .is_err()
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

    fn transform_path(
        &self,
        (path, span): &Spanned<RustPath<StatelessResolver>>,
    ) -> Spanned<Self::ResolvedRustPath> {
        let Some(r) = self.classify_path(path) else {
            self.base
                .borrow()
                .terminate_with_error(*span, &format!("Unresolved name {path}"));
        };
        (r.1, *span)
    }

    fn transform_subscription(
        &self,
        subscription: &Spanned<<StatelessResolver as TypeResolver>::SubscriptionIdentifier>,
    ) -> Spanned<Self::SubscriptionIdentifier> {
        (
            self.register_subscription(subscription.clone()),
            subscription.1,
        )
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
