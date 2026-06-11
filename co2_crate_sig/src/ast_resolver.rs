use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::Arc,
};

use co2_ast::{
    Declaration, DeclarationSpecifier, Declarator, DoTransform as _, EnumSpecifier, Expression,
    InitDeclarator, Initializer, RustPath, RustTy, Span, Spanned, StatelessResolver,
    StructOrUnionSpecifier, TypeQueryResult, TypeResolver,
};
use co2_parser::parse_expression_tokens;
use co2_preprocessor::PreprocessedSource;
use rustc_public_generative::{
    DefData, FileId, HirStructureCtx,
    rustc_public::{
        DefId,
        mir::Mutability,
        ty::{Region, RegionKind, RigidTy, Ty},
    },
};
use rustc_public_generative::{HirTy, HirTyKind};

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
        Expression::MethodCall {
            receiver, params, ..
        } => {
            expr_contains_label_address(&receiver.0)
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
        Expression::VaCopy { dest, src } => {
            expr_contains_label_address(&dest.0) || expr_contains_label_address(&src.0)
        }
        Expression::VaArg { args, .. } => expr_contains_label_address(&args.0),
        Expression::BuiltinConstantP { expr } => expr_contains_label_address(&expr.0),
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
    pub array_len_const_counter: usize,
    pub pending_typedefs: Vec<(
        DefId,
        String,
        Vec<co2_ast::Spanned<DeclarationSpecifier<LocalResolver>>>,
        Declarator<LocalResolver>,
        co2_ast::Span,
        bool,
    )>,
    pub pending_static: Vec<(
        DefId,
        String,
        Vec<co2_ast::Spanned<DeclarationSpecifier<LocalResolver>>>,
        InitDeclarator<LocalResolver>,
        co2_ast::Span,
    )>,
    pub array_len_consts: HashMap<usize, RegisteredArrayLenConst>,
    pub array_len_const_exprs: HashMap<usize, co2_ast::Spanned<Expression<LocalResolver>>>,
    pub hir_ctx: &'static HirStructureCtx<'static>,
    pub file_id: FileId,
    pub preprocessed: Arc<PreprocessedSource>,
    pub file_ids: Arc<HashMap<co2_ast::FileId, FileId>>,
    pub(crate) struct_manager: StructManager,
    pub(crate) unrepresentable_typedefs: HashMap<String, CTy>,
    pub(crate) typedef_tys: HashMap<DefId, HirTy>,
    pub(crate) transparent_unions: HashSet<DefId>,
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
            .field("array_len_const_counter", &self.array_len_const_counter)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
pub struct RegisteredArrayLenConst {
    pub id: usize,
    pub def_id: DefId,
    pub rhs: DefId,
    pub name: String,
    pub span: co2_ast::Span,
    pub resolver: LocalResolver,
    pub expr: co2_ast::Spanned<Expression<LocalResolver>>,
}

#[derive(Debug, Clone)]
pub struct RegisteredSubscription {
    pub raw: co2_ast::LazySubscription,
    pub array_len_const: Option<usize>,
}

impl LocalResolverBase {
    pub(crate) fn mark_transparent_union(&mut self, ty: &HirTy) {
        let HirTyKind::Adt(def, _) = ty.kind else {
            return;
        };
        self.transparent_unions.insert(def);
    }

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
        self.array_len_consts
            .values()
            .find(|registered| registered.span == span)
            .cloned()
    }

    pub fn lookup_array_len_const_expr(
        &self,
        id: usize,
    ) -> Option<co2_ast::Spanned<Expression<LocalResolver>>> {
        self.array_len_const_exprs.get(&id).cloned()
    }

    pub fn lookup_array_len_const_by_id(&self, id: usize) -> Option<RegisteredArrayLenConst> {
        self.array_len_consts.get(&id).cloned()
    }

    pub fn lookup_array_len_const_by_def(&self, def_id: DefId) -> Option<RegisteredArrayLenConst> {
        self.array_len_consts
            .values()
            .find(|registered| registered.def_id == def_id)
            .cloned()
    }
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
    pub fn dependency_info(&self) -> rustc_public_generative::DependencyInfo<'static> {
        let guard = self.base.borrow();
        let hir_ctx: &'static HirStructureCtx<'static> = guard.hir_ctx;
        rustc_public_generative::DependencyInfo { tcx: hir_ctx.tcx }
    }

    pub fn normalize_ty_defaults(&self, ty: Ty) -> Ty {
        let guard = self.base.borrow();
        guard.hir_ctx.normalize_ty_defaults(ty)
    }

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

    pub fn dependency_const_value(
        &self,
        def_id: DefId,
    ) -> Option<rustc_public_generative::DependencyConstValue> {
        self.base.borrow().hir_ctx.dependency_const_value(def_id)
    }

    pub fn local_const_int_value(
        &self,
        def_id: DefId,
        span: co2_ast::Span,
    ) -> Result<i128, (co2_ast::Span, String)> {
        self.base.borrow_mut().eval_local_const(def_id, span)
    }

    pub fn has_local_const_value(&self, def_id: DefId) -> bool {
        self.base.borrow().has_local_const_value(def_id)
    }

    pub fn local_constexpr_int_value(
        &self,
        local: u32,
        span: co2_ast::Span,
    ) -> Result<i128, (co2_ast::Span, String)> {
        self.base.borrow_mut().eval_local_constexpr(local, span)
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

    pub fn is_transparent_union_def(&self, def_id: DefId) -> bool {
        self.base.borrow().transparent_unions.contains(&def_id)
    }

    pub fn c_adt_display_info(
        &self,
        def_id: DefId,
    ) -> Option<crate::struct_manager::CAdtDisplayInfo> {
        self.base.borrow().c_adt_display_info(def_id)
    }

    pub fn validate_constexpr_decl(
        &self,
        specifiers: &[Spanned<DeclarationSpecifier<LocalResolver>>],
        declarator: &Declarator<LocalResolver>,
        ty: &crate::CTy,
        initializer: Option<&Spanned<Initializer<LocalResolver>>>,
    ) -> Result<(), (co2_ast::Span, String)> {
        self.base
            .borrow_mut()
            .validate_constexpr_decl(specifiers, declarator, ty, initializer)
    }

    pub fn lookup_array_len_const_expr(
        &self,
        id: usize,
    ) -> Option<Spanned<Expression<LocalResolver>>> {
        self.base.borrow().lookup_array_len_const_expr(id)
    }

    pub fn eval_const_expr(
        &self,
        expr: &Spanned<Expression<LocalResolver>>,
    ) -> Result<i128, (co2_ast::Span, String)> {
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

    pub fn traits_in_scope_with_method(&self, method: &str) -> Vec<(String, DefId, String)> {
        self.base
            .borrow_mut()
            .resolver
            .traits_in_scope_with_method(method)
    }

    pub fn resolve_method(
        &self,
        receiver_ty: Ty,
        method: &str,
        span: co2_ast::Span,
        ufcs_trait: Option<DefId>,
    ) -> Result<Option<(DefId, TypeQueryResult, MethodResolutionKind)>, (co2_ast::Span, String)>
    {
        let trait_candidates = self
            .base
            .borrow_mut()
            .resolver
            .traits_in_scope_with_method(method);
        let trait_candidates_ref = trait_candidates.clone();
        let hir_ctx = self.base.borrow().hir_ctx;
        let mut applicable = Vec::new();
        for (trait_name, trait_def, trait_path) in trait_candidates {
            // If UFCS, only check the specified trait
            if let Some(ufcs_trait_def) = ufcs_trait {
                if trait_def != ufcs_trait_def {
                    continue;
                }
            }
            if hir_ctx.type_implements_trait(self.current_owner, receiver_ty, trait_def)
                && let Some((method_def, class)) = self
                    .base
                    .borrow_mut()
                    .resolver
                    .resolve_trait_method(&trait_path, method)
            {
                applicable.push((trait_name, method_def, class));
            }
        }

        match applicable.len() {
            0 => {
                // UFCS with specified trait: no fallback to auto-deref/auto-ref
                if ufcs_trait.is_some() {
                    return Ok(None);
                }
                // Auto-deref: resolve method on the pointee of Ref/RawPtr
                if let rustc_public_generative::rustc_public::ty::TyKind::RigidTy(
                    rustc_public_generative::rustc_public::ty::RigidTy::Ref(_, inner, _)
                    | rustc_public_generative::rustc_public::ty::RigidTy::RawPtr(inner, _),
                ) = receiver_ty.kind()
                {
                    return self.resolve_method(inner, method, span, None);
                }
                // Auto-ref: try &T and &mut T
                for mutability in [Mutability::Not, Mutability::Mut] {
                    let ref_ty = Ty::from_rigid_kind(RigidTy::Ref(
                        Region {
                            kind: RegionKind::ReStatic,
                        },
                        receiver_ty,
                        mutability,
                    ));
                    if let Some(found) = self
                        .base
                        .borrow_mut()
                        .resolver
                        .resolve_inherent_method_for_sig(ref_ty, method)
                    {
                        let (method_def, class) = found;
                        return Ok(Some((method_def, class, MethodResolutionKind::Inherent)));
                    }
                    let mut applicable = Vec::new();
                    for (trait_name, trait_def, trait_path) in trait_candidates_ref.clone() {
                        if (hir_ctx.type_implements_trait(self.current_owner, ref_ty, trait_def)
                            || hir_ctx.type_implements_trait(
                                self.current_owner,
                                receiver_ty,
                                trait_def,
                            ))
                            && let Some((method_def, class)) = self
                                .base
                                .borrow_mut()
                                .resolver
                                .resolve_trait_method(&trait_path, method)
                        {
                            applicable.push((trait_name, method_def, class));
                        }
                    }
                    match applicable.len() {
                        0 => {}
                        1 => {
                            let (_, method_def, class) = applicable.pop().unwrap();
                            return Ok(Some((method_def, class, MethodResolutionKind::Trait)));
                        }
                        _ => {
                            return Err((
                                span,
                                format!(
                                    "multiple traits in scope provide method `{method}` for receiver type {ref_ty:?}: {}",
                                    applicable
                                        .into_iter()
                                        .map(|(trait_name, _, _)| trait_name)
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                ),
                            ));
                        }
                    }
                }
                Ok(None)
            }
            1 => {
                let (_, method_def, class) = applicable.pop().unwrap();
                Ok(Some((method_def, class, MethodResolutionKind::Trait)))
            }
            _ => Err((
                span,
                format!(
                    "multiple traits in scope provide method `{method}` for receiver type {receiver_ty:?}: {}",
                    applicable
                        .into_iter()
                        .map(|(trait_name, _, _)| trait_name)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )),
        }
    }

    pub fn resolve_inherent_method(
        &self,
        receiver_ty: Ty,
        method: &str,
        span: co2_ast::Span,
    ) -> Result<Option<rustc_public_generative::ResolvedMethod>, (co2_ast::Span, String)> {
        self.dependency_info()
            .resolve_inherent_method(self.current_owner, receiver_ty, method)
            .map_err(|msg| (span, msg))
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
        let id = base.array_len_const_counter;
        base.array_len_const_counter += 1;
        let name = format!("__co2_array_len_{id}");
        let def_id = base.hir_ctx.allocate_def_id(
            base.hir_ctx.root_crate_def_id(),
            &DefData::ValueNs(name.clone()),
        );
        let rhs = base.hir_ctx.allocate_def_id(def_id, &DefData::AnonConst);
        let registered = RegisteredArrayLenConst {
            id,
            def_id,
            rhs,
            name,
            span: subscription.1,
            resolver: self.clone(),
            expr: expr.clone(),
        };
        base.array_len_const_exprs
            .insert(id, registered.expr.clone());
        base.array_len_consts.insert(id, registered.clone());
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
        receiver_generic_args: Vec<Spanned<RustTy<LocalResolver>>>,
        ufcs_trait: Option<DefId>,
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
    ) -> Result<(TypeQueryResult, Self::ResolvedRustPath), (String, Span)> {
        let stripped_path: co2_ast::RustPath<StatelessResolver> = co2_ast::RustPath {
            segments: path
                .segments
                .iter()
                .filter_map(|(segment, span)| match segment {
                    co2_ast::RustPathSegment::Ident(ident) => {
                        Some((co2_ast::RustPathSegment::Ident(ident.clone()), *span))
                    }
                    co2_ast::RustPathSegment::Generics(_)
                    | co2_ast::RustPathSegment::Qualified { .. } => None,
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
            return Ok((TypeQueryResult::Expr, DefOrLocal::FuncName));
        }
        if let Some(ty) = self
            .base
            .borrow()
            .unrepresentable_typedefs
            .get(&path_pretty)
        {
            return Ok((
                TypeQueryResult::Type,
                DefOrLocal::UnrepresentableType(ty.clone()),
            ));
        }
        let base_resolve = self
            .base
            .borrow_mut()
            .resolver
            .resolve_relative_expr_path(&self.module_path, &path_pretty);
        let expr_path_result = base_resolve.map(|res| match res {
            ResolvedExprPath::Def(def_id, class) => (
                DefOrLocal::Def {
                    def_id,
                    generic_args: generic_args.clone(),
                },
                class,
            ),
        });
        let has_direct_expr_path = expr_path_result.is_some();
        let Some((def, class)) = self
            .locals
            .borrow()
            .get(&path_pretty)
            .cloned()
            .or_else(|| {
                // UFCS: <Type as Trait>::method
                if stripped_path.segments.len() == 1
                    && let Some((co2_ast::RustPathSegment::Ident(method), _)) =
                        stripped_path.segments.last()
                    && let Some((
                        co2_ast::RustPathSegment::Qualified {
                            type_segments,
                            trait_segments,
                        },
                        _,
                    )) = path.segments.first()
                {
                    let type_path = co2_ast::RustPath {
                        segments: type_segments.clone(),
                    };
                    let (receiver_class, receiver) = self.classify_path(&type_path).ok()?;
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
                    let trait_path = co2_ast::RustPath {
                        segments: trait_segments.clone(),
                    };
                    let ufcs_trait =
                        self.classify_path(&trait_path)
                            .ok()
                            .and_then(|(class, def)| {
                                if class == TypeQueryResult::Type {
                                    match def {
                                        DefOrLocal::Def { def_id, .. } => Some(def_id),
                                        _ => None,
                                    }
                                } else {
                                    None
                                }
                            });
                    return Some((
                        DefOrLocal::AssocMethod {
                            receiver,
                            method: method.clone(),
                            receiver_generic_args,
                            ufcs_trait,
                        },
                        TypeQueryResult::Expr,
                    ));
                }
                None
            })
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
                let (receiver_class, receiver) = self.classify_path(&receiver_path).ok()?;
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
                if receiver_generic_args.is_empty() && has_direct_expr_path {
                    // For non-trait receivers (inherent methods on structs),
                    // the direct expr path is sufficient. For trait receivers,
                    // we need AssocMethod handling to infer concrete Self type.
                    if !self.dependency_info().is_trait(receiver) {
                        return None;
                    }
                }
                Some((
                    DefOrLocal::AssocMethod {
                        receiver,
                        method: method.clone(),
                        receiver_generic_args,
                        ufcs_trait: None,
                    },
                    TypeQueryResult::Expr,
                ))
            })
            .or(expr_path_result)
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
            })
        else {
            let span = path.segments.first().unwrap().1;
            return Err((format!("Unresolved name {path}"), span));
        };
        if !generic_args.is_empty() {
            let span = path.segments.last().unwrap().1;
            if matches!(def, DefOrLocal::Local(_) | DefOrLocal::FuncName) {
                return Err((
                    "type arguments are not allowed on local variable".to_owned(),
                    span,
                ));
            }
            if matches!(
                def,
                DefOrLocal::Prim(_) | DefOrLocal::Const(_) | DefOrLocal::UnrepresentableType(_)
            ) {
                return Err((
                    format!("type arguments are not allowed on `{path_pretty}`"),
                    span,
                ));
            }
        }
        Ok((class, def))
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
            Declaration::FunctionDefinition { .. }
            | Declaration::RustTypeAlias { .. }
            | Declaration::RustStruct { .. }
            | Declaration::BreakCo2 => next,
            Declaration::PragmaPack { action } => {
                next.base.borrow_mut().apply_pack_action(action);
                next
            }
            Declaration::Declaration {
                attrs: _,
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
                            decl.0.is_transparent_union,
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
                .borrow_mut()
                .resolver
                .resolve_in_current(["fn"])
                .is_none()
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
        let r = match self.classify_path(path) {
            Ok(r) => r,
            Err((msg, span)) => {
                self.base.borrow().terminate_with_error(span, &msg);
            }
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
