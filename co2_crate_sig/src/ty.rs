use std::collections::HashSet;

use co2_ast::{
    BinOp, Constant, DeclarationSpecifier, Declarator, Expression, GenericAssociation, Initializer,
    IntegerSuffix, Span, Spanned, StorageClassSpecifier, StringLiteralPrefix, StructOrUnionKind,
    TypeName, TypeQualifier, TypeResolver, TypeSpecifier, UnaryOp,
};
use rustc_public_generative::{
    FunctionAbi, FunctionInput, FunctionSignature, HirTy, HirTyConst, HirTyKind,
};
use rustc_public_generative::{
    HirGenericArg,
    rustc_public::{
        CrateItem, DefId,
        mir::Mutability,
        ty::{FloatTy, GenericArgKind, GenericArgs, IntTy, RigidTy, TyKind, UintTy},
    },
};

use crate::{CrateSigCtx, LocalResolver, LocalResolverBase, LogicalAdtFieldKind};

#[derive(Debug, Clone)]
pub enum CTy {
    Ty(HirTy),
    Function(FunctionSignature),
    UnsizedArray(HirTy),
}

pub enum CompressedTypeSpecifier {
    Void,
    PrimitiveTy(PrimitiveTy),
    StructOrUnion {
        kind: StructOrUnionKind,
        specifier: Spanned<<LocalResolver as TypeResolver>::StructOrUnionIdentifier>,
    },
    Enum(Spanned<<LocalResolver as TypeResolver>::EnumIdentifier>),
    TypedefName(Spanned<<LocalResolver as TypeResolver>::ResolvedRustPath>),
    TypeofType(TypeName<LocalResolver>),
    TypeofExpr(Spanned<Expression<LocalResolver>>),
    Auto,
}

fn spanned_error(span: Span, msg: impl Into<String>) -> (Span, String) {
    (span, msg.into())
}

impl CompressedTypeSpecifier {
    pub fn build(
        specifiers: Vec<Spanned<DeclarationSpecifier<LocalResolver>>>,
    ) -> Result<Self, (co2_ast::Span, String)> {
        enum Base {
            Int,
            Double,
            Char,
        }
        let has_auto = specifiers.iter().any(|(spec, _)| {
            matches!(
                spec,
                DeclarationSpecifier::StorageSpecifier((StorageClassSpecifier::Auto, _))
            )
        });
        let span = type_specifier_span(&specifiers).unwrap_or_else(|| {
            specifiers
                .first()
                .map(|(_, span)| *span)
                .expect("compressed type specifier needs at least one specifier")
        });
        let specifiers = specifiers
            .into_iter()
            .filter_map(|x| match x.0 {
                DeclarationSpecifier::TypeSpecifier(s) => Some(s.0),
                DeclarationSpecifier::TypeQualifier(_)
                | DeclarationSpecifier::StorageSpecifier(_)
                | DeclarationSpecifier::FunctionSpecifier(_) => None,
            })
            .collect::<Vec<_>>();
        if specifiers.is_empty() {
            if has_auto {
                return Ok(CompressedTypeSpecifier::Auto);
            }
            return Err(spanned_error(span, "no type specifier found"));
        }
        if let [specifier] = specifiers.as_slice() {
            'b: {
                return Ok(match specifier {
                    TypeSpecifier::Void => CompressedTypeSpecifier::Void,
                    TypeSpecifier::Float => {
                        CompressedTypeSpecifier::PrimitiveTy(PrimitiveTy::FloatTy(FloatTy::F32))
                    }
                    TypeSpecifier::Bool => CompressedTypeSpecifier::PrimitiveTy(PrimitiveTy::Bool),
                    &TypeSpecifier::StructOrUnion { kind, specifier } => {
                        CompressedTypeSpecifier::StructOrUnion { kind, specifier }
                    }
                    &TypeSpecifier::Enum(e) => CompressedTypeSpecifier::Enum(e),
                    TypeSpecifier::TypedefName(t) => {
                        CompressedTypeSpecifier::TypedefName(t.clone())
                    }
                    TypeSpecifier::TypeofType(type_name) => {
                        CompressedTypeSpecifier::TypeofType((**type_name).clone())
                    }
                    TypeSpecifier::TypeofExpr(expr) => {
                        CompressedTypeSpecifier::TypeofExpr((**expr).clone())
                    }
                    _ => break 'b,
                });
            }
        }
        let mut base = None;
        let mut signed = None;
        let mut long = 0u32;
        let mut short = 0u32;
        for spec in specifiers {
            match spec {
                TypeSpecifier::Int | TypeSpecifier::Char | TypeSpecifier::Double => {
                    if base.is_some() {
                        return Err(spanned_error(span, "duplicate base specifier found"));
                    }
                    base = Some(match spec {
                        TypeSpecifier::Int => Base::Int,
                        TypeSpecifier::Char => Base::Char,
                        TypeSpecifier::Double => Base::Double,
                        _ => unreachable!(),
                    });
                }
                TypeSpecifier::Short => short += 1,
                TypeSpecifier::Long => long += 1,
                TypeSpecifier::Signed | TypeSpecifier::Unsigned => {
                    if base.is_some() {
                        return Err(spanned_error(span, "duplicate sign specifier found"));
                    }
                    signed = Some(matches!(spec, TypeSpecifier::Signed));
                }
                TypeSpecifier::Alignas => {}
                TypeSpecifier::Bool
                | TypeSpecifier::Void
                | TypeSpecifier::Float
                | TypeSpecifier::StructOrUnion { .. }
                | TypeSpecifier::Enum(_)
                | TypeSpecifier::TypedefName(_)
                | TypeSpecifier::TypeofType(_)
                | TypeSpecifier::TypeofExpr(_) => {
                    return Err(spanned_error(span, "This specifier should be used alone"));
                }
            }
        }
        let base = base.unwrap_or(Base::Int);
        Ok(CompressedTypeSpecifier::PrimitiveTy(match base {
            Base::Int => {
                let signed = signed.unwrap_or(true);
                match (long, short, signed) {
                    (1.., 1.., _) => return Err(spanned_error(span, "Mixed short and long")),
                    (3.., _, _) => {
                        return Err(spanned_error(span, "long repeated too many times"));
                    }
                    (_, 2.., _) => {
                        return Err(spanned_error(span, "short repeated too many times"));
                    }
                    (0, 0, true) => PrimitiveTy::IntTy(IntTy::I32),
                    (0, 0, false) => PrimitiveTy::UintTy(UintTy::U32),
                    (0, 1, true) => PrimitiveTy::IntTy(IntTy::I16),
                    (0, 1, false) => PrimitiveTy::UintTy(UintTy::U16),
                    (1..=2, 0, true) => PrimitiveTy::IntTy(IntTy::I64),
                    (1..=2, 0, false) => PrimitiveTy::UintTy(UintTy::U64),
                }
            }
            Base::Double => {
                if short > 0 {
                    return Err(spanned_error(span, "short double is invalid"));
                }
                if signed.is_some() {
                    return Err(spanned_error(span, "signedness for double is invalid"));
                }
                PrimitiveTy::FloatTy(if long > 0 {
                    FloatTy::F128
                } else {
                    FloatTy::F64
                })
            }
            Base::Char => {
                if short > 0 || long > 0 {
                    return Err(spanned_error(span, "short and long char is invalid"));
                }
                match signed {
                    Some(true) | None => PrimitiveTy::IntTy(IntTy::I8),
                    Some(false) => PrimitiveTy::UintTy(UintTy::U8),
                }
            }
        }))
    }
}

fn has_const_qualifier_in_decl_specs(
    specs: &[Spanned<DeclarationSpecifier<LocalResolver>>],
) -> bool {
    specs.iter().any(|(spec, _)| {
        matches!(
            spec,
            DeclarationSpecifier::TypeQualifier((TypeQualifier::Const, _))
        )
    })
}

fn type_specifier_span(specs: &[Spanned<DeclarationSpecifier<LocalResolver>>]) -> Option<Span> {
    let mut spans = specs.iter().filter_map(|(spec, _)| match spec {
        DeclarationSpecifier::TypeSpecifier((_, span)) => Some(*span),
        _ => None,
    });
    let first = spans.next()?;
    let first_data = first.data();
    let mut start = first_data.start;
    let mut end = first_data.end;
    let first_context = first_data.context;
    for span in spans {
        let data = span.data();
        if data.context != first_context {
            continue;
        }
        start = start.min(data.start);
        end = end.max(data.end);
    }
    Some(Span::from_parts(first_context, start..end))
}

fn has_constexpr_storage_specifier(specs: &[Spanned<DeclarationSpecifier<LocalResolver>>]) -> bool {
    specs.iter().any(|(spec, _)| spec.is_constexpr())
}

fn has_volatile_qualifier_in_decl_specs(
    specs: &[Spanned<DeclarationSpecifier<LocalResolver>>],
) -> Option<Span> {
    specs
        .iter()
        .find(|(spec, _)| {
            matches!(
                spec,
                DeclarationSpecifier::TypeQualifier((TypeQualifier::Volatile, _))
            )
        })
        .map(|x| x.1)
}

fn has_atomic_storage_specifier(
    specs: &[Spanned<DeclarationSpecifier<LocalResolver>>],
) -> Option<Span> {
    specs
        .iter()
        .find(|(spec, _)| {
            matches!(
                spec,
                DeclarationSpecifier::StorageSpecifier((StorageClassSpecifier::Atomic, _))
                    | DeclarationSpecifier::TypeQualifier((TypeQualifier::Atomic, _))
            )
        })
        .map(|x| x.1)
}

fn declarator_has_restrict_qualifier(decl: &Declarator<LocalResolver>) -> Option<Span> {
    match decl {
        Declarator::Abstract | Declarator::Identifier(_) => None,
        Declarator::FunctionDeclarator { declarator, .. }
        | Declarator::ArrayDeclarator { declarator, .. } => {
            declarator_has_restrict_qualifier(&declarator.0)
        }
        Declarator::PointerDeclarator {
            declarator,
            qualifiers,
        } => qualifiers
            .iter()
            .find(|(qualifier, _)| matches!(qualifier, TypeQualifier::Restrict))
            .map(|x| x.1)
            .or_else(|| declarator_has_restrict_qualifier(&declarator.0)),
    }
}

impl CrateSigCtx<'_> {
    pub(crate) fn base_ty_of_decl(
        &mut self,
        specifiers: Vec<Spanned<DeclarationSpecifier<LocalResolver>>>,
        parser_span: Span,
    ) -> CTy {
        self.resolver
            .borrow_mut()
            .base_ty_of_decl(specifiers, parser_span)
    }

    pub(crate) fn lower_function_signature(
        &mut self,
        base: CTy,
        base_const: bool,
        declarator: Spanned<Declarator<LocalResolver>>,
    ) -> Result<(String, FunctionSignature), (co2_ast::Span, String)> {
        self.resolver
            .borrow_mut()
            .lower_function_signature(base, base_const, declarator)
    }

    pub(crate) fn lower_value_decl_ctype(
        &mut self,
        base: CTy,
        base_const: bool,
        declarator: Spanned<Declarator<LocalResolver>>,
    ) -> (
        String,
        CTy,
        Option<co2_ast::Spanned<co2_ast::Initializer<LocalResolver>>>,
    ) {
        let span = declarator.1;
        match self.extract_decl_type_with_consts(base, base_const, declarator) {
            Ok((ty, name, array_len)) => {
                let Some(name) = name else {
                    CrateSigCtx::<'_>::terminate_with_error(span, "Unexpected abstract declarator");
                };
                (name, ty, array_len)
            }
            Err(err) => CrateSigCtx::<'_>::terminate_with_spanned_error(err),
        }
    }

    /// TODO: This function is duplicate with [`LocalResolverBase::extract_decl_type`]
    fn extract_decl_type_with_consts(
        &mut self,
        current: CTy,
        current_const: bool,
        (decl, span): Spanned<Declarator<LocalResolver>>,
    ) -> Result<
        (
            CTy,
            Option<String>,
            Option<co2_ast::Spanned<co2_ast::Initializer<LocalResolver>>>,
        ),
        (co2_ast::Span, String),
    > {
        let rust_span = self.co2_span_to_rustc(span);
        match decl {
            Declarator::Abstract => Ok((current, None, None)),
            Declarator::Identifier((ident, _)) => Ok((current, Some(ident.1), None)),
            Declarator::FunctionDeclarator {
                declarator,
                param_list,
            } => {
                let mut inputs = Vec::with_capacity(param_list.parameters.len());
                let c_variadic = param_list.effective_ellipsis();
                if !param_list.empty_params() {
                    for param in param_list.parameters {
                        let param_decl = param.1;
                        let param_base_const = has_const_qualifier_in_decl_specs(&param.0);
                        let param_base = self.base_ty_of_decl(param.0, param_decl.1);
                        let (param_decl_ty, _) = self.resolver.borrow_mut().extract_decl_type(
                            param_base,
                            param_base_const,
                            param_decl,
                        )?;
                        let param_ty = match param_decl_ty {
                            CTy::Ty(ty) => {
                                let peeled_ty =
                                    self.resolver.borrow().peel_constexpr_typedef(ty.clone());
                                if let HirTyKind::Array(_, inner) = peeled_ty.kind {
                                    HirTy::new_ptr(*inner, Mutability::Mut, ty.span)
                                } else if let HirTyKind::Array(_, inner) = ty.kind {
                                    HirTy::new_ptr(*inner, Mutability::Mut, ty.span)
                                } else {
                                    ty
                                }
                            }
                            CTy::Function(sig) => self.resolver.borrow_mut().maybe_uninit_of(
                                HirTy {
                                    kind: HirTyKind::FnPtr(Box::new(sig)),
                                    span: rust_span,
                                },
                                rust_span,
                                span,
                            ),
                            CTy::UnsizedArray(elem) => {
                                let span = elem.span;
                                HirTy::new_ptr(elem, Mutability::Mut, span)
                            }
                        };
                        inputs.push(FunctionInput {
                            name: None,
                            ty: param_ty,
                        });
                    }
                }
                let function_ty = match current {
                    CTy::Ty(ret) => CTy::Function(FunctionSignature {
                        lifetimes: vec![],
                        inputs,
                        output: ret,
                        abi: FunctionAbi::C,
                        is_unsafe: false,
                        c_variadic,
                    }),
                    CTy::Function(_) => {
                        return Err(spanned_error(
                            span,
                            "function returning function is not valid",
                        ));
                    }
                    CTy::UnsizedArray(_) => {
                        return Err(spanned_error(
                            span,
                            "function returning unsized array is not valid",
                        ));
                    }
                };
                self.extract_decl_type_with_consts(function_ty, false, *declarator)
            }
            Declarator::PointerDeclarator {
                declarator,
                qualifiers,
            } => {
                let ptr_mutability = if current_const {
                    Mutability::Not
                } else {
                    Mutability::Mut
                };
                let ptr_or_fn_ptr = match current {
                    CTy::Ty(inner) => CTy::Ty(HirTy::new_ptr(inner, ptr_mutability, rust_span)),
                    CTy::Function(sig) => CTy::Ty(self.resolver.borrow_mut().maybe_uninit_of(
                        HirTy {
                            kind: HirTyKind::FnPtr(Box::new(sig)),
                            span: rust_span,
                        },
                        rust_span,
                        span,
                    )),
                    CTy::UnsizedArray(inner) => CTy::Ty(HirTy::new_ptr(
                        HirTy::new_ptr(inner, Mutability::Mut, rust_span),
                        ptr_mutability,
                        rust_span,
                    )),
                };
                let next_const = qualifiers
                    .iter()
                    .any(|(q, _)| matches!(q, TypeQualifier::Const));
                self.extract_decl_type_with_consts(ptr_or_fn_ptr, next_const, *declarator)
            }
            Declarator::ArrayDeclarator {
                declarator,
                subscription,
            } => {
                let inner = match current {
                    CTy::Ty(inner) => inner,
                    CTy::Function(_) => {
                        return Err(spanned_error(span, "array of functions is not valid"));
                    }
                    CTy::UnsizedArray(_) => {
                        return Err(spanned_error(span, "array of unsized arrays is not valid"));
                    }
                };
                let len = if let Some(len) = subscription.0.raw.constant_len() {
                    (HirTyConst::Literal(len as usize), None)
                } else if subscription.0.raw.is_unsized() || subscription.0.raw.is_unspecified_vla()
                {
                    return self.extract_decl_type_with_consts(
                        CTy::UnsizedArray(inner),
                        current_const,
                        *declarator,
                    );
                } else {
                    let registered = subscription.0.array_len_const.as_ref().ok_or_else(|| {
                        spanned_error(subscription.1, "missing registered array size constant")
                    })?;
                    let registered = self
                        .resolver
                        .borrow()
                        .lookup_array_len_const_by_id(*registered)
                        .ok_or_else(|| {
                            spanned_error(subscription.1, "missing registered array size constant")
                        })?;
                    let literal_len = self
                        .eval_array_len_expr(&registered.expr)
                        .unwrap_or_else(|err| CrateSigCtx::<'_>::terminate_with_spanned_error(err));
                    let len_expr = (
                        Expression::Constant(Constant::Int(
                            literal_len as i128,
                            IntegerSuffix::None,
                        )),
                        registered.span,
                    );
                    (
                        HirTyConst::Literal(literal_len),
                        Some((Initializer::Expr(len_expr), registered.span)),
                    )
                };
                let array_ty = CTy::Ty(HirTy::new_array(inner, len.0, rust_span));
                let (ty, name, nested_len) =
                    self.extract_decl_type_with_consts(array_ty, current_const, *declarator)?;
                Ok((ty, name, nested_len.or(len.1)))
            }
        }
    }

    pub(crate) fn lower_rust_function_signature(
        &mut self,
        sig: co2_ast::RustFunctionSignature<LocalResolver>,
    ) -> (String, FunctionSignature) {
        let name = sig.name.0.1;
        let output = self.lower_rust_ty(sig.ret_ty);
        let mut inputs = Vec::new();
        for param in sig.params {
            inputs.push(FunctionInput {
                name: Some(param.name.0.1),
                ty: self.lower_rust_ty(param.ty),
            });
        }
        (
            name,
            FunctionSignature {
                lifetimes: vec![],
                inputs,
                output,
                abi: FunctionAbi::Rust,
                is_unsafe: false,
                c_variadic: false,
            },
        )
    }

    pub(crate) fn lower_rust_ty(
        &mut self,
        (ty, span): co2_ast::Spanned<co2_ast::RustTy<LocalResolver>>,
    ) -> HirTy {
        let rust_span = self.co2_span_to_rustc(span);
        match ty {
            co2_ast::RustTy::Path((path, _)) => self
                .resolver
                .borrow_mut()
                .hir_ty_of_resolved_path(&path, span),
            co2_ast::RustTy::Ptr { mutable, inner } => {
                let inner = self.lower_rust_ty(*inner);
                HirTy::new_ptr(
                    inner,
                    if mutable {
                        Mutability::Mut
                    } else {
                        Mutability::Not
                    },
                    rust_span,
                )
            }
            co2_ast::RustTy::Ref { mutable, inner } => {
                let inner = self.lower_rust_ty(*inner);
                // FIXME: lifetime
                HirTy::new_ref(
                    inner,
                    if mutable {
                        Mutability::Mut
                    } else {
                        Mutability::Not
                    },
                    rustc_public_generative::HirLifetime::Static,
                    rust_span,
                )
            }
            co2_ast::RustTy::Tuple(elems) => {
                let elems = elems.into_iter().map(|e| self.lower_rust_ty(e)).collect();
                HirTy::new_tuple(elems, rust_span)
            }
            co2_ast::RustTy::Never => HirTy {
                kind: HirTyKind::Never,
                span: rust_span,
            },
            _ => todo!("lower other rust types"),
        }
    }

    pub(crate) fn eval_array_len_expr(
        &mut self,
        expr: &Spanned<Expression<LocalResolver>>,
    ) -> Result<usize, (co2_ast::Span, String)> {
        let value = self.eval_const_expr(expr)?;
        usize::try_from(value).map_err(|_| {
            spanned_error(
                expr.1,
                format!("array size must be a non-negative integer, got {value}"),
            )
        })
    }

    pub(crate) fn eval_const_expr(
        &mut self,
        expr: &Spanned<Expression<LocalResolver>>,
    ) -> Result<i128, (co2_ast::Span, String)> {
        self.resolver.borrow_mut().eval_const_expr(expr)
    }
}

fn round_up(value: usize, align: usize) -> usize {
    if align <= 1 {
        value
    } else {
        value.div_ceil(align) * align
    }
}

enum ResolvedConstFieldAccess {
    Direct { path: Vec<usize> },
    Bitfield,
}

impl LocalResolverBase {
    pub(crate) fn has_local_enum_const(&self, def_id: DefId) -> bool {
        self.enum_const_values.contains_key(&def_id)
            || self
                .struct_manager
                .pending_enum_consts
                .iter()
                .any(|pending| pending.def_id == def_id)
    }

    pub(crate) fn has_local_const_value(&self, def_id: DefId) -> bool {
        self.has_local_enum_const(def_id) || self.constexpr_def_exprs.contains_key(&def_id)
    }

    pub(crate) fn has_local_constexpr(&self, local: u32) -> bool {
        self.constexpr_local_exprs.contains_key(&local)
    }

    pub(crate) fn is_constexpr_def(&self, def_id: DefId) -> bool {
        self.constexpr_def_exprs.contains_key(&def_id)
    }

    pub(crate) fn peel_constexpr_typedef(&self, mut ty: HirTy) -> HirTy {
        let mut seen = std::collections::HashSet::new();
        loop {
            let HirTyKind::Adt(def, _) = ty.kind else {
                return ty;
            };
            if !seen.insert(def) {
                return ty;
            }
            let Some(next_ty) = self.typedef_tys.get(&def).cloned() else {
                return ty;
            };
            ty = next_ty;
        }
    }

    pub(crate) fn validate_constexpr_decl(
        &mut self,
        specifiers: &[Spanned<DeclarationSpecifier<LocalResolver>>],
        declarator: &Declarator<LocalResolver>,
        ty: &CTy,
        initializer: Option<&Spanned<Initializer<LocalResolver>>>,
    ) -> Result<(), (co2_ast::Span, String)> {
        let span = type_specifier_span(specifiers)
            .or_else(|| initializer.map(|(_, span)| *span))
            .expect("constexpr declaration must have a span");
        if !has_constexpr_storage_specifier(specifiers) {
            return Ok(());
        }

        let Some(initializer) = initializer else {
            return Err(spanned_error(span, "`constexpr` requires an initializer"));
        };
        let Initializer::Expr(expr) = &initializer.0 else {
            return Err(spanned_error(
                span,
                "`constexpr` object type must be scalar",
            ));
        };

        if let Some(span) = has_volatile_qualifier_in_decl_specs(specifiers) {
            return Err(spanned_error(
                span,
                "`constexpr` object type cannot be volatile-qualified",
            ));
        }
        if let Some(span) = has_atomic_storage_specifier(specifiers) {
            return Err(spanned_error(
                span,
                "`constexpr` object type cannot be atomic",
            ));
        }
        if let Some(span) = declarator_has_restrict_qualifier(declarator) {
            return Err(spanned_error(
                span,
                "`constexpr` object type cannot be restrict-qualified",
            ));
        }

        match ty {
            CTy::Ty(ty) => {
                let peeled = self.peel_constexpr_typedef(ty.clone());
                match peeled.kind {
                    HirTyKind::Bool
                    | HirTyKind::Char
                    | HirTyKind::Int(_)
                    | HirTyKind::Uint(_)
                    | HirTyKind::Float(_)
                    | HirTyKind::RawPtr(_, _) => {}
                    HirTyKind::Adt(def, _) if self.is_enum_def(def) => {}
                    _ => {
                        return Err(spanned_error(
                            span,
                            "`constexpr` object type must be scalar",
                        ));
                    }
                }
            }
            CTy::Function(_) => {
                return Err(spanned_error(
                    span,
                    "`constexpr` object type must be scalar",
                ));
            }
            CTy::UnsizedArray(elem_ty) => {
                // Unsized arrays are allowed if they have initializers (they get sized from the initializer)
                // The element type must be scalar or a typedef to a scalar
                let peeled = self.peel_constexpr_typedef(elem_ty.clone());
                match peeled.kind {
                    HirTyKind::Bool
                    | HirTyKind::Char
                    | HirTyKind::Int(_)
                    | HirTyKind::Uint(_)
                    | HirTyKind::Float(_) => {}
                    HirTyKind::Adt(def, _) if self.is_enum_def(def) => {}
                    _ => {
                        return Err(spanned_error(
                            span,
                            "`constexpr` object type must be scalar",
                        ));
                    }
                }
            }
        }

        if matches!(
            ty,
            CTy::Ty(HirTy {
                kind: HirTyKind::RawPtr(_, _),
                ..
            })
        ) && !Self::is_null_pointer_constexpr_expr(expr)
        {
            return Err(spanned_error(
                expr.1,
                "`constexpr` pointer initializer must be null",
            ));
        }

        if !matches!(
            ty,
            CTy::Ty(HirTy {
                kind: HirTyKind::RawPtr(_, _),
                ..
            })
        ) && !matches!(ty, CTy::UnsizedArray(_))
            && self.eval_const_expr(expr).is_err()
        {
            return Err(spanned_error(
                expr.1,
                "`constexpr` initializer must be a constant expression",
            ));
        }

        Ok(())
    }

    fn is_null_pointer_constexpr_expr((expr, _): &Spanned<Expression<LocalResolver>>) -> bool {
        match expr {
            Expression::Constant(Constant::Int(0, _)) => true,
            Expression::Cast { expr, .. } => Self::is_null_pointer_constexpr_expr(expr),
            _ => false,
        }
    }

    fn hir_ty_of_rust_ty(
        &mut self,
        (ty, span): co2_ast::Spanned<co2_ast::RustTy<LocalResolver>>,
    ) -> HirTy {
        let rust_span = self.co2_span_to_rustc(span);
        match ty {
            co2_ast::RustTy::Path((path, path_span)) => {
                self.hir_ty_of_resolved_path(&path, path_span)
            }
            co2_ast::RustTy::Ptr { mutable, inner } => {
                let inner = self.hir_ty_of_rust_ty(*inner);
                HirTy::new_ptr(
                    inner,
                    if mutable {
                        Mutability::Mut
                    } else {
                        Mutability::Not
                    },
                    rust_span,
                )
            }
            co2_ast::RustTy::Ref { mutable, inner } => {
                let inner = self.hir_ty_of_rust_ty(*inner);
                HirTy::new_ref(
                    inner,
                    if mutable {
                        Mutability::Mut
                    } else {
                        Mutability::Not
                    },
                    rustc_public_generative::HirLifetime::Static,
                    rust_span,
                )
            }
            co2_ast::RustTy::Tuple(elems) => {
                let elems = elems
                    .into_iter()
                    .map(|e| self.hir_ty_of_rust_ty(e))
                    .collect();
                HirTy::new_tuple(elems, rust_span)
            }
            co2_ast::RustTy::Array { inner, len } => {
                let Some(len) = len.0.constant_len() else {
                    panic!("unsupported non-literal Rust array generic argument")
                };
                let len =
                    usize::try_from(len).expect("array generic argument length should fit usize");
                HirTy::new_array(
                    self.hir_ty_of_rust_ty(*inner),
                    HirTyConst::Literal(len),
                    rust_span,
                )
            }
            co2_ast::RustTy::BareFn { params, ret_ty } => {
                let inputs = params
                    .into_iter()
                    .map(|param| FunctionInput {
                        name: None,
                        ty: self.hir_ty_of_rust_ty(param),
                    })
                    .collect();
                let output = self.hir_ty_of_rust_ty(*ret_ty);
                HirTy {
                    kind: HirTyKind::FnPtr(Box::new(FunctionSignature {
                        lifetimes: vec![],
                        inputs,
                        output,
                        abi: FunctionAbi::Rust,
                        is_unsafe: false,
                        c_variadic: false,
                    })),
                    span: rust_span,
                }
            }
            co2_ast::RustTy::Never => HirTy {
                kind: HirTyKind::Never,
                span: rust_span,
            },
            co2_ast::RustTy::Slice(_) => panic!("slice generic arguments are not supported here"),
            co2_ast::RustTy::Wild => {
                panic!("Wild generic argument is not supported in crate signature context")
            }
            co2_ast::RustTy::Lifetime(_) => {
                panic!("lifetime should be handled by hir_generic_args_of_resolved_path")
            }
        }
    }

    fn hir_generic_args_of_resolved_path(
        &mut self,
        generic_args: &[Spanned<co2_ast::RustTy<LocalResolver>>],
    ) -> Vec<HirGenericArg> {
        generic_args
            .iter()
            .map(|arg| match &arg.0 {
                co2_ast::RustTy::Lifetime((name, _)) => {
                    if name == "static" {
                        HirGenericArg::Lifetime(rustc_public_generative::HirLifetime::Static)
                    } else {
                        self.terminate_with_spanned_error((
                            arg.1,
                            format!("unknown lifetime '{name}"),
                        ))
                    }
                }
                _ => HirGenericArg::Ty(self.hir_ty_of_rust_ty(arg.clone())),
            })
            .collect()
    }

    fn hir_ty_of_resolved_path(
        &mut self,
        path: &crate::DefOrLocal,
        parser_span: co2_ast::Span,
    ) -> HirTy {
        let span = self.co2_span_to_rustc(parser_span);
        match path {
            crate::DefOrLocal::Def {
                def_id,
                generic_args,
            } => {
                let hir_args = self.hir_generic_args_of_resolved_path(generic_args);
                if !generic_args.is_empty() {
                    let ty = CrateItem(*def_id).ty();
                    if let TyKind::RigidTy(RigidTy::Adt(_, GenericArgs(params))) = ty.kind() {
                        let expected_lifetimes = params
                            .iter()
                            .filter(|a| matches!(a, GenericArgKind::Lifetime(_)))
                            .count();
                        let provided_lifetimes = hir_args
                            .iter()
                            .filter(|a| matches!(a, HirGenericArg::Lifetime(_)))
                            .count();
                        if expected_lifetimes > provided_lifetimes {
                            self.terminate_with_spanned_error((
                                parser_span,
                                "missed lifetime parameter".to_string(),
                            ));
                        }
                    }
                }
                HirTy::adt(*def_id, hir_args, span)
            }
            crate::DefOrLocal::Const(_) => panic!("invalid const in type position"),
            crate::DefOrLocal::AssocMethod { .. } => {
                panic!("invalid associated method in type position")
            }
            crate::DefOrLocal::Local(_) | crate::DefOrLocal::LocalConst(_) => {
                panic!("invalid parsing")
            }
            crate::DefOrLocal::FuncName => panic!("invalid __func__ in type position"),
            crate::DefOrLocal::Prim(primitive_ty) => self.hir_ty_of_prim(*primitive_ty, span),
            crate::DefOrLocal::UnrepresentableType(ty) => match ty {
                CTy::Ty(ty) => ty.clone(),
                CTy::Function(_) => panic!("function is invalid as a type name"),
                CTy::UnsizedArray(_) => panic!("unsized array is invalid as a type name"),
            },
            crate::DefOrLocal::InlineRustTy(ty) => {
                self.hir_ty_of_rust_ty((*ty.clone(), parser_span))
            }
        }
    }

    pub(crate) fn eval_array_len_expr(
        &mut self,
        expr: &Spanned<Expression<LocalResolver>>,
    ) -> Result<usize, (co2_ast::Span, String)> {
        let value = self.eval_const_expr(expr)?;
        usize::try_from(value).map_err(|_| {
            spanned_error(
                expr.1,
                format!("array size must be a non-negative integer, got {value}"),
            )
        })
    }

    // TODO: this function is probably wrong and should be removed
    pub(crate) fn eval_const_expr(
        &mut self,
        (expr, span): &Spanned<Expression<LocalResolver>>,
    ) -> Result<i128, (co2_ast::Span, String)> {
        match expr {
            Expression::Constant(Constant::Int(v, _)) => Ok(*v),
            Expression::Constant(Constant::Char(ch)) => Ok(i128::from(*ch as u8 as i8)),
            Expression::Constant(Constant::Float(_, _)) => Err(spanned_error(
                *span,
                "cannot use floats in const expressions",
            )),
            Expression::Identifier((resolved, _)) => match resolved {
                crate::DefOrLocal::Const(def_id) => {
                    if self.has_local_const_value(*def_id) {
                        self.eval_local_const(*def_id, *span)
                    } else if let Some(val) = self.hir_ctx.dependency_const_value(*def_id) {
                        match val {
                            rustc_public_generative::DependencyConstValue::Bool(b) => {
                                Ok(i128::from(b))
                            }
                            rustc_public_generative::DependencyConstValue::Char(c) => Ok(c as i128),
                            rustc_public_generative::DependencyConstValue::I8(i) => {
                                Ok(i128::from(i))
                            }
                            rustc_public_generative::DependencyConstValue::I16(i) => {
                                Ok(i128::from(i))
                            }
                            rustc_public_generative::DependencyConstValue::I32(i) => {
                                Ok(i128::from(i))
                            }
                            rustc_public_generative::DependencyConstValue::I64(i)
                            | rustc_public_generative::DependencyConstValue::Isize(i) => {
                                Ok(i128::from(i))
                            }
                            rustc_public_generative::DependencyConstValue::I128(i) => Ok(i),
                            rustc_public_generative::DependencyConstValue::U8(u) => {
                                Ok(i128::from(u))
                            }
                            rustc_public_generative::DependencyConstValue::U16(u) => {
                                Ok(i128::from(u))
                            }
                            rustc_public_generative::DependencyConstValue::U32(u) => {
                                Ok(i128::from(u))
                            }
                            rustc_public_generative::DependencyConstValue::U64(u)
                            | rustc_public_generative::DependencyConstValue::Usize(u) => {
                                Ok(i128::from(u))
                            }
                            rustc_public_generative::DependencyConstValue::U128(u) => Ok(u as i128),
                            rustc_public_generative::DependencyConstValue::F32(_)
                            | rustc_public_generative::DependencyConstValue::F64(_) => {
                                Err(spanned_error(*span, "float constant in array size"))
                            }
                        }
                    } else {
                        Err(spanned_error(
                            *span,
                            format!("unsupported identifier in constant expression: {resolved:?}"),
                        ))
                    }
                }
                crate::DefOrLocal::Def { def_id, .. } => self.eval_local_const(*def_id, *span),
                crate::DefOrLocal::LocalConst(local) => self.eval_local_constexpr(*local, *span),
                _ => Err(spanned_error(
                    *span,
                    format!("unsupported identifier in constant expression: {resolved:?}"),
                )),
            },
            Expression::UnaryOp(op, inner) => {
                let inner = self.eval_const_expr(inner)?;
                match op {
                    UnaryOp::Plus => Ok(inner),
                    UnaryOp::Minus => inner
                        .checked_neg()
                        .ok_or_else(|| spanned_error(*span, "integer overflow in const eval")),
                    UnaryOp::Not => Ok(i128::from(inner == 0)),
                    UnaryOp::Com => Ok(!inner),
                    _ => Err(spanned_error(*span, "unsupported unary op in array size")),
                }
            }
            Expression::BinOp(lhs, op, rhs) => {
                let lhs = self.eval_const_expr(lhs)?;
                let rhs = self.eval_const_expr(rhs)?;
                match op {
                    BinOp::Add => lhs
                        .checked_add(rhs)
                        .ok_or_else(|| spanned_error(*span, "integer overflow in const eval")),
                    BinOp::Sub => lhs
                        .checked_sub(rhs)
                        .ok_or_else(|| spanned_error(*span, "integer overflow in const eval")),
                    BinOp::Mul => lhs
                        .checked_mul(rhs)
                        .ok_or_else(|| spanned_error(*span, "integer overflow in const eval")),
                    BinOp::Div => {
                        if rhs == 0 {
                            Err(spanned_error(*span, "division by zero happened in const eval"))
                        } else {
                            lhs.checked_div(rhs).ok_or_else(|| {
                                spanned_error(*span, "integer overflow in const eval")
                            })
                        }
                    }
                    BinOp::Rem => {
                        if rhs == 0 {
                            Err(spanned_error(*span, "division by zero happened in const eval"))
                        } else {
                            lhs.checked_rem(rhs).ok_or_else(|| {
                                spanned_error(*span, "integer overflow in const eval")
                            })
                        }
                    }
                    BinOp::BitOr => Ok(lhs | rhs),
                    BinOp::BitXor => Ok(lhs ^ rhs),
                    BinOp::BitAnd => Ok(lhs & rhs),
                    BinOp::Eq => Ok(i128::from(lhs == rhs)),
                    BinOp::Lt => Ok(i128::from(lhs < rhs)),
                    BinOp::Le => Ok(i128::from(lhs <= rhs)),
                    BinOp::Ne => Ok(i128::from(lhs != rhs)),
                    BinOp::Ge => Ok(i128::from(lhs >= rhs)),
                    BinOp::Gt => Ok(i128::from(lhs > rhs)),
                    BinOp::Shl => lhs
                        .checked_shl(rhs as u32)
                        .ok_or_else(|| spanned_error(*span, "shift out of bounds in const eval")),
                    BinOp::Shr => lhs
                        .checked_shr(rhs as u32)
                        .ok_or_else(|| spanned_error(*span, "shift out of bounds in const eval")),
                    BinOp::And => Ok(i128::from((lhs != 0) && (rhs != 0))),
                    BinOp::Or => Ok(i128::from((lhs != 0) || (rhs != 0))),
                    BinOp::Comma | BinOp::Assign => {
                        Err(spanned_error(*span, "unsupported binary op in array size"))
                    }
                }
            }
            Expression::Conditional {
                cond,
                then_expr,
                else_expr,
            } => {
                if self.eval_const_expr(cond)? != 0 {
                    self.eval_const_expr(then_expr)
                } else {
                    self.eval_const_expr(else_expr)
                }
            }
            Expression::Cast { type_name, expr } => {
                let value = self.eval_const_expr(expr)?;
                let target_ty = self.lower_type_name_for_const(*type_name.clone(), *span);
                self.cast_const_int(value, &target_ty, *span)
            }
            Expression::SizeofType(type_name) => {
                let ty = self.lower_type_name_for_const(*type_name.clone(), *span);
                Ok(self.sizeof_hir_ty(&ty, *span)?.0 as i128)
            }
            Expression::Offsetof {
                ty: type_name,
                field,
                field_span,
            } => {
                let ty = self.lower_type_name_for_const(*type_name.clone(), *span);
                Ok(self.offsetof_hir_ty(&ty, field, *span, *field_span)? as i128)
            }
            Expression::Sizeof(expr) => {
                let ty = self.type_of_expr_for_sizeof(expr);
                Ok(self.sizeof_hir_ty(&ty, *span)?.0 as i128)
            }
            Expression::AlignofType(type_name) => {
                let ty = self.lower_type_name_for_const(*type_name.clone(), *span);
                Ok(self.sizeof_hir_ty(&ty, *span)?.1 as i128)
            }
            Expression::Alignof(expr) => {
                let ty = self.type_of_expr_for_sizeof(expr);
                Ok(self.sizeof_hir_ty(&ty, *span)?.1 as i128)
            }
            Expression::BuiltinTypesCompatibleP { ty1, ty2 } => {
                let t1 = self.lower_type_name_for_const(*ty1.clone(), *span);
                let t2 = self.lower_type_name_for_const(*ty2.clone(), *span);
                Ok(i128::from(hir_tys_compatible(&t1, &t2)))
            }
            Expression::BuiltinConstantP { expr } => {
                Ok(i128::from(self.eval_const_expr(expr).is_ok()))
            }
            Expression::GenericSelection {
                controlling,
                associations,
            } => {
                let controlling_ty = self.type_of_expr_for_sizeof(controlling);
                let mut default_expr = None;
                for (assoc, _assoc_span) in associations {
                    match assoc {
                        GenericAssociation::Default { expr } => {
                            if default_expr.is_none() {
                                default_expr = Some(expr);
                            }
                        }
                        GenericAssociation::Type { type_name, expr } => {
                            let assoc_ty = self.lower_type_name_for_const(type_name.clone(), *span);
                            if hir_tys_compatible(&assoc_ty, &controlling_ty) {
                                return self.eval_const_expr(expr);
                            }
                        }
                    }
                }
                if let Some(expr) = default_expr {
                    self.eval_const_expr(expr)
                } else {
                    Err(spanned_error(
                        *span,
                        "no matching association in _Generic and no default provided",
                    ))
                }
            }
            Expression::Call { .. } => Err(spanned_error(*span, "cannot call non-const function")),
            _ => Err(spanned_error(*span, "unsupported constant expression")),
        }
    }

    pub(crate) fn eval_local_const(
        &mut self,
        def_id: DefId,
        span: Span,
    ) -> Result<i128, (co2_ast::Span, String)> {
        if let Some(expr) = self.constexpr_def_exprs.get(&def_id).cloned() {
            return self.eval_const_expr(&expr);
        }

        if let Some(val) = self.enum_const_values.get(&def_id) {
            return Ok(*val);
        }

        let mir_info = self
            .struct_manager
            .pending_enum_consts
            .iter()
            .find(|e| e.def_id == def_id)
            .map(|e| &e.mir_info)
            .ok_or_else(|| {
                spanned_error(span, format!("could not find enum constant {def_id:?}"))
            })?;

        let value = match &mir_info {
            crate::MirOwnerInfo::EnumConstZeroed => 0,
            crate::MirOwnerInfo::EnumConstExplicit { initializer, .. } => {
                self.eval_const_expr(&initializer.clone())?
            }
            crate::MirOwnerInfo::EnumConstPrevPlus(prev_id, _) => {
                let prev_id = *prev_id;
                self.eval_local_const(prev_id, span)? + 1
            }
            _ => {
                return Err(spanned_error(
                    span,
                    format!("def {def_id:?} is not an enum constant"),
                ));
            }
        };

        self.enum_const_values.insert(def_id, value);
        Ok(value)
    }

    pub(crate) fn eval_local_constexpr(
        &mut self,
        local: u32,
        span: Span,
    ) -> Result<i128, (co2_ast::Span, String)> {
        let expr = self
            .constexpr_local_exprs
            .get(&local)
            .cloned()
            .ok_or_else(|| {
                spanned_error(
                    span,
                    format!("missing constexpr initializer for local {local}"),
                )
            })?;
        self.eval_const_expr(&expr)
    }

    fn hir_ty_of_dependency_const_value(
        value: &rustc_public_generative::DependencyConstValue,
        span: rustc_public_generative::rustc_public::ty::Span,
    ) -> HirTy {
        match value {
            rustc_public_generative::DependencyConstValue::Bool(_) => HirTy {
                kind: HirTyKind::Bool,
                span,
            },
            rustc_public_generative::DependencyConstValue::Char(_)
            | rustc_public_generative::DependencyConstValue::I32(_) => {
                HirTy::signed_ty(IntTy::I32, span)
            }
            rustc_public_generative::DependencyConstValue::I8(_) => {
                HirTy::signed_ty(IntTy::I8, span)
            }
            rustc_public_generative::DependencyConstValue::I16(_) => {
                HirTy::signed_ty(IntTy::I16, span)
            }
            rustc_public_generative::DependencyConstValue::I64(_) => {
                HirTy::signed_ty(IntTy::I64, span)
            }
            rustc_public_generative::DependencyConstValue::I128(_) => {
                HirTy::signed_ty(IntTy::I128, span)
            }
            rustc_public_generative::DependencyConstValue::Isize(_) => {
                HirTy::signed_ty(IntTy::Isize, span)
            }
            rustc_public_generative::DependencyConstValue::U8(_) => {
                HirTy::unsigned_ty(UintTy::U8, span)
            }
            rustc_public_generative::DependencyConstValue::U16(_) => {
                HirTy::unsigned_ty(UintTy::U16, span)
            }
            rustc_public_generative::DependencyConstValue::U32(_) => {
                HirTy::unsigned_ty(UintTy::U32, span)
            }
            rustc_public_generative::DependencyConstValue::U64(_) => {
                HirTy::unsigned_ty(UintTy::U64, span)
            }
            rustc_public_generative::DependencyConstValue::U128(_) => {
                HirTy::unsigned_ty(UintTy::U128, span)
            }
            rustc_public_generative::DependencyConstValue::Usize(_) => {
                HirTy::unsigned_ty(UintTy::Usize, span)
            }
            rustc_public_generative::DependencyConstValue::F32(_) => {
                HirTy::float_ty(FloatTy::F32, span)
            }
            rustc_public_generative::DependencyConstValue::F64(_) => {
                HirTy::float_ty(FloatTy::F64, span)
            }
        }
    }

    fn maybe_local_enum_const_ty(
        &self,
        def_id: DefId,
        span: rustc_public_generative::rustc_public::ty::Span,
    ) -> Option<HirTy> {
        self.has_local_enum_const(def_id)
            .then(|| HirTy::signed_ty(IntTy::I32, span))
    }

    fn scalar_const_ty(
        &self,
        def_id: DefId,
        span: rustc_public_generative::rustc_public::ty::Span,
    ) -> Option<HirTy> {
        self.hir_ctx
            .dependency_const_value(def_id)
            .map(|value| Self::hir_ty_of_dependency_const_value(&value, span))
            .or_else(|| self.maybe_local_enum_const_ty(def_id, span))
    }

    fn maybe_const_eval_named_ty(
        &self,
        def_id: DefId,
        span: rustc_public_generative::rustc_public::ty::Span,
    ) -> Option<HirTy> {
        self.global_value_tys
            .get(&def_id)
            .cloned()
            .or_else(|| self.maybe_local_enum_const_ty(def_id, span))
    }

    fn cast_const_int(
        &self,
        value: i128,
        target_ty: &HirTy,
        span: co2_ast::Span,
    ) -> Result<i128, (co2_ast::Span, String)> {
        if let HirTyKind::Adt(def, _) = target_ty.kind
            && self.is_enum_def(def)
        {
            return self.cast_const_int(value, &HirTy::signed_ty(IntTy::I32, target_ty.span), span);
        }
        match target_ty.kind {
            HirTyKind::Bool => Ok(i128::from(value != 0)),
            HirTyKind::Char => {
                let codepoint = u32::try_from(value)
                    .map_err(|_| spanned_error(span, format!("invalid char cast value {value}")))?;
                char::from_u32(codepoint)
                    .map(|ch| ch as i128)
                    .ok_or_else(|| spanned_error(span, format!("invalid char cast value {value}")))
            }
            HirTyKind::Int(IntTy::I8) => Ok(i128::from(value as i8)),
            HirTyKind::Int(IntTy::I16) => Ok(i128::from(value as i16)),
            HirTyKind::Int(IntTy::I32) => Ok(i128::from(value as i32)),
            HirTyKind::Int(IntTy::I64) => Ok(i128::from(value as i64)),
            HirTyKind::Int(IntTy::I128) => Ok(value),
            HirTyKind::Int(IntTy::Isize) => Ok((value as isize) as i128),
            HirTyKind::Uint(UintTy::U8) => Ok(i128::from(value as u8)),
            HirTyKind::Uint(UintTy::U16) => Ok(i128::from(value as u16)),
            HirTyKind::Uint(UintTy::U32) => Ok(i128::from(value as u32)),
            HirTyKind::Uint(UintTy::U64) => Ok(i128::from(value as u64)),
            HirTyKind::Uint(UintTy::U128) => Ok((value as u128) as i128),
            HirTyKind::Uint(UintTy::Usize) => Ok((value as usize) as i128),
            HirTyKind::Adt(def, ref args) => {
                if args.is_empty()
                    && let Some(ty) = self.typedef_tys.get(&def)
                {
                    return self.cast_const_int(value, ty, span);
                }
                Err(spanned_error(
                    span,
                    format!("unsupported adt target in constant expression: {def:?}"),
                ))
            }
            _ => Err(spanned_error(
                span,
                format!(
                    "unsupported cast target in constant expression: {:?}",
                    target_ty.kind
                ),
            )),
        }
    }

    fn type_of_expr_for_sizeof(
        &mut self,
        (expr, span): &Spanned<Expression<LocalResolver>>,
    ) -> HirTy {
        let rust_span = self.co2_span_to_rustc(*span);
        match expr {
            Expression::Constant(Constant::String(s)) => {
                let elem_ty = match s.prefix {
                    StringLiteralPrefix::None
                    | StringLiteralPrefix::Str
                    | StringLiteralPrefix::Utf8 => HirTy::signed_ty(IntTy::I8, rust_span),
                    StringLiteralPrefix::Utf16 => HirTy::unsigned_ty(UintTy::U16, rust_span),
                    StringLiteralPrefix::Utf32 => HirTy::unsigned_ty(UintTy::U32, rust_span),
                    StringLiteralPrefix::Wide => HirTy::signed_ty(IntTy::I32, rust_span),
                };
                HirTy::new_array(
                    elem_ty,
                    HirTyConst::Literal(s.nul_terminated_len()),
                    rust_span,
                )
            }
            Expression::Constant(Constant::Int(_, suffix)) => {
                let kind = match suffix {
                    IntegerSuffix::None => HirTyKind::Int(IntTy::I32),
                    IntegerSuffix::Long => HirTyKind::Int(IntTy::I64),
                    IntegerSuffix::LongLong => HirTyKind::Int(IntTy::I128),
                    IntegerSuffix::Unsigned => HirTyKind::Uint(UintTy::U32),
                    IntegerSuffix::UnsignedLong => HirTyKind::Uint(UintTy::U64),
                    IntegerSuffix::UnsignedLongLong => HirTyKind::Uint(UintTy::U128),
                    IntegerSuffix::Usize => HirTyKind::Uint(UintTy::Usize),
                    IntegerSuffix::Isize => HirTyKind::Int(IntTy::Isize),
                    IntegerSuffix::U8 => HirTyKind::Uint(UintTy::U8),
                    IntegerSuffix::U16 => HirTyKind::Uint(UintTy::U16),
                    IntegerSuffix::U32 => HirTyKind::Uint(UintTy::U32),
                    IntegerSuffix::U64 => HirTyKind::Uint(UintTy::U64),
                    IntegerSuffix::U128 => HirTyKind::Uint(UintTy::U128),
                    IntegerSuffix::I8 => HirTyKind::Int(IntTy::I8),
                    IntegerSuffix::I16 => HirTyKind::Int(IntTy::I16),
                    IntegerSuffix::I32 => HirTyKind::Int(IntTy::I32),
                    IntegerSuffix::I64 => HirTyKind::Int(IntTy::I64),
                    IntegerSuffix::I128 => HirTyKind::Int(IntTy::I128),
                };
                HirTy {
                    kind,
                    span: rust_span,
                }
            }
            Expression::Constant(Constant::Char(_)) => HirTy::signed_ty(IntTy::I8, rust_span),
            Expression::Identifier((resolved, _)) => match resolved {
                crate::DefOrLocal::Local(_) | crate::DefOrLocal::LocalConst(_) => self
                    .terminate_with_error(
                        *span,
                        "local identifiers are invalid in crate signature",
                    ),
                crate::DefOrLocal::Def { def_id, .. } => self
                    .maybe_const_eval_named_ty(*def_id, rust_span)
                    .unwrap_or_else(|| {
                        self.terminate_with_error(
                            *span,
                            &format!("missing global or const type for def {def_id:?}"),
                        )
                    }),
                crate::DefOrLocal::Const(def_id) => {
                    self.scalar_const_ty(*def_id, rust_span).unwrap_or_else(|| {
                        self.terminate_with_error(
                            *span,
                            &format!("missing scalar constant value for def {def_id:?}"),
                        )
                    })
                }
                crate::DefOrLocal::AssocMethod { .. } => {
                    self.terminate_with_error(*span, "associated method path is invalid in sizeof")
                }
                crate::DefOrLocal::FuncName => {
                    self.terminate_with_error(*span, "__func__ is invalid in sizeof")
                }
                crate::DefOrLocal::Prim(primitive_ty) => {
                    self.hir_ty_of_prim(*primitive_ty, rust_span)
                }
                _ => self.terminate_with_error(
                    *span,
                    "unsupported identifier in sizeof(array size expr)",
                ),
            },
            Expression::Field(base, field) => {
                let base_ty = self.type_of_expr_for_sizeof(base);
                self.lookup_field_ty_for_sizeof(base_ty, &field.0, *span)
            }
            Expression::Arrow(base, field) => {
                let base_ty = self.type_of_expr_for_sizeof(base);
                let pointee = match base_ty.kind {
                    HirTyKind::RawPtr(_, inner) | HirTyKind::Ref(_, _, inner) => *inner,
                    _ => self.terminate_with_error(
                        *span,
                        "arrow base must be a pointer in sizeof(array size expr)",
                    ),
                };
                self.lookup_field_ty_for_sizeof(pointee, &field.0, *span)
            }
            Expression::Subscript(base, _) => {
                let base_ty = self.type_of_expr_for_sizeof(base);
                match base_ty.kind {
                    HirTyKind::Array(_, inner)
                    | HirTyKind::RawPtr(_, inner)
                    | HirTyKind::Ref(_, _, inner) => *inner,
                    _ => self.terminate_with_error(
                        *span,
                        "subscript base must be array or pointer in sizeof(array size expr)",
                    ),
                }
            }
            Expression::Cast { type_name, .. } => {
                self.lower_type_name_for_const(*type_name.clone(), *span)
            }
            Expression::UnaryOp(op, inner) => match op {
                UnaryOp::Deref => {
                    let inner_ty = self.type_of_expr_for_sizeof(inner);
                    match inner_ty.kind {
                        HirTyKind::RawPtr(_, pointee) | HirTyKind::Ref(_, _, pointee) => *pointee,
                        _ => self.terminate_with_error(
                            *span,
                            "cannot dereference non-pointer in sizeof(array size expr)",
                        ),
                    }
                }
                UnaryOp::AddrOf => {
                    let inner_ty = self.type_of_expr_for_sizeof(inner);
                    HirTy::new_ptr(inner_ty, Mutability::Mut, rust_span)
                }
                UnaryOp::Plus | UnaryOp::Minus | UnaryOp::Not | UnaryOp::Com => {
                    self.type_of_expr_for_sizeof(inner)
                }
            },
            Expression::BinOp(lhs, op, rhs) => match op {
                BinOp::Eq
                | BinOp::Ne
                | BinOp::Lt
                | BinOp::Le
                | BinOp::Gt
                | BinOp::Ge
                | BinOp::And
                | BinOp::Or => HirTy {
                    kind: HirTyKind::Int(IntTy::I32),
                    span: rust_span,
                },
                BinOp::Comma => self.type_of_expr_for_sizeof(rhs),
                _ => self.type_of_expr_for_sizeof(lhs),
            },
            _ => self.terminate_with_error(*span, "unsupported sizeof operand in array size"),
        }
    }

    fn lookup_field_ty_for_sizeof(
        &self,
        base_ty: HirTy,
        field_name: &str,
        span: co2_ast::Span,
    ) -> HirTy {
        let base_ty = self.peel_typedefs_for_sizeof(base_ty);
        let HirTyKind::Adt(def, _) = base_ty.kind else {
            self.terminate_with_error(
                span,
                "field access requires struct or union type in sizeof(array size expr)",
            )
        };
        self.adt_field_ty(def, field_name).unwrap_or_else(|| {
            self.terminate_with_error(
                span,
                &format!("unknown field `{field_name}` in sizeof(array size expr)"),
            )
        })
    }

    fn peel_typedefs_for_sizeof(&self, mut ty: HirTy) -> HirTy {
        let mut seen = HashSet::new();
        loop {
            let HirTyKind::Adt(def, _) = ty.kind else {
                return ty;
            };
            if !seen.insert(def) {
                return ty;
            }
            let Some(next_ty) = self.typedef_tys.get(&def).cloned() else {
                return ty;
            };
            ty = next_ty;
        }
    }

    fn resolve_offsetof_field_access(
        &self,
        ty: &HirTy,
        field: &str,
    ) -> Option<ResolvedConstFieldAccess> {
        let HirTyKind::Adt(def, _) = self.peel_typedefs_for_sizeof(ty.clone()).kind else {
            return None;
        };
        self.resolve_offsetof_field_access_from_metadata(def, field)
    }

    fn resolve_offsetof_field_access_from_metadata(
        &self,
        def: DefId,
        field: &str,
    ) -> Option<ResolvedConstFieldAccess> {
        let fields = self.adt_logical_fields(def)?;
        for logical_field in &fields {
            if logical_field.name != field || logical_field.name.starts_with("__anon_field_") {
                continue;
            }
            return Some(match logical_field.kind {
                LogicalAdtFieldKind::Direct { physical_index } => {
                    ResolvedConstFieldAccess::Direct {
                        path: vec![physical_index],
                    }
                }
                LogicalAdtFieldKind::Bitfield { .. } => ResolvedConstFieldAccess::Bitfield,
            });
        }
        for logical_field in fields {
            let LogicalAdtFieldKind::Direct { physical_index } = logical_field.kind else {
                continue;
            };
            if !logical_field.name.starts_with("__anon_field_") {
                continue;
            }
            let HirTyKind::Adt(nested_def, _) = logical_field.ty.kind else {
                continue;
            };
            if let Some(resolved) =
                self.resolve_offsetof_field_access_from_metadata(nested_def, field)
            {
                return Some(match resolved {
                    ResolvedConstFieldAccess::Direct { mut path } => {
                        path.insert(0, physical_index);
                        ResolvedConstFieldAccess::Direct { path }
                    }
                    ResolvedConstFieldAccess::Bitfield => ResolvedConstFieldAccess::Bitfield,
                });
            }
        }
        None
    }

    fn offsetof_hir_ty(
        &mut self,
        ty: &HirTy,
        field: &str,
        span: Span,
        field_span: Span,
    ) -> Result<usize, (co2_ast::Span, String)> {
        let ty = self.peel_typedefs_for_sizeof(ty.clone());
        let access = self
            .resolve_offsetof_field_access(&ty, field)
            .ok_or_else(|| {
                spanned_error(
                    field_span,
                    format!("offsetof: field '{field}' not found in type"),
                )
            })?;
        let ResolvedConstFieldAccess::Direct { path } = access else {
            return Err(spanned_error(
                field_span,
                format!("offsetof: field '{field}' is a bitfield"),
            ));
        };

        let mut offset = 0usize;
        let mut cur_ty = ty;
        for field_idx in path {
            let peeled_ty = self.peel_typedefs_for_sizeof(cur_ty.clone());
            let HirTyKind::Adt(def, _) = peeled_ty.kind else {
                return Err(spanned_error(
                    span,
                    "offsetof: field access requires a struct or union type",
                ));
            };
            let (kind, fields) = self.adt_layout_info(def).ok_or_else(|| {
                spanned_error(span, "offsetof: failed to compute layout for type")
            })?;
            let pack_align = self
                .struct_manager
                .definitions
                .get(&def)
                .and_then(|data| data.pack_align)
                .map(|n| n as usize);
            let field_ty = fields.get(field_idx).cloned().ok_or_else(|| {
                spanned_error(
                    span,
                    format!("offsetof: field index {field_idx} out of bounds"),
                )
            })?;
            let field_offset = match kind {
                co2_ast::StructOrUnionKind::Struct => {
                    let mut running_offset = 0usize;
                    for prev_field in fields.iter().take(field_idx) {
                        let (field_size, field_align) = self.sizeof_hir_ty(prev_field, span)?;
                        let field_align =
                            pack_align.map_or(field_align, |pack| field_align.min(pack));
                        running_offset = round_up(running_offset, field_align);
                        running_offset += field_size;
                    }
                    let (_, field_align) = self.sizeof_hir_ty(&field_ty, span)?;
                    let field_align = pack_align.map_or(field_align, |pack| field_align.min(pack));
                    round_up(running_offset, field_align)
                }
                co2_ast::StructOrUnionKind::Union => 0,
            };
            offset += field_offset;
            cur_ty = field_ty;
        }
        Ok(offset)
    }

    fn lower_type_name_for_const(
        &mut self,
        type_name: TypeName<LocalResolver>,
        span: Span,
    ) -> HirTy {
        if type_name.abstract_declarator.is_none()
            && let [
                (
                    co2_ast::SpecifierQualifier::TypeSpecifier((
                        TypeSpecifier::TypedefName((path, path_span)),
                        _,
                    )),
                    _,
                ),
            ] = type_name.specifier_qualifier_list.as_slice()
        {
            let rust_span = self.co2_span_to_rustc(*path_span);
            match path {
                crate::DefOrLocal::UnrepresentableType(ty) => match ty {
                    CTy::Ty(ty) => return ty.clone(),
                    CTy::Function(_) => {
                        self.terminate_with_error(span, "function is invalid as a type name")
                    }
                    CTy::UnsizedArray(_) => {
                        self.terminate_with_error(span, "unsized array is invalid as a type name")
                    }
                },
                crate::DefOrLocal::Def { def_id, .. } => {
                    if let Some(ty) = self.maybe_const_eval_named_ty(*def_id, rust_span) {
                        return ty;
                    }
                }
                crate::DefOrLocal::Const(def_id) => {
                    return self.scalar_const_ty(*def_id, rust_span).unwrap_or_else(|| {
                        self.terminate_with_error(
                            span,
                            &format!("missing scalar constant value for def {def_id:?}"),
                        )
                    });
                }
                crate::DefOrLocal::LocalConst(_) => {
                    self.terminate_with_error(
                        span,
                        "local identifiers are invalid in crate signature",
                    );
                }
                _ => {}
            }
        }

        let specifiers = type_name
            .specifier_qualifier_list
            .into_iter()
            .map(|(s, span)| {
                let s = match s {
                    co2_ast::SpecifierQualifier::TypeSpecifier(t) => {
                        DeclarationSpecifier::TypeSpecifier(t)
                    }
                    co2_ast::SpecifierQualifier::TypeQualifier(t) => {
                        DeclarationSpecifier::TypeQualifier(t)
                    }
                };
                (s, span)
            })
            .collect::<Vec<_>>();
        let base_const = has_const_qualifier_in_decl_specs(&specifiers);
        let base = self.base_ty_of_decl(specifiers, span);
        let ty = match type_name.abstract_declarator {
            None => base,
            Some(decl) => {
                self.extract_decl_type(base, base_const, decl)
                    .unwrap_or_else(|err| self.terminate_with_spanned_error(err))
                    .0
            }
        };
        match ty {
            CTy::Ty(ty) => ty,
            CTy::Function(_) => {
                self.terminate_with_error(span, "function is invalid as a type name")
            }
            CTy::UnsizedArray(_) => {
                self.terminate_with_error(span, "unsized array is invalid as a type name")
            }
        }
    }

    fn sizeof_hir_ty(
        &mut self,
        ty: &HirTy,
        span: Span,
    ) -> Result<(usize, usize), (co2_ast::Span, String)> {
        match &ty.kind {
            HirTyKind::Bool
            | HirTyKind::Char
            | HirTyKind::Int(IntTy::I8)
            | HirTyKind::Uint(UintTy::U8) => Ok((1, 1)),
            HirTyKind::Int(IntTy::I16)
            | HirTyKind::Uint(UintTy::U16)
            | HirTyKind::Float(FloatTy::F16) => Ok((2, 2)),
            HirTyKind::Int(IntTy::I32)
            | HirTyKind::Uint(UintTy::U32)
            | HirTyKind::Float(FloatTy::F32) => Ok((4, 4)),
            HirTyKind::Int(IntTy::I64 | IntTy::Isize)
            | HirTyKind::Uint(UintTy::U64 | UintTy::Usize)
            | HirTyKind::Float(FloatTy::F64)
            | HirTyKind::RawPtr(..)
            | HirTyKind::Ref(..)
            | HirTyKind::FnPtr(_) => Ok((8, 8)),
            HirTyKind::Int(IntTy::I128)
            | HirTyKind::Uint(UintTy::U128)
            | HirTyKind::Float(FloatTy::F128) => Ok((16, 16)),
            HirTyKind::Tuple(inner) if inner.is_empty() => Ok((0, 1)),
            HirTyKind::Array(HirTyConst::Literal(len), inner) => {
                let (elem_size, elem_align) = self.sizeof_hir_ty(inner, span)?;
                Ok((elem_size * len, elem_align))
            }
            HirTyKind::Array(HirTyConst::ConstDef(def_id), inner) => {
                let registered = self.lookup_array_len_const_by_def(*def_id).ok_or_else(|| {
                    spanned_error(span, "unsupported array length const in sizeof")
                })?;
                let len = self.eval_array_len_expr(&registered.expr)?;
                let (elem_size, elem_align) = self.sizeof_hir_ty(inner, span)?;
                Ok((elem_size * len, elem_align))
            }
            HirTyKind::Adt(def, args) => {
                if let Some((kind, fields)) = self.adt_layout_info(*def) {
                    let mut size = 0usize;
                    let mut align = 1usize;
                    match kind {
                        co2_ast::StructOrUnionKind::Struct => {
                            for field in fields {
                                let (field_size, field_align) = self.sizeof_hir_ty(&field, span)?;
                                align = align.max(field_align);
                                size = round_up(size, field_align);
                                size += field_size;
                            }
                        }
                        co2_ast::StructOrUnionKind::Union => {
                            for field in fields {
                                let (field_size, field_align) = self.sizeof_hir_ty(&field, span)?;
                                align = align.max(field_align);
                                size = size.max(field_size);
                            }
                        }
                    }
                    Ok((round_up(size, align), align))
                } else if let Some(ty) = self.typedef_tys.get(def).cloned() {
                    self.sizeof_hir_ty(&ty, span)
                } else if self
                    .resolver
                    .resolve("core::mem::MaybeUninit")
                    .ok()
                    .map(|(d, _)| d)
                    == Some(*def)
                {
                    // MaybeUninit<T> has the same size and alignment as T.
                    if let Some(HirGenericArg::Ty(inner)) = args.first() {
                        self.sizeof_hir_ty(inner, span)
                    } else {
                        Err(spanned_error(
                            span,
                            "unsupported ADT in sizeof(array size expr)",
                        ))
                    }
                } else {
                    Err(spanned_error(
                        span,
                        "unsupported ADT in sizeof(array size expr)",
                    ))
                }
            }
            _ => Err(spanned_error(
                span,
                "unsupported type in sizeof(array size expr)",
            )),
        }
    }

    pub(crate) fn lower_function_signature(
        &mut self,
        base: CTy,
        base_const: bool,
        declarator: Spanned<Declarator<LocalResolver>>,
    ) -> Result<(String, FunctionSignature), (co2_ast::Span, String)> {
        let span = declarator.1;
        let parsed_param_names = function_param_names(&declarator.0);
        let (decl_ty, name) = self.extract_decl_type(base, base_const, declarator)?;
        let name = name.ok_or_else(|| spanned_error(span, "missing function name"))?;
        let CTy::Function(mut sig) = decl_ty else {
            return Err(spanned_error(span, "it wasn't function"));
        };
        let names = parsed_param_names.unwrap_or_else(|| vec![None; sig.inputs.len()]);
        for (input, name) in sig.inputs.iter_mut().zip(names) {
            input.name = name;
        }
        Ok((name, sig))
    }

    pub(crate) fn lower_value_decl_type_maybe_unsized(
        &mut self,
        base: CTy,
        base_const: bool,
        declarator: Spanned<Declarator<LocalResolver>>,
    ) -> (String, HirTy, bool) {
        let span = declarator.1;
        let (decl_ty, name) = self
            .extract_decl_type(base, base_const, declarator)
            .unwrap_or_else(|err| self.terminate_with_spanned_error(err));
        let name =
            name.unwrap_or_else(|| self.terminate_with_error(span, "missing declaration name"));
        match decl_ty {
            CTy::Ty(ty) => (name, ty, false),
            CTy::Function(_) => self.terminate_with_error(
                span,
                "function is not a first-class declaration type in this context",
            ),
            CTy::UnsizedArray(ty) => {
                let rust_span = self.co2_span_to_rustc(span);
                (
                    name,
                    HirTy::new_array(ty, HirTyConst::Literal(0), rust_span),
                    true,
                )
            }
        }
    }

    fn hir_ty_of_prim(
        &mut self,
        primitive_ty: PrimitiveTy,
        span: rustc_public_generative::rustc_public::ty::Span,
    ) -> HirTy {
        match primitive_ty {
            PrimitiveTy::Bool => HirTy {
                kind: HirTyKind::Bool,
                span,
            },
            PrimitiveTy::Str => HirTy {
                kind: HirTyKind::Str,
                span,
            },
            PrimitiveTy::IntTy(int_ty) => HirTy::signed_ty(int_ty, span),
            PrimitiveTy::UintTy(uint_ty) => HirTy::unsigned_ty(uint_ty, span),
            PrimitiveTy::FloatTy(float_ty) => HirTy::float_ty(float_ty, span),
        }
    }

    pub(crate) fn base_ty_of_decl(
        &mut self,
        specifiers: Vec<Spanned<DeclarationSpecifier<LocalResolver>>>,
        parser_span: Span,
    ) -> CTy {
        let type_span = type_specifier_span(&specifiers).unwrap_or(parser_span);
        let span = self.co2_span_to_rustc(type_span);
        let specifier = match CompressedTypeSpecifier::build(specifiers) {
            Ok(s) => s,
            Err(err) => self.terminate_with_spanned_error(err),
        };
        let ty = match specifier {
            CompressedTypeSpecifier::Void => HirTy::new_tuple(vec![], span),
            CompressedTypeSpecifier::PrimitiveTy(ty) => self.hir_ty_of_prim(ty, span),
            CompressedTypeSpecifier::Enum(specifier) => HirTy::adt(specifier.0, vec![], span),
            CompressedTypeSpecifier::StructOrUnion { kind: _, specifier } => {
                HirTy::adt(specifier.0, vec![], span)
            }
            CompressedTypeSpecifier::TypedefName((path, path_span)) => match path {
                crate::DefOrLocal::UnrepresentableType(ty) => {
                    return ty;
                }
                _ => self.hir_ty_of_resolved_path(&path, path_span),
            },
            CompressedTypeSpecifier::TypeofType(type_name) => {
                return CTy::Ty(self.lower_type_name_for_const(type_name, parser_span));
            }
            CompressedTypeSpecifier::TypeofExpr(expr) => {
                return CTy::Ty(self.type_of_expr_for_sizeof(&expr));
            }
            CompressedTypeSpecifier::Auto => {
                CrateSigCtx::<'_>::terminate_with_error(
                    type_span,
                    "`auto` requires an initializer",
                );
            }
        };
        CTy::Ty(ty)
    }

    fn extract_decl_type(
        &mut self,
        current: CTy,
        current_const: bool,
        (decl, span): Spanned<Declarator<LocalResolver>>,
    ) -> Result<(CTy, Option<String>), (co2_ast::Span, String)> {
        let rust_span = self.co2_span_to_rustc(span);
        match decl {
            Declarator::Abstract => Ok((current, None)),
            Declarator::Identifier((ident, _)) => Ok((current, Some(ident.1))),
            Declarator::FunctionDeclarator {
                declarator,
                param_list,
            } => {
                let mut inputs = Vec::with_capacity(param_list.parameters.len());
                let c_variadic = param_list.effective_ellipsis();
                if !param_list.empty_params() {
                    for param in param_list.parameters {
                        let param_decl = param.1;
                        let param_base_const = has_const_qualifier_in_decl_specs(&param.0);
                        let param_base = self.base_ty_of_decl(param.0, param_decl.1);
                        let (param_decl_ty, _) =
                            self.extract_decl_type(param_base, param_base_const, param_decl)?;
                        let param_ty = match param_decl_ty {
                            CTy::Ty(ty) => {
                                let peeled_ty = self.peel_constexpr_typedef(ty.clone());
                                if let HirTyKind::Array(_, inner) = peeled_ty.kind {
                                    HirTy::new_ptr(*inner, Mutability::Mut, ty.span)
                                } else if let HirTyKind::Array(_, inner) = ty.kind {
                                    HirTy::new_ptr(*inner, Mutability::Mut, ty.span)
                                } else {
                                    ty
                                }
                            }
                            CTy::Function(sig) => {
                                // C adjusts function parameters to function pointers.
                                self.maybe_uninit_of(
                                    HirTy {
                                        kind: HirTyKind::FnPtr(Box::new(sig)),
                                        span: rust_span,
                                    },
                                    rust_span,
                                    span,
                                )
                            }
                            CTy::UnsizedArray(elem) => {
                                let span = elem.span;
                                HirTy::new_ptr(elem, Mutability::Mut, span)
                            }
                        };
                        inputs.push(FunctionInput {
                            name: None,
                            ty: param_ty,
                        });
                    }
                }
                let function_ty = match current {
                    CTy::Ty(ret) => CTy::Function(FunctionSignature {
                        lifetimes: vec![],
                        inputs,
                        output: ret,
                        abi: FunctionAbi::C,
                        is_unsafe: false,
                        c_variadic,
                    }),
                    CTy::Function(_) => {
                        return Err(spanned_error(
                            span,
                            "function returning function is not valid",
                        ));
                    }
                    CTy::UnsizedArray(_) => {
                        return Err(spanned_error(
                            span,
                            "function returning unsized array is not valid",
                        ));
                    }
                };
                self.extract_decl_type(function_ty, false, *declarator)
            }
            Declarator::PointerDeclarator {
                declarator,
                qualifiers,
            } => {
                let ptr_mutability = if current_const {
                    Mutability::Not
                } else {
                    Mutability::Mut
                };
                let ptr_or_fn_ptr = match current {
                    CTy::Ty(inner) => CTy::Ty(HirTy::new_ptr(inner, ptr_mutability, rust_span)),
                    CTy::Function(sig) => CTy::Ty(self.maybe_uninit_of(
                        HirTy {
                            kind: HirTyKind::FnPtr(Box::new(sig)),
                            span: rust_span,
                        },
                        rust_span,
                        span,
                    )),
                    CTy::UnsizedArray(inner) => CTy::Ty(HirTy::new_ptr(
                        HirTy::new_ptr(inner, Mutability::Mut, rust_span),
                        ptr_mutability,
                        rust_span,
                    )),
                };
                let next_const = qualifiers
                    .iter()
                    .any(|(q, _)| matches!(q, TypeQualifier::Const));
                self.extract_decl_type(ptr_or_fn_ptr, next_const, *declarator)
            }
            Declarator::ArrayDeclarator {
                declarator,
                subscription,
            } => {
                let inner = match current {
                    CTy::Ty(inner) => inner,
                    CTy::Function(_) => {
                        return Err(spanned_error(span, "array of functions is not valid"));
                    }
                    CTy::UnsizedArray(_) => {
                        return Err(spanned_error(span, "array of unsized arrays is not valid"));
                    }
                };
                let len = if let Some(len) = subscription.0.raw.constant_len() {
                    HirTyConst::Literal(len as usize)
                } else if subscription.0.raw.is_unsized() || subscription.0.raw.is_unspecified_vla()
                {
                    return self.extract_decl_type(
                        CTy::UnsizedArray(inner),
                        current_const,
                        *declarator,
                    );
                } else {
                    let registered = subscription.0.array_len_const.as_ref().ok_or_else(|| {
                        spanned_error(
                            subscription.1,
                            "array size must be a non-negative integer, got -1",
                        )
                    })?;
                    let expr = self
                        .lookup_array_len_const_by_id(*registered)
                        .ok_or_else(|| {
                            spanned_error(
                                subscription.1,
                                "array size must be a non-negative integer, got -1",
                            )
                        })?
                        .expr;
                    let literal_len = self
                        .eval_array_len_expr(&expr)
                        .unwrap_or_else(|err| self.terminate_with_spanned_error(err));
                    HirTyConst::Literal(literal_len)
                };
                let array_ty = CTy::Ty(HirTy::new_array(inner, len, rust_span));
                self.extract_decl_type(array_ty, current_const, *declarator)
            }
        }
    }

    fn maybe_uninit_of(
        &mut self,
        inner: HirTy,
        span: rustc_public_generative::rustc_public::ty::Span,
        co2_span: co2_ast::Span,
    ) -> HirTy {
        let (def, _) = self
            .resolver
            .resolve("core::mem::MaybeUninit")
            .unwrap_or_else(|e| {
                self.terminate_with_error(
                    co2_span,
                    &format!("failed to resolve core::mem::MaybeUninit: {e}"),
                )
            });
        HirTy::adt(def, vec![HirGenericArg::Ty(inner)], span)
    }

    pub(crate) fn terminate_with_spanned_error(&self, (span, msg): (co2_ast::Span, String)) -> ! {
        self.terminate_with_error(span, &msg)
    }

    pub(crate) fn terminate_with_error(&self, span: co2_ast::Span, msg: &str) -> ! {
        co2_ast::emit_errors_and_terminate(vec![co2_ast::Rich::custom(span, msg)]);
    }
}

fn function_param_names(decl: &Declarator<LocalResolver>) -> Option<Vec<Option<String>>> {
    match decl {
        Declarator::FunctionDeclarator {
            param_list,
            declarator,
        } if declarator.0.is_terminal() => Some(
            param_list
                .parameters
                .iter()
                .map(|param| Some(param.1.0.ident()?.1))
                .collect(),
        ),
        Declarator::FunctionDeclarator { declarator, .. }
        | Declarator::PointerDeclarator { declarator, .. }
        | Declarator::ArrayDeclarator { declarator, .. } => function_param_names(&declarator.0),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy)]
pub enum PrimitiveTy {
    Bool,
    Str,
    IntTy(IntTy),
    UintTy(UintTy),
    FloatTy(FloatTy),
}

impl PrimitiveTy {
    pub(crate) fn parse(name: &str) -> Option<Self> {
        match name {
            "u8" => Some(PrimitiveTy::UintTy(UintTy::U8)),
            "i8" => Some(PrimitiveTy::IntTy(IntTy::I8)),
            "u16" => Some(PrimitiveTy::UintTy(UintTy::U16)),
            "i16" => Some(PrimitiveTy::IntTy(IntTy::I16)),
            "u32" => Some(PrimitiveTy::UintTy(UintTy::U32)),
            "i32" => Some(PrimitiveTy::IntTy(IntTy::I32)),
            "u64" => Some(PrimitiveTy::UintTy(UintTy::U64)),
            "i64" => Some(PrimitiveTy::IntTy(IntTy::I64)),
            "u128" => Some(PrimitiveTy::UintTy(UintTy::U128)),
            "i128" => Some(PrimitiveTy::IntTy(IntTy::I128)),
            "usize" => Some(PrimitiveTy::UintTy(UintTy::Usize)),
            "isize" => Some(PrimitiveTy::IntTy(IntTy::Isize)),
            "str" => Some(PrimitiveTy::Str),
            "bool" => Some(PrimitiveTy::Bool),
            "f16" => Some(PrimitiveTy::FloatTy(FloatTy::F16)),
            "f32" | "_Float32" | "_Float32x" => Some(PrimitiveTy::FloatTy(FloatTy::F32)),
            "f64" | "_Float64" | "_Float64x" => Some(PrimitiveTy::FloatTy(FloatTy::F64)),
            "f128" | "_Float128" => Some(PrimitiveTy::FloatTy(FloatTy::F128)),
            _ => None,
        }
    }
}

/// Structural type compatibility check for `__builtin_types_compatible_p` and `_Generic`
/// in constant-expression contexts. Ignores the span field of `HirTy`.
fn hir_tys_compatible(a: &HirTy, b: &HirTy) -> bool {
    match (&a.kind, &b.kind) {
        (HirTyKind::Bool, HirTyKind::Bool)
        | (HirTyKind::Char, HirTyKind::Char)
        | (HirTyKind::Never, HirTyKind::Never) => true,
        (HirTyKind::Int(ia), HirTyKind::Int(ib)) => ia == ib,
        (HirTyKind::Uint(ua), HirTyKind::Uint(ub)) => ua == ub,
        (HirTyKind::Float(fa), HirTyKind::Float(fb)) => fa == fb,
        (HirTyKind::Adt(da, _), HirTyKind::Adt(db, _)) => da == db,
        (HirTyKind::RawPtr(ma, ia), HirTyKind::RawPtr(mb, ib)) => {
            ma == mb && hir_tys_compatible(ia, ib)
        }
        (HirTyKind::Tuple(va), HirTyKind::Tuple(vb)) => {
            va.len() == vb.len()
                && va
                    .iter()
                    .zip(vb.iter())
                    .all(|(a, b)| hir_tys_compatible(a, b))
        }
        _ => false,
    }
}
