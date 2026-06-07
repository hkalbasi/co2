use std::collections::HashMap;

use co2_ast::{
    Constant, Declaration, DeclarationSpecifier, Declarator, Expression, InitDeclarator,
    Initializer, RustTy, Span, Spanned, TypeName, TypeQualifier,
};
use co2_crate_sig::{CompressedTypeSpecifier, LocalResolver, LogicalAdtFieldKind};
use co2_parser::parse_expression_tokens;
use la_arena::Arena;
use rustc_public_generative::{
    HirTy,
    rustc_public::{
        CrateItem,
        abi::FieldsShape,
        mir::{Mutability, Safety},
        ty::{
            Abi, AdtDef, Binder, FnSig, ForeignDef, GenericArgKind, GenericArgs, IntTy, Region,
            RegionKind, RigidTy, Span as RustSpan, Ty, TyConst, TyKind, UintTy,
        },
    },
};

use crate::expr::HirExpr;
use crate::resolver::HirCtx;
use crate::stmt::HirStmt;
use crate::ty::{
    adt_field_tys, array_elem_ty, enum_payload_ty, is_array_ty, resolve_field_path_in_adt,
    ty_matches_expected,
};
use crate::{
    expr::coerce_expr_to_type,
    item::{HirLocal, LocalId},
};

pub enum CTy {
    Ty(Ty),
    Function(FnSig),
    UnsizedArray(Ty),
}

fn spanned_error(span: co2_ast::Span, msg: impl Into<String>) -> (co2_ast::Span, String) {
    (span, msg.into())
}

fn invalid_span() -> Span {
    Span::from_parts(co2_ast::FileId::INVALID, 0..0)
}

fn c_ty_matches_expected(expected: &CTy, actual: &CTy) -> bool {
    match (expected, actual) {
        (CTy::Ty(expected), CTy::Ty(actual)) => ty_matches_expected(*expected, *actual),
        (CTy::UnsizedArray(expected), CTy::UnsizedArray(actual)) => {
            ty_matches_expected(*expected, *actual)
        }
        (CTy::UnsizedArray(expected), CTy::Ty(actual))
        | (CTy::Ty(actual), CTy::UnsizedArray(expected)) => {
            array_elem_ty(*actual).is_some_and(|actual| ty_matches_expected(*expected, actual))
        }
        (CTy::Function(expected), CTy::Function(actual)) => {
            fn_sig_matches_expected(expected, actual)
        }
        _ => false,
    }
}

fn fn_sig_matches_expected(expected: &FnSig, actual: &FnSig) -> bool {
    expected.c_variadic == actual.c_variadic
        && expected.safety == actual.safety
        && expected.abi == actual.abi
        && expected.inputs_and_output.len() == actual.inputs_and_output.len()
        && expected
            .inputs_and_output
            .iter()
            .zip(actual.inputs_and_output.iter())
            .all(|(expected, actual)| ty_matches_expected(*expected, *actual))
}

fn declarator_has_restrict_qualifier(decl: &Declarator<LocalResolver>) -> bool {
    match decl {
        Declarator::Abstract | Declarator::Identifier(_) => false,
        Declarator::FunctionDeclarator { declarator, .. }
        | Declarator::ArrayDeclarator { declarator, .. } => {
            declarator_has_restrict_qualifier(&declarator.0)
        }
        Declarator::PointerDeclarator {
            declarator,
            qualifiers,
        } => {
            qualifiers
                .iter()
                .any(|(qualifier, _)| matches!(qualifier, TypeQualifier::Restrict))
                || declarator_has_restrict_qualifier(&declarator.0)
        }
    }
}

fn is_null_pointer_constexpr_expr((expr, _): &Spanned<Expression<LocalResolver>>) -> bool {
    match expr {
        Expression::Constant(Constant::Int(0, _)) => true,
        Expression::Cast { expr, .. } => is_null_pointer_constexpr_expr(expr),
        _ => false,
    }
}

fn dependency_const_value_to_i128(
    value: &rustc_public_generative::DependencyConstValue,
) -> Result<i128, (co2_ast::Span, String)> {
    match value {
        rustc_public_generative::DependencyConstValue::Bool(v) => Ok(i128::from(*v)),
        rustc_public_generative::DependencyConstValue::Char(ch) => Ok(*ch as i128),
        rustc_public_generative::DependencyConstValue::I8(v) => Ok(i128::from(*v)),
        rustc_public_generative::DependencyConstValue::I16(v) => Ok(i128::from(*v)),
        rustc_public_generative::DependencyConstValue::I32(v) => Ok(i128::from(*v)),
        rustc_public_generative::DependencyConstValue::I64(v)
        | rustc_public_generative::DependencyConstValue::Isize(v) => Ok(i128::from(*v)),
        rustc_public_generative::DependencyConstValue::I128(v) => Ok(*v),
        rustc_public_generative::DependencyConstValue::U8(v) => Ok(i128::from(*v)),
        rustc_public_generative::DependencyConstValue::U16(v) => Ok(i128::from(*v)),
        rustc_public_generative::DependencyConstValue::U32(v) => Ok(i128::from(*v)),
        rustc_public_generative::DependencyConstValue::U64(v)
        | rustc_public_generative::DependencyConstValue::Usize(v) => Ok(i128::from(*v)),
        rustc_public_generative::DependencyConstValue::U128(v) => Ok(*v as i128),
        rustc_public_generative::DependencyConstValue::F32(_)
        | rustc_public_generative::DependencyConstValue::F64(_) => Err(spanned_error(
            invalid_span(),
            "float constant in array size",
        )),
    }
}

fn cast_const_int_to_ty(value: i128, target_ty: Ty) -> Result<i128, (co2_ast::Span, String)> {
    match target_ty.kind() {
        TyKind::RigidTy(RigidTy::Bool) => Ok(i128::from(value != 0)),
        TyKind::RigidTy(RigidTy::Char) => {
            let codepoint = u32::try_from(value).map_err(|_| {
                spanned_error(invalid_span(), format!("invalid char cast value {value}"))
            })?;
            char::from_u32(codepoint)
                .map(|ch| ch as i128)
                .ok_or_else(|| {
                    spanned_error(invalid_span(), format!("invalid char cast value {value}"))
                })
        }
        TyKind::RigidTy(RigidTy::Int(IntTy::I8)) => Ok(i128::from(value as i8)),
        TyKind::RigidTy(RigidTy::Int(IntTy::I16)) => Ok(i128::from(value as i16)),
        TyKind::RigidTy(RigidTy::Int(IntTy::I32) | RigidTy::Adt(_, _)) => {
            Ok(i128::from(value as i32))
        }
        TyKind::RigidTy(RigidTy::Int(IntTy::I64)) => Ok(i128::from(value as i64)),
        TyKind::RigidTy(RigidTy::Int(IntTy::I128)) => Ok(value),
        TyKind::RigidTy(RigidTy::Int(IntTy::Isize)) => Ok((value as isize) as i128),
        TyKind::RigidTy(RigidTy::Uint(UintTy::U8)) => Ok(i128::from(value as u8)),
        TyKind::RigidTy(RigidTy::Uint(UintTy::U16)) => Ok(i128::from(value as u16)),
        TyKind::RigidTy(RigidTy::Uint(UintTy::U32)) => Ok(i128::from(value as u32)),
        TyKind::RigidTy(RigidTy::Uint(UintTy::U64)) => Ok(i128::from(value as u64)),
        TyKind::RigidTy(RigidTy::Uint(UintTy::U128)) => Ok((value as u128) as i128),
        TyKind::RigidTy(RigidTy::Uint(UintTy::Usize)) => Ok((value as usize) as i128),
        _ => Err(spanned_error(
            invalid_span(),
            format!(
                "unsupported cast target in constant expression: {:?}",
                target_ty.kind()
            ),
        )),
    }
}

fn is_scalar_ty(ctx: &HirCtx<'_>, ty: Ty, span: RustSpan) -> bool {
    if enum_payload_ty(ty).is_some() {
        return true;
    }
    match ty.kind() {
        TyKind::RigidTy(
            RigidTy::Bool
            | RigidTy::Char
            | RigidTy::Int(_)
            | RigidTy::Uint(_)
            | RigidTy::Float(_)
            | RigidTy::RawPtr(_, _),
        ) => true,
        TyKind::RigidTy(RigidTy::Adt(def, _)) => {
            // Check if this is a typedef to a scalar type
            // Create a HirTy for the Adt to check via peeling
            let adt_hir_ty = HirTy {
                kind: rustc_public_generative::HirTyKind::Adt(def.0, vec![]),
                span,
            };
            let peeled = ctx.decl_resolver.peel_constexpr_typedef_hir(adt_hir_ty);
            matches!(
                peeled.kind,
                rustc_public_generative::HirTyKind::Bool
                    | rustc_public_generative::HirTyKind::Char
                    | rustc_public_generative::HirTyKind::Int(_)
                    | rustc_public_generative::HirTyKind::Uint(_)
                    | rustc_public_generative::HirTyKind::Float(_)
                    | rustc_public_generative::HirTyKind::RawPtr(_, _)
            )
        }
        _ => false,
    }
}

fn validate_local_constexpr_decl(
    ctx: &HirCtx<'_>,
    specifiers: &[Spanned<DeclarationSpecifier<LocalResolver>>],
    declarator: &Declarator<LocalResolver>,
    ty: &CTy,
    initializer: Option<&Spanned<Initializer<LocalResolver>>>,
) -> Result<(), (co2_ast::Span, String)> {
    let span = specifiers
        .first()
        .map(|(_, span)| *span)
        .or_else(|| initializer.map(|(_, span)| *span))
        .expect("constexpr declaration should have a span");
    if !specifiers.iter().any(|spec| spec.0.is_constexpr()) {
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

    if specifiers.iter().any(|spec| {
        matches!(
            spec.0,
            DeclarationSpecifier::TypeQualifier((TypeQualifier::Volatile, _))
        )
    }) {
        return Err(spanned_error(
            span,
            "`constexpr` object type cannot be volatile-qualified",
        ));
    }
    if specifiers.iter().any(|spec| {
        matches!(
            spec.0,
            DeclarationSpecifier::TypeQualifier((TypeQualifier::Atomic, _))
        )
    }) {
        return Err(spanned_error(
            span,
            "`constexpr` object type cannot be atomic",
        ));
    }
    if declarator_has_restrict_qualifier(declarator) {
        return Err(spanned_error(
            span,
            "`constexpr` object type cannot be restrict-qualified",
        ));
    }

    let rust_span = specifiers
        .first()
        .map(|(_, sp)| ctx.to_rust_span(*sp))
        .expect("should have at least one specifier");

    match ty {
        CTy::Ty(ty) => {
            if !is_scalar_ty(ctx, *ty, rust_span) {
                return Err(spanned_error(
                    span,
                    "`constexpr` object type must be scalar",
                ));
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
            // The element type must be scalar
            if !is_scalar_ty(ctx, *elem_ty, rust_span) {
                return Err(spanned_error(
                    span,
                    "`constexpr` object type must be scalar",
                ));
            }
        }
    }

    if matches!(ty, CTy::Ty(ty) if matches!(ty.kind(), TyKind::RigidTy(RigidTy::RawPtr(_, _)))) {
        if !is_null_pointer_constexpr_expr(expr) {
            return Err(spanned_error(
                expr.1,
                "`constexpr` pointer initializer must be null",
            ));
        }
    } else if !matches!(ty, CTy::UnsizedArray(_))
        && ctx.decl_resolver.eval_const_expr(expr).is_err()
    {
        return Err(spanned_error(
            span,
            "`constexpr` initializer must be a constant expression",
        ));
    }

    Ok(())
}

enum ResolvedConstFieldAccess {
    Direct { path: Vec<usize> },
    Bitfield,
}

#[derive(Clone, Debug)]
pub struct HirDecl {
    pub local: LocalId,
    pub initializer: Option<HirExpr>,
    pub span: RustSpan,
}

impl HirCtx<'_> {
    pub(crate) fn maybe_uninit_of(&self, inner: Ty) -> Ty {
        Ty::from_rigid_kind(RigidTy::Adt(
            self.wellknown_defs.maybe_uninit,
            GenericArgs(vec![GenericArgKind::Type(inner)]),
        ))
    }

    /// Lower generic args that may contain `RustTy::Wild`.
    /// `Wild` is replaced with the function's own `TyKind::Param` at the given `param_idx`,
    /// so inference can fill it in later.
    pub(crate) fn lower_generic_arg_with_wild(
        &self,
        param_idx: usize,
        arg: &Spanned<RustTy<LocalResolver>>,
        fn_params: &[GenericArgKind],
    ) -> GenericArgKind {
        match &arg.0 {
            RustTy::Wild => fn_params
                .get(param_idx)
                .cloned()
                .unwrap_or(GenericArgKind::Type(Ty::from_rigid_kind(RigidTy::Never))),
            _ => GenericArgKind::Type(self.lower_rust_ty(arg.clone())),
        }
    }

    pub(crate) fn lower_generic_args(
        &self,
        generic_args: &[Spanned<RustTy<LocalResolver>>],
    ) -> Vec<GenericArgKind> {
        generic_args
            .iter()
            .map(|arg| GenericArgKind::Type(self.lower_rust_ty(arg.clone())))
            .collect()
    }

    pub(crate) fn lower_rust_ty(&self, (ty, span): Spanned<RustTy<LocalResolver>>) -> Ty {
        match ty {
            RustTy::Path((path, path_span)) => self.ty_of_resolved_path(&path, path_span),
            RustTy::Ptr { mutable, inner } => Ty::new_ptr(
                self.lower_rust_ty(*inner),
                if mutable {
                    Mutability::Mut
                } else {
                    Mutability::Not
                },
            ),
            RustTy::Ref { mutable, inner } => Ty::new_ref(
                rustc_public_generative::rustc_public::ty::Region {
                    kind: RegionKind::ReStatic,
                },
                self.lower_rust_ty(*inner),
                if mutable {
                    Mutability::Mut
                } else {
                    Mutability::Not
                },
            ),
            RustTy::Tuple(elems) => Ty::new_tuple(
                &elems
                    .into_iter()
                    .map(|elem| self.lower_rust_ty(elem))
                    .collect::<Vec<_>>(),
            ),
            RustTy::Slice(inner) => Ty::from_rigid_kind(RigidTy::Slice(self.lower_rust_ty(*inner))),
            RustTy::Array { inner, len } => {
                let Some(len) = len.0.constant_len() else {
                    self.terminate_with_error(
                        span,
                        "Rust array generic arguments currently require a literal length",
                    );
                };
                let Ok(len) = usize::try_from(len) else {
                    self.terminate_with_error(
                        span,
                        "Rust array generic argument length is too large for this target",
                    );
                };
                Ty::from_rigid_kind(RigidTy::Array(
                    self.lower_rust_ty(*inner),
                    TyConst::try_from_target_usize(
                        len.try_into().expect("array len should fit u64"),
                    )
                    .expect("array len should fit target usize"),
                ))
            }
            RustTy::BareFn { params, ret_ty } => {
                let mut inputs_and_output = params
                    .into_iter()
                    .map(|param| self.lower_rust_ty(param))
                    .collect::<Vec<_>>();
                inputs_and_output.push(self.lower_rust_ty(*ret_ty));
                Ty::from_rigid_kind(RigidTy::FnPtr(Binder::dummy(FnSig {
                    inputs_and_output,
                    c_variadic: false,
                    safety: Safety::Safe,
                    abi: Abi::Rust,
                })))
            }
            RustTy::Never => Ty::from_rigid_kind(RigidTy::Never),
            RustTy::Wild => {
                self.terminate_with_error(
                    span,
                    "Wild type argument `_` can only be used in function call generic arguments",
                );
            }
        }
    }

    pub(crate) fn ty_of_resolved_path(
        &self,
        path: &co2_crate_sig::DefOrLocal,
        span: co2_ast::Span,
    ) -> Ty {
        match path {
            co2_crate_sig::DefOrLocal::Def {
                def_id,
                generic_args,
            } if generic_args.is_empty() => CrateItem(*def_id).ty(),
            co2_crate_sig::DefOrLocal::Def {
                def_id,
                generic_args,
            } => Ty::from_rigid_kind(RigidTy::Adt(
                AdtDef(*def_id),
                GenericArgs(self.lower_generic_args(generic_args)),
            )),
            co2_crate_sig::DefOrLocal::Const(_) => panic!("Invalid const in type position"),
            co2_crate_sig::DefOrLocal::AssocMethod { .. } => {
                panic!("Invalid associated method in type position")
            }
            co2_crate_sig::DefOrLocal::Local(_) => panic!("Invalid local in type position"),
            co2_crate_sig::DefOrLocal::LocalConst(_) => {
                panic!("Invalid local constexpr in type position")
            }
            co2_crate_sig::DefOrLocal::FuncName => panic!("Invalid __func__ in type position"),
            co2_crate_sig::DefOrLocal::Prim(primitive_ty) => prim_ty_to_ty(*primitive_ty),
            co2_crate_sig::DefOrLocal::UnrepresentableType(sig_ty) => {
                match Self::sig_cty_to_cty(sig_ty) {
                    CTy::Ty(ty) => ty,
                    CTy::Function(_) => panic!("Function is invalid as a type name"),
                    CTy::UnsizedArray(_) => panic!("Unsized array is invalid as a type name"),
                }
            }
            co2_crate_sig::DefOrLocal::InlineRustTy(ty) => self.lower_rust_ty((*ty.clone(), span)),
        }
    }

    pub(crate) fn lower_decl(
        &self,
        decl: Declaration<LocalResolver>,
        out: &mut Vec<HirStmt>,
        locals: &mut Arena<HirLocal>,
        local_map: &mut HashMap<usize, LocalId>,
    ) -> Result<(), (co2_ast::Span, String)> {
        match decl {
            Declaration::FunctionDefinition { .. } => Err(spanned_error(
                invalid_span(),
                "nested function declaration is not supported",
            )),
            Declaration::RustStruct { .. } | Declaration::RustTypeAlias { .. } => {
                Err(spanned_error(
                    invalid_span(),
                    "nested rust style type declaration is not supported",
                ))
            }
            Declaration::Declaration {
                declaration_specifiers,
                declarators,
                ..
            } => {
                if declarators.is_empty() {
                    let parser_span = declaration_specifiers
                        .first()
                        .map(|specifier| specifier.1)
                        .expect("declaration should have at least one specifier");
                    let _ = self.base_ty_of_decl(declaration_specifiers, parser_span);
                    return Ok(());
                }
                for init in declarators {
                    let InitDeclarator {
                        declarator,
                        initializer,
                        is_transparent_union: _,
                    } = init.0;
                    let is_constexpr = declaration_specifiers
                        .iter()
                        .any(|spec| spec.0.is_constexpr());
                    let raw_initializer = initializer.clone();
                    let declarator_for_checks = declarator.0.clone();
                    let ((name, parser_span), ty) = self.lower_value_decl_type(
                        declaration_specifiers.clone(),
                        declarator,
                        locals,
                        local_map,
                    );
                    validate_local_constexpr_decl(
                        self,
                        &declaration_specifiers,
                        &declarator_for_checks,
                        &ty,
                        raw_initializer.as_ref(),
                    )
                    .unwrap_or_else(|err| self.terminate_with_spanned_error(err));
                    let ty = match ty {
                        CTy::Ty(ty) => ty,
                        CTy::Function(sig) => {
                            // In C, function types in declarations are adjusted to function pointers
                            // sig is FnSig from rustc_public
                            let fn_ptr_ty = Ty::from_rigid_kind(RigidTy::FnPtr(Binder::dummy(sig)));
                            // In C mode, wrap function pointers in MaybeUninit
                            self.maybe_uninit_of(fn_ptr_ty)
                        }
                        CTy::UnsizedArray(elem) => {
                            let Some(initializer) = initializer else {
                                self.terminate_with_error(
                                    parser_span,
                                    "Unsized array without initializer is invalid",
                                );
                            };
                            let real_len = crate::infer_array_len_from_initializer_in_scope(
                                initializer.clone(),
                                elem,
                                self,
                                locals,
                                local_map,
                            );
                            let real_ty = Ty::from_rigid_kind(RigidTy::Array(
                                elem,
                                TyConst::try_from_target_usize(real_len).unwrap(),
                            ));
                            let tree = self.lower_to_initializer_tree(
                                real_ty,
                                initializer,
                                locals,
                                local_map,
                            );
                            let tree_expr =
                                self.initializer_tree_to_expr(&tree, real_ty, parser_span);

                            let span = self.to_rust_span(parser_span);
                            let local = locals.alloc(HirLocal {
                                name: name.1.clone(),
                                ty: real_ty,
                                span,
                                read_only: is_constexpr,
                            });
                            local_map.insert(name.0, local);

                            out.push(HirStmt::Decl(HirDecl {
                                local,
                                initializer: Some(tree_expr),
                                span,
                            }));
                            continue;
                        }
                    };

                    let span = self.to_rust_span(parser_span);
                    let local = locals.alloc(HirLocal {
                        name: name.1.clone(),
                        ty,
                        span,
                        read_only: is_constexpr,
                    });
                    local_map.insert(name.0, local);

                    let needs_tree = raw_initializer
                        .as_ref()
                        .is_some_and(|(init, _)| match init {
                            Initializer::List(_) => true,
                            Initializer::Expr((expr, _)) => {
                                is_array_ty(ty)
                                    || matches!(expr, Expression::Constant(Constant::String(_)))
                            }
                        });

                    let initializer = if let Some(init) = initializer {
                        match init.0 {
                            Initializer::Expr(expr) if !needs_tree => {
                                let parser_span = expr.1;
                                let mut expr = self.lower_expr(expr, locals, local_map)?;
                                if is_array_ty(expr.ty) && !is_array_ty(ty) {
                                    expr = self.array_to_pointer_decay(&expr);
                                }
                                // TODO: This code is very wrong. We should not touch local types beside their declared type.
                                let local_ty = if ty_matches_expected(ty, expr.ty) {
                                    expr.ty
                                } else {
                                    ty
                                };
                                locals[local].ty = local_ty;
                                // END TODO.
                                let expr = if let resolver = &self.decl_resolver
                                    && resolver.normalize_ty_for_current_owner(expr.ty)
                                        == resolver.normalize_ty_for_current_owner(local_ty)
                                {
                                    expr
                                } else {
                                    let expr_ty = expr.ty;
                                    match coerce_expr_to_type(expr, local_ty) {
                                        Some(it) => it,
                                        None => self.terminate_with_error(
                                            parser_span,
                                            &format!(
                                                "initializer type mismatch: expected {}, got {}",
                                                self.format_ty(local_ty),
                                                self.format_ty(expr_ty)
                                            ),
                                        ),
                                    }
                                };
                                Some(expr)
                            }
                            _ if needs_tree => {
                                let tree = self.lower_to_initializer_tree(
                                    ty,
                                    init.clone(),
                                    locals,
                                    local_map,
                                );
                                let expr = self.initializer_tree_to_expr(&tree, ty, parser_span);
                                Some(expr)
                            }
                            _ => None,
                        }
                    } else {
                        None
                    };

                    out.push(HirStmt::Decl(HirDecl {
                        local,
                        initializer,
                        span,
                    }));
                }
                Ok(())
            }
            Declaration::PragmaPack { .. } | Declaration::BreakCo2 => Ok(()),
        }
    }

    pub(crate) fn try_lower_value_decl_type(
        &self,
        declaration_specifiers: Vec<Spanned<DeclarationSpecifier<LocalResolver>>>,
        declarator: Spanned<Declarator<LocalResolver>>,
        locals: &mut Arena<HirLocal>,
        local_map: &mut HashMap<usize, LocalId>,
    ) -> Result<(Spanned<(usize, String)>, CTy), (co2_ast::Span, String)> {
        let span = declarator.1;
        let base =
            self.base_ty_of_decl_in_scope(declaration_specifiers, declarator.1, locals, local_map);
        let base_const = has_const_qualifier_in_decl_specs(&base.1);
        let (decl_ty, name) = self.extract_decl_type_in_scope(
            base.0,
            base_const,
            declarator,
            Some((locals, local_map)),
        )?;
        let name = name.ok_or_else(|| spanned_error(span, "missing declaration name"))?;
        Ok((name, decl_ty))
    }

    pub(crate) fn lower_value_decl_type(
        &self,
        declaration_specifiers: Vec<Spanned<DeclarationSpecifier<LocalResolver>>>,
        declarator: Spanned<Declarator<LocalResolver>>,
        locals: &mut Arena<HirLocal>,
        local_map: &mut HashMap<usize, LocalId>,
    ) -> (Spanned<(usize, String)>, CTy) {
        match self.try_lower_value_decl_type(declaration_specifiers, declarator, locals, local_map)
        {
            Ok(x) => x,
            Err(err) => self.terminate_with_spanned_error(err),
        }
    }

    pub(crate) fn lower_type_name_in_scope(
        &self,
        type_name: TypeName<LocalResolver>,
        span: co2_ast::Span,
        locals: &mut Arena<HirLocal>,
        local_map: &mut HashMap<usize, LocalId>,
    ) -> Result<Ty, (co2_ast::Span, String)> {
        match self.lower_type_name_cty_with_scope(type_name, span, Some((locals, local_map)))? {
            CTy::Ty(ty) => Ok(ty),
            CTy::Function(_) => {
                self.terminate_with_error(span, "Function is invalid as a type name");
            }
            CTy::UnsizedArray(_) => {
                self.terminate_with_error(span, "Unsized array is invalid as a type name");
            }
        }
    }

    pub(crate) fn type_names_compatible_in_scope(
        &self,
        ty1: TypeName<LocalResolver>,
        ty2: TypeName<LocalResolver>,
        span: co2_ast::Span,
        locals: &mut Arena<HirLocal>,
        local_map: &mut HashMap<usize, LocalId>,
    ) -> Result<bool, (co2_ast::Span, String)> {
        let ty1 =
            self.lower_type_name_cty_with_scope(ty1, span, Some((&mut *locals, &mut *local_map)))?;
        let ty2 = self.lower_type_name_cty_with_scope(ty2, span, Some((locals, local_map)))?;
        Ok(c_ty_matches_expected(&ty1, &ty2))
    }

    fn lower_type_name_cty_with_scope(
        &self,
        type_name: TypeName<LocalResolver>,
        span: co2_ast::Span,
        mut typeof_scope: Option<(&mut Arena<HirLocal>, &mut HashMap<usize, LocalId>)>,
    ) -> Result<CTy, (co2_ast::Span, String)> {
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
        let base = self.base_ty_of_decl_with_scope(
            specifiers,
            span,
            typeof_scope
                .as_mut()
                .map(|(locals, local_map)| (&mut **locals, &mut **local_map)),
        );
        let ty = match type_name.abstract_declarator {
            None => base.0,
            Some(decl) => {
                let (ty, name) = self.extract_decl_type_in_scope(
                    base.0,
                    has_const_qualifier_in_decl_specs(&base.1),
                    decl,
                    typeof_scope,
                )?;
                if let Some((_, span)) = name {
                    self.terminate_with_error(span, "type name should not have name");
                }
                ty
            }
        };
        Ok(ty)
    }

    fn lower_typeof_expr(
        &self,
        expr: &Spanned<Expression<LocalResolver>>,
        locals: &mut Arena<HirLocal>,
        local_map: &mut HashMap<usize, LocalId>,
    ) -> Result<CTy, (co2_ast::Span, String)> {
        let ty = self.type_of_expr_for_sizeof(expr, locals, local_map)?;
        match ty.kind() {
            TyKind::RigidTy(RigidTy::FnDef(_, _)) => {
                let sig = ty
                    .kind()
                    .fn_sig()
                    .expect("FnDef should have fn signature")
                    .skip_binder();
                Ok(CTy::Function(sig))
            }
            _ => Ok(CTy::Ty(ty)),
        }
    }

    fn base_ty_of_decl(
        &self,
        specifiers: Vec<Spanned<DeclarationSpecifier<LocalResolver>>>,
        span: co2_ast::Span,
    ) -> (CTy, Vec<Spanned<DeclarationSpecifier<LocalResolver>>>) {
        self.base_ty_of_decl_with_scope(specifiers, span, None)
    }

    fn base_ty_of_decl_in_scope(
        &self,
        specifiers: Vec<Spanned<DeclarationSpecifier<LocalResolver>>>,
        span: co2_ast::Span,
        locals: &mut Arena<HirLocal>,
        local_map: &mut HashMap<usize, LocalId>,
    ) -> (CTy, Vec<Spanned<DeclarationSpecifier<LocalResolver>>>) {
        self.base_ty_of_decl_with_scope(specifiers, span, Some((locals, local_map)))
    }

    fn base_ty_of_decl_with_scope(
        &self,
        specifiers: Vec<Spanned<DeclarationSpecifier<LocalResolver>>>,
        span: co2_ast::Span,
        mut typeof_scope: Option<(&mut Arena<HirLocal>, &mut HashMap<usize, LocalId>)>,
    ) -> (CTy, Vec<Spanned<DeclarationSpecifier<LocalResolver>>>) {
        let specifier = match CompressedTypeSpecifier::build(specifiers.clone()) {
            Ok(s) => s,
            Err(err) => self.terminate_with_spanned_error(err),
        };

        let ty = match specifier {
            CompressedTypeSpecifier::Void => Ty::new_tuple(&[]),
            CompressedTypeSpecifier::PrimitiveTy(primitive_ty) => prim_ty_to_ty(primitive_ty),
            CompressedTypeSpecifier::StructOrUnion { kind: _, specifier } => {
                // For forward-declared (incomplete) structs, lowering.rs registers them in
                // `typedef_tys` as `HirTy::adt(foreign_def, ...)` where `foreign_def` is a
                // ForeignType. Using `RigidTy::Adt(typedef_def, ...)` would ICE because rustc
                // calls `adt_def` on the TyAlias DefId.  Use `RigidTy::Foreign` instead.
                let foreign =
                    self.decl_resolver
                        .get_typedef_hir_ty(specifier.0)
                        .and_then(|hir_ty| match hir_ty.kind {
                            rustc_public_generative::HirTyKind::Adt(foreign_def, _) => {
                                Some(foreign_def)
                            }
                            _ => None,
                        });
                if let Some(foreign_def) = foreign {
                    Ty::from_rigid_kind(RigidTy::Foreign(ForeignDef(foreign_def)))
                } else {
                    Ty::from_rigid_kind(RigidTy::Adt(AdtDef(specifier.0), GenericArgs(vec![])))
                }
            }
            CompressedTypeSpecifier::Enum(specifier) => {
                Ty::from_rigid_kind(RigidTy::Adt(AdtDef(specifier.0), GenericArgs(vec![])))
            }
            CompressedTypeSpecifier::TypedefName(path) => {
                return match &path.0 {
                    co2_crate_sig::DefOrLocal::UnrepresentableType(sig_ty) => {
                        (Self::sig_cty_to_cty(sig_ty), specifiers)
                    }
                    _ => (CTy::Ty(self.ty_of_resolved_path(&path.0, span)), specifiers),
                };
            }
            CompressedTypeSpecifier::TypeofType(type_name) => {
                return (
                    self.lower_type_name_cty_with_scope(
                        type_name,
                        span,
                        typeof_scope
                            .as_mut()
                            .map(|(locals, local_map)| (&mut **locals, &mut **local_map)),
                    )
                    .unwrap_or_else(|err| self.terminate_with_spanned_error(err)),
                    specifiers,
                );
            }
            CompressedTypeSpecifier::TypeofExpr(expr) => {
                let Some((locals, local_map)) = typeof_scope.as_mut() else {
                    self.terminate_with_error(span, "typeof expression is not supported here");
                };
                return (
                    self.lower_typeof_expr(&expr, locals, local_map)
                        .unwrap_or_else(|err| self.terminate_with_spanned_error(err)),
                    specifiers,
                );
            }
        };

        (CTy::Ty(ty), specifiers)
    }

    pub(crate) fn eval_array_len_expr_in_scope(
        &self,
        expr: &Spanned<Expression<LocalResolver>>,
        locals: &mut Arena<HirLocal>,
        local_map: &mut HashMap<usize, LocalId>,
    ) -> Result<usize, (co2_ast::Span, String)> {
        let value = self.eval_const_expr_in_scope(expr, locals, local_map)?;
        usize::try_from(value).map_err(|_| {
            spanned_error(
                expr.1,
                format!("array size must be a non-negative integer, got {value}"),
            )
        })
    }

    pub(crate) fn eval_const_expr_in_scope(
        &self,
        (expr, span): &Spanned<Expression<LocalResolver>>,
        locals: &mut Arena<HirLocal>,
        local_map: &mut HashMap<usize, LocalId>,
    ) -> Result<i128, (co2_ast::Span, String)> {
        match expr {
            Expression::Constant(Constant::Int(v, _)) => Ok(*v),
            Expression::Constant(Constant::Char(ch)) => Ok(i128::from(*ch as u8 as i8)),
            Expression::Identifier((resolved, _)) => match resolved {
                co2_crate_sig::DefOrLocal::Const(def_id) => {
                    if self.decl_resolver.has_local_const_value(*def_id) {
                        self.decl_resolver.local_const_int_value(*def_id, *span)
                    } else if let Some(value) = self.decl_resolver.dependency_const_value(*def_id) {
                        dependency_const_value_to_i128(&value)
                    } else {
                        Err(spanned_error(
                            *span,
                            format!("unsupported identifier in constant expression: {resolved:?}"),
                        ))
                    }
                }
                co2_crate_sig::DefOrLocal::Def { def_id, .. } => {
                    self.decl_resolver.local_const_int_value(*def_id, *span)
                }
                co2_crate_sig::DefOrLocal::LocalConst(local) => {
                    self.decl_resolver.local_constexpr_int_value(*local, *span)
                }
                _ => Err(spanned_error(
                    *span,
                    format!("unsupported identifier in constant expression: {resolved:?}"),
                )),
            },
            Expression::UnaryOp(op, inner) => {
                let inner = self.eval_const_expr_in_scope(inner, locals, local_map)?;
                match op {
                    co2_ast::UnaryOp::Plus => Ok(inner),
                    co2_ast::UnaryOp::Minus => Ok(-inner),
                    co2_ast::UnaryOp::Not => Ok(i128::from(inner == 0)),
                    co2_ast::UnaryOp::Com => Ok(!inner),
                    _ => Err(spanned_error(*span, "unsupported unary op in array size")),
                }
            }
            Expression::BinOp(lhs, op, rhs) => {
                let lhs = self.eval_const_expr_in_scope(lhs, locals, local_map)?;
                let rhs = self.eval_const_expr_in_scope(rhs, locals, local_map)?;
                match op {
                    co2_ast::BinOp::Add => Ok(lhs + rhs),
                    co2_ast::BinOp::Sub => Ok(lhs - rhs),
                    co2_ast::BinOp::Mul => Ok(lhs * rhs),
                    co2_ast::BinOp::Div => Ok(lhs / rhs),
                    co2_ast::BinOp::Rem => Ok(lhs % rhs),
                    co2_ast::BinOp::BitOr => Ok(lhs | rhs),
                    co2_ast::BinOp::BitXor => Ok(lhs ^ rhs),
                    co2_ast::BinOp::BitAnd => Ok(lhs & rhs),
                    co2_ast::BinOp::Eq => Ok(i128::from(lhs == rhs)),
                    co2_ast::BinOp::Lt => Ok(i128::from(lhs < rhs)),
                    co2_ast::BinOp::Le => Ok(i128::from(lhs <= rhs)),
                    co2_ast::BinOp::Ne => Ok(i128::from(lhs != rhs)),
                    co2_ast::BinOp::Ge => Ok(i128::from(lhs >= rhs)),
                    co2_ast::BinOp::Gt => Ok(i128::from(lhs > rhs)),
                    co2_ast::BinOp::Shl => Ok(lhs << rhs),
                    co2_ast::BinOp::Shr => Ok(lhs >> rhs),
                    co2_ast::BinOp::And => Ok(i128::from((lhs != 0) && (rhs != 0))),
                    co2_ast::BinOp::Or => Ok(i128::from((lhs != 0) || (rhs != 0))),
                    co2_ast::BinOp::Comma | co2_ast::BinOp::Assign => {
                        Err(spanned_error(*span, "unsupported binary op in array size"))
                    }
                }
            }
            Expression::Conditional {
                cond,
                then_expr,
                else_expr,
            } => {
                if self.eval_const_expr_in_scope(cond, locals, local_map)? != 0 {
                    self.eval_const_expr_in_scope(then_expr, locals, local_map)
                } else {
                    self.eval_const_expr_in_scope(else_expr, locals, local_map)
                }
            }
            Expression::Cast { type_name, expr } => {
                let value = self.eval_const_expr_in_scope(expr, locals, local_map)?;
                let target_ty =
                    self.lower_type_name_in_scope(*type_name.clone(), *span, locals, local_map)?;
                cast_const_int_to_ty(value, target_ty)
            }
            Expression::SizeofType(type_name) => {
                let ty =
                    self.lower_type_name_in_scope(*type_name.clone(), *span, locals, local_map)?;
                Ok(i128::from(self.sizeof_ty(ty)?))
            }
            Expression::Offsetof {
                ty: type_name,
                field,
                field_span,
            } => {
                let ty =
                    self.lower_type_name_in_scope(*type_name.clone(), *span, locals, local_map)?;
                Ok(i128::from(self.offsetof_ty(
                    ty,
                    field,
                    *span,
                    *field_span,
                )?))
            }
            Expression::Sizeof(expr) => Ok(i128::from(self.sizeof_expr(expr, locals, local_map)?)),
            Expression::AlignofType(type_name) => {
                let ty =
                    self.lower_type_name_in_scope(*type_name.clone(), *span, locals, local_map)?;
                Ok(i128::from(self.alignof_ty(ty)?))
            }
            Expression::Alignof(expr) => {
                let ty = self.type_of_expr_for_sizeof(expr, locals, local_map)?;
                Ok(i128::from(self.alignof_ty(ty)?))
            }
            Expression::BuiltinTypesCompatibleP { ty1, ty2 } => {
                Ok(i128::from(self.type_names_compatible_in_scope(
                    *ty1.clone(),
                    *ty2.clone(),
                    *span,
                    locals,
                    local_map,
                )?))
            }
            Expression::BuiltinConstantP { expr } => Ok(i128::from(
                self.eval_const_expr_in_scope(expr, locals, local_map)
                    .is_ok(),
            )),
            Expression::GenericSelection {
                controlling,
                associations,
            } => {
                let controlling_ty =
                    self.type_of_expr_for_sizeof(controlling, locals, local_map)?;
                let mut default_expr = None;
                for (assoc, _) in associations {
                    match assoc {
                        co2_ast::GenericAssociation::Default { expr } => {
                            if default_expr.is_none() {
                                default_expr = Some(expr);
                            }
                        }
                        co2_ast::GenericAssociation::Type { type_name, expr } => {
                            let assoc_ty = self.lower_type_name_in_scope(
                                type_name.clone(),
                                *span,
                                locals,
                                local_map,
                            )?;
                            if ty_matches_expected(assoc_ty, controlling_ty) {
                                return self.eval_const_expr_in_scope(expr, locals, local_map);
                            }
                        }
                    }
                }
                if let Some(expr) = default_expr {
                    self.eval_const_expr_in_scope(expr, locals, local_map)
                } else {
                    Err(spanned_error(
                        *span,
                        "no matching association in _Generic and no default provided",
                    ))
                }
            }
            _ => Err(spanned_error(
                *span,
                "unsupported constant expression in array size",
            )),
        }
    }

    fn sizeof_expr(
        &self,
        expr: &Spanned<Expression<LocalResolver>>,
        locals: &mut Arena<HirLocal>,
        local_map: &mut HashMap<usize, LocalId>,
    ) -> Result<u64, (co2_ast::Span, String)> {
        if let Expression::Constant(Constant::String(s)) = &expr.0 {
            return Ok(s.storage_size() as u64);
        }
        let ty = self.type_of_expr_for_sizeof(expr, locals, local_map)?;
        self.sizeof_ty(ty)
    }

    fn type_of_expr_for_sizeof(
        &self,
        expr: &Spanned<Expression<LocalResolver>>,
        locals: &mut Arena<HirLocal>,
        local_map: &mut HashMap<usize, LocalId>,
    ) -> Result<Ty, (co2_ast::Span, String)> {
        let result = self.lower_expr(expr.clone(), locals, local_map)?;
        Ok(result.ty)
    }

    fn sizeof_ty(&self, ty: Ty) -> Result<u64, (co2_ast::Span, String)> {
        ty.layout()
            .map_err(|e| {
                spanned_error(
                    invalid_span(),
                    format!("failed to compute layout for sizeof: {e}"),
                )
            })
            .map(|layout| layout.shape().size.bytes() as u64)
    }

    fn alignof_ty(&self, ty: Ty) -> Result<u64, (co2_ast::Span, String)> {
        ty.layout()
            .map_err(|e| {
                spanned_error(
                    invalid_span(),
                    format!("failed to compute layout for alignof: {e}"),
                )
            })
            .map(|layout| layout.shape().abi_align)
    }

    fn resolve_offsetof_field_access(
        &self,
        ty: Ty,
        field: &str,
    ) -> Option<ResolvedConstFieldAccess> {
        let TyKind::RigidTy(RigidTy::Adt(def, _)) = ty.kind() else {
            return resolve_field_path_in_adt(ty, field)
                .map(|(path, _)| ResolvedConstFieldAccess::Direct { path });
        };
        self.resolve_offsetof_field_access_from_metadata(def.0, field)
            .or_else(|| {
                resolve_field_path_in_adt(ty, field)
                    .map(|(path, _)| ResolvedConstFieldAccess::Direct { path })
            })
    }

    fn resolve_offsetof_field_access_from_metadata(
        &self,
        def: rustc_public_generative::rustc_public::DefId,
        field: &str,
    ) -> Option<ResolvedConstFieldAccess> {
        let fields = self.decl_resolver.adt_logical_fields(def)?;
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
            let rustc_public_generative::HirTyKind::Adt(nested_def, _) = logical_field.ty.kind
            else {
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

    fn offsetof_ty(
        &self,
        ty: Ty,
        field: &str,
        span: co2_ast::Span,
        field_span: co2_ast::Span,
    ) -> Result<u64, (co2_ast::Span, String)> {
        let access = self
            .resolve_offsetof_field_access(ty, field)
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

        let mut offset_bytes = 0u64;
        let mut cur_ty = ty;
        for field_idx in path {
            let layout = cur_ty.layout().map_err(|e| {
                spanned_error(span, format!("offsetof: failed to compute layout: {e}"))
            })?;
            let field_offset = match &layout.shape().fields {
                FieldsShape::Arbitrary { offsets, .. } => offsets
                    .get(field_idx)
                    .ok_or_else(|| {
                        spanned_error(
                            span,
                            format!("offsetof: field index {field_idx} out of bounds"),
                        )
                    })?
                    .bytes() as u64,
                other => {
                    return Err(spanned_error(
                        span,
                        format!("offsetof: unsupported layout kind for type: {other:?}"),
                    ));
                }
            };
            offset_bytes += field_offset;
            cur_ty = adt_field_tys(cur_ty)
                .and_then(|tys| tys.into_iter().nth(field_idx))
                .ok_or_else(|| {
                    spanned_error(
                        span,
                        format!("offsetof: failed to get field type at index {field_idx}"),
                    )
                })?;
        }
        Ok(offset_bytes)
    }

    fn extract_decl_type(
        &self,
        current: CTy,
        current_const: bool,
        (decl, span): Spanned<Declarator<LocalResolver>>,
    ) -> Result<(CTy, Option<Spanned<(usize, String)>>), (co2_ast::Span, String)> {
        self.extract_decl_type_in_scope(current, current_const, (decl, span), None)
    }

    fn extract_decl_type_in_scope(
        &self,
        current: CTy,
        current_const: bool,
        (decl, span): Spanned<Declarator<LocalResolver>>,
        mut scope: Option<(&mut Arena<HirLocal>, &mut HashMap<usize, LocalId>)>,
    ) -> Result<(CTy, Option<Spanned<(usize, String)>>), (co2_ast::Span, String)> {
        match decl {
            Declarator::Abstract => Ok((current, None)),
            Declarator::Identifier(ident) => Ok((current, Some(ident))),
            Declarator::FunctionDeclarator {
                declarator,
                param_list,
            } => {
                let mut inputs = Vec::with_capacity(param_list.parameters.len());
                let c_variadic = param_list.effective_ellipsis();
                if !param_list.empty_params() {
                    for param in param_list.parameters {
                        let param_base = self.base_ty_of_decl(param.0, span);
                        let param_base_const = has_const_qualifier_in_decl_specs(&param_base.1);
                        let (param_decl_ty, _) =
                            self.extract_decl_type(param_base.0, param_base_const, param.1)?;
                        let mut param_ty = match param_decl_ty {
                            CTy::Ty(ty) => ty,
                            CTy::Function(sig) => {
                                // C adjusts function parameters to function pointers.
                                Ty::from_rigid_kind(RigidTy::FnPtr(Binder::dummy(sig)))
                            }
                            CTy::UnsizedArray(elem) => Ty::new_ptr(elem, Mutability::Mut),
                        };
                        // Function arguments are always decayed to pointer in C.
                        if let Some(elem) =
                            self.peel_typedef_array_elem(param_ty, self.to_rust_span(span))
                        {
                            param_ty = Ty::new_ptr(elem, Mutability::Mut);
                        }
                        inputs.push(param_ty);
                    }
                }
                let function_ty = match current {
                    CTy::Ty(ret) => {
                        let mut inputs_and_output = inputs;
                        inputs_and_output.push(ret);
                        CTy::Function(FnSig {
                            inputs_and_output,
                            c_variadic,
                            safety: Safety::Safe,
                            abi: Abi::C { unwind: false },
                        })
                    }
                    CTy::Function(_) => {
                        return Err(spanned_error(
                            span,
                            "returning function without ptr is not valid",
                        ));
                    }
                    CTy::UnsizedArray(_) => {
                        return Err(spanned_error(span, "returning unsized array is not valid"));
                    }
                };
                self.extract_decl_type_in_scope(function_ty, false, *declarator, scope)
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
                    CTy::Ty(inner) => CTy::Ty(Ty::new_ptr(inner, ptr_mutability)),
                    CTy::Function(sig) => {
                        let fn_ptr = Ty::from_rigid_kind(RigidTy::FnPtr(Binder::dummy(sig)));
                        CTy::Ty(self.maybe_uninit_of(fn_ptr))
                    }
                    CTy::UnsizedArray(elem) => CTy::Ty(Ty::new_ptr(
                        Ty::new_ptr(elem, Mutability::Mut),
                        ptr_mutability,
                    )),
                };
                let next_const = qualifiers
                    .iter()
                    .any(|(q, _)| matches!(q, TypeQualifier::Const));
                self.extract_decl_type_in_scope(ptr_or_fn_ptr, next_const, *declarator, scope)
            }
            Declarator::ArrayDeclarator {
                declarator,
                subscription,
            } => {
                let array_ty = match current {
                    CTy::Ty(inner) => {
                        if let Some(size) = subscription.0.raw.constant_len() {
                            CTy::Ty(Ty::try_new_array(inner, size as u64).map_err(|e| {
                                spanned_error(
                                    subscription.1,
                                    format!("failed to build array type: {e}"),
                                )
                            })?)
                        } else if subscription.0.raw.is_unsized()
                            || subscription.0.raw.is_unspecified_vla()
                        {
                            CTy::UnsizedArray(inner)
                        } else {
                            let expr =
                                if let Some(const_id) = subscription.0.array_len_const.as_ref() {
                                    self.decl_resolver
                                        .lookup_array_len_const_expr(*const_id)
                                        .ok_or_else(|| {
                                            spanned_error(
                                                subscription.1,
                                                "Can not calculate subscription",
                                            )
                                        })?
                                } else {
                                    let tokens = subscription
                                        .0
                                        .raw
                                        .tokens
                                        .get(1..subscription.0.raw.tokens.len().saturating_sub(1))
                                        .ok_or_else(|| {
                                            spanned_error(
                                                subscription.1,
                                                "Can not calculate subscription",
                                            )
                                        })?
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
                                    parse_expression_tokens(
                                        &tokens,
                                        subscription.1,
                                        self.decl_resolver.clone(),
                                    )
                                };
                            let len = if let Some((locals, local_map)) = scope.as_mut() {
                                self.eval_array_len_expr_in_scope(&expr, locals, local_map)
                            } else {
                                let mut locals = Arena::new();
                                let mut local_map = HashMap::new();
                                self.eval_array_len_expr_in_scope(
                                    &expr,
                                    &mut locals,
                                    &mut local_map,
                                )
                            }
                            .unwrap_or_else(|err| self.terminate_with_spanned_error(err));
                            CTy::Ty(Ty::try_new_array(inner, len as u64).map_err(|e| {
                                spanned_error(
                                    subscription.1,
                                    format!("failed to build array type: {e}"),
                                )
                            })?)
                        }
                    }
                    CTy::Function(_) => {
                        return Err(spanned_error(span, "array of functions is invalid"));
                    }
                    CTy::UnsizedArray(_) => {
                        return Err(spanned_error(span, "array of unsized arrays is invalid"));
                    }
                };
                self.extract_decl_type_in_scope(array_ty, current_const, *declarator, scope)
            }
        }
    }

    fn hir_ty_to_ty(hir_ty: &HirTy) -> Ty {
        hir_ty_to_ty(hir_ty)
    }

    fn peel_typedef_array_elem(&self, ty: Ty, span: RustSpan) -> Option<Ty> {
        if let Some(elem) = array_elem_ty(ty) {
            return Some(elem);
        }
        if let TyKind::RigidTy(RigidTy::Adt(def, _)) = ty.kind() {
            let hir_ty = HirTy {
                kind: rustc_public_generative::HirTyKind::Adt(def.0, vec![]),
                span,
            };
            let peeled = self.decl_resolver.peel_constexpr_typedef_hir(hir_ty);
            if let rustc_public_generative::HirTyKind::Array(_, _) = &peeled.kind {
                let peeled_ty = Self::hir_ty_to_ty(&peeled);
                return array_elem_ty(peeled_ty);
            }
        }
        None
    }

    fn sig_cty_to_cty(sig_ty: &co2_crate_sig::CTy) -> CTy {
        match sig_ty {
            co2_crate_sig::CTy::Ty(hir_ty) => CTy::Ty(Self::hir_ty_to_ty(hir_ty)),
            co2_crate_sig::CTy::Function(sig) => {
                // Convert FunctionSignature to FnSig
                let abi = match sig.abi {
                    rustc_public_generative::FunctionAbi::Rust => Abi::Rust,
                    rustc_public_generative::FunctionAbi::C => Abi::C { unwind: false },
                };
                let safety = if sig.is_unsafe {
                    Safety::Unsafe
                } else {
                    Safety::Safe
                };
                // Convert inputs to Ty and add output at the end
                let mut inputs_and_output: Vec<Ty> = sig
                    .inputs
                    .iter()
                    .map(|input| Self::hir_ty_to_ty(&input.ty))
                    .collect();
                inputs_and_output.push(Self::hir_ty_to_ty(&sig.output));
                CTy::Function(FnSig {
                    inputs_and_output,
                    c_variadic: sig.c_variadic,
                    safety,
                    abi,
                })
            }
            co2_crate_sig::CTy::UnsizedArray(hir_ty) => {
                CTy::UnsizedArray(Self::hir_ty_to_ty(hir_ty))
            }
        }
    }
}

pub(crate) fn hir_ty_to_ty(hir_ty: &HirTy) -> Ty {
    match &hir_ty.kind {
        rustc_public_generative::HirTyKind::Bool => Ty::bool_ty(),
        rustc_public_generative::HirTyKind::Char => Ty::from_rigid_kind(RigidTy::Char),
        rustc_public_generative::HirTyKind::Str => Ty::from_rigid_kind(RigidTy::Str),
        &rustc_public_generative::HirTyKind::Int(int_ty) => Ty::signed_ty(int_ty),
        &rustc_public_generative::HirTyKind::Uint(uint_ty) => Ty::unsigned_ty(uint_ty),
        &rustc_public_generative::HirTyKind::Float(float_ty) => {
            Ty::from_rigid_kind(RigidTy::Float(float_ty))
        }
        rustc_public_generative::HirTyKind::Tuple(items) => {
            Ty::from_rigid_kind(RigidTy::Tuple(items.iter().map(hir_ty_to_ty).collect()))
        }
        rustc_public_generative::HirTyKind::RawPtr(mutability, inner) => {
            Ty::from_rigid_kind(RigidTy::RawPtr(hir_ty_to_ty(inner), *mutability))
        }
        rustc_public_generative::HirTyKind::Array(len, inner) => {
            let ty_const = match len {
                rustc_public_generative::HirTyConst::Literal(value) => {
                    TyConst::try_from_target_usize((*value).try_into().unwrap()).unwrap()
                }
                rustc_public_generative::HirTyConst::ConstDef(_def_id) => {
                    todo!()
                }
            };
            Ty::from_rigid_kind(RigidTy::Array(hir_ty_to_ty(inner), ty_const))
        }
        rustc_public_generative::HirTyKind::Adt(def_id, generic_args) => {
            if generic_args.is_empty() {
                return CrateItem(*def_id).ty();
            }
            let args = generic_args
                .iter()
                .map(|arg| match arg {
                    rustc_public_generative::HirGenericArg::Ty(ty) => {
                        GenericArgKind::Type(hir_ty_to_ty(ty))
                    }
                    rustc_public_generative::HirGenericArg::Lifetime(_) => {
                        GenericArgKind::Lifetime(Region {
                            kind: RegionKind::ReStatic,
                        })
                    }
                })
                .collect();
            Ty::from_rigid_kind(RigidTy::Adt(AdtDef(*def_id), GenericArgs(args)))
        }
        rustc_public_generative::HirTyKind::FnPtr(sig) => {
            let sig = sig.as_ref();
            let abi = match sig.abi {
                rustc_public_generative::FunctionAbi::Rust => Abi::Rust,
                rustc_public_generative::FunctionAbi::C => Abi::C { unwind: false },
            };
            let safety = if sig.is_unsafe {
                Safety::Unsafe
            } else {
                Safety::Safe
            };
            let mut inputs_and_output: Vec<Ty> = sig
                .inputs
                .iter()
                .map(|input| hir_ty_to_ty(&input.ty))
                .collect();
            inputs_and_output.push(hir_ty_to_ty(&sig.output));
            Ty::from_rigid_kind(RigidTy::FnPtr(Binder::dummy(FnSig {
                inputs_and_output,
                c_variadic: sig.c_variadic,
                safety,
                abi,
            })))
        }
        rustc_public_generative::HirTyKind::Ref(mutability, lifetime, inner) => {
            let region = match lifetime {
                rustc_public_generative::HirLifetime::Static
                | rustc_public_generative::HirLifetime::Param(_) => Region {
                    kind: RegionKind::ReStatic,
                },
            };
            Ty::from_rigid_kind(RigidTy::Ref(region, hir_ty_to_ty(inner), *mutability))
        }
        rustc_public_generative::HirTyKind::Never => Ty::from_rigid_kind(RigidTy::Never),
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

fn prim_ty_to_ty(primitive_ty: co2_crate_sig::PrimitiveTy) -> Ty {
    match primitive_ty {
        co2_crate_sig::PrimitiveTy::Bool => Ty::bool_ty(),
        co2_crate_sig::PrimitiveTy::Str => Ty::from_rigid_kind(RigidTy::Str),
        co2_crate_sig::PrimitiveTy::IntTy(int_ty) => Ty::from_rigid_kind(RigidTy::Int(int_ty)),
        co2_crate_sig::PrimitiveTy::UintTy(uint_ty) => Ty::from_rigid_kind(RigidTy::Uint(uint_ty)),
        co2_crate_sig::PrimitiveTy::FloatTy(float_ty) => {
            Ty::from_rigid_kind(RigidTy::Float(float_ty))
        }
    }
}
