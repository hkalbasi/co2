use std::collections::HashSet;

use co2_ast::{
    BinOp, Constant, DeclarationSpecifier, Declarator, Expression, Span, Spanned,
    StructOrUnionKind, TypeName, TypeQualifier, TypeResolver, TypeSpecifier, UnaryOp,
};
use rustc_public_generative::{FunctionAbi, FunctionSignature, HirTy, HirTyConst, HirTyKind};
use rustc_public_generative::{
    HirGenericArg,
    rustc_public::{
        DefId,
        mir::Mutability,
        ty::{FloatTy, IntTy, UintTy},
    },
};

use crate::{CrateSigCtx, LocalResolver, LocalResolverBase};

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
}

impl CompressedTypeSpecifier {
    pub fn build(
        specifiers: Vec<Spanned<DeclarationSpecifier<LocalResolver>>>,
    ) -> Result<Self, String> {
        let specifiers = specifiers
            .into_iter()
            .filter_map(|x| match x.0 {
                DeclarationSpecifier::TypeSpecifier(s) => Some(s.0),
                DeclarationSpecifier::TypeQualifier(_) => None,
                DeclarationSpecifier::StorageSpecifier(_) => None,
                DeclarationSpecifier::FunctionSpecifier(_) => None,
            })
            .collect::<Vec<_>>();
        if specifiers.is_empty() {
            return Err("no type specifier found".to_owned());
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
                    _ => break 'b,
                });
            }
        }
        enum Base {
            Int,
            Double,
            Char,
        }
        let mut base = None;
        let mut signed = None;
        let mut long = 0u32;
        let mut short = 0u32;
        for spec in specifiers {
            match spec {
                TypeSpecifier::Int | TypeSpecifier::Char | TypeSpecifier::Double => {
                    if base.is_some() {
                        return Err("duplicate base specifier found".to_owned());
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
                        return Err("duplicate sign specifier found".to_owned());
                    }
                    signed = Some(matches!(spec, TypeSpecifier::Signed));
                }
                TypeSpecifier::Bool
                | TypeSpecifier::Void
                | TypeSpecifier::Float
                | TypeSpecifier::StructOrUnion { .. }
                | TypeSpecifier::Enum(_)
                | TypeSpecifier::TypedefName(_) => {
                    return Err("This specifier should be used alone".to_owned());
                }
            }
        }
        let base = base.unwrap_or(Base::Int);
        Ok(CompressedTypeSpecifier::PrimitiveTy(match base {
            Base::Int => {
                let signed = signed.unwrap_or(true);
                match (long, short, signed) {
                    (1.., 1.., _) => return Err("Mixed short and long".to_owned()),
                    (3.., _, _) => return Err("long repeated too many times".to_owned()),
                    (_, 2.., _) => return Err("short repeated too many times".to_owned()),
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
                    return Err("short double is invalid".to_owned());
                }
                if signed.is_some() {
                    return Err("signedness for double is invalid".to_owned());
                }
                PrimitiveTy::FloatTy(if long > 0 {
                    FloatTy::F128
                } else {
                    FloatTy::F64
                })
            }
            Base::Char => {
                if short > 0 || long > 0 {
                    return Err("short and long char is invalid".to_owned());
                }
                match signed {
                    Some(true) => PrimitiveTy::IntTy(IntTy::I8),
                    Some(false) => PrimitiveTy::UintTy(UintTy::U8),
                    None => PrimitiveTy::IntTy(IntTy::I8),
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
    ) -> Result<(String, FunctionSignature, Vec<String>), String> {
        self.resolver
            .borrow_mut()
            .lower_function_signature(base, base_const, declarator)
    }

    pub(crate) fn lower_value_decl_ctype(
        &mut self,
        base: CTy,
        base_const: bool,
        declarator: Spanned<Declarator<LocalResolver>>,
        resolver: &LocalResolver,
    ) -> (
        String,
        CTy,
        Option<co2_ast::Spanned<co2_ast::Initializer<LocalResolver>>>,
    ) {
        let span = declarator.1;
        match self.extract_decl_type_with_consts(base, base_const, declarator, resolver) {
            Ok((ty, name, array_len)) => {
                let Some(name) = name else {
                    self.terminate_with_error(span, "Unexpected abstract declarator");
                };
                (name, ty, array_len)
            }
            Err(e) => {
                self.terminate_with_error(span, &e);
            }
        }
    }

    fn extract_decl_type_with_consts(
        &mut self,
        current: CTy,
        current_const: bool,
        (decl, span): Spanned<Declarator<LocalResolver>>,
        resolver: &LocalResolver,
    ) -> Result<
        (
            CTy,
            Option<String>,
            Option<co2_ast::Spanned<co2_ast::Initializer<LocalResolver>>>,
        ),
        String,
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
                        let param_base_const = has_const_qualifier_in_decl_specs(&param.0);
                        let param_base = self.base_ty_of_decl(param.0, span);
                        let (param_decl_ty, _) = self.resolver.borrow_mut().extract_decl_type(
                            param_base,
                            param_base_const,
                            param.1,
                        )?;
                        let param_ty = match param_decl_ty {
                            CTy::Ty(ty) => {
                                if let HirTyKind::Array(_, inner) = ty.kind {
                                    HirTy::new_ptr(*inner, Mutability::Mut, ty.span)
                                } else {
                                    ty
                                }
                            }
                            CTy::Function(sig) => {
                                // C adjusts function parameters to function pointers.
                                HirTy {
                                    kind: HirTyKind::FnPtr(Box::new(sig)),
                                    span: rust_span,
                                }
                            }
                            CTy::UnsizedArray(elem) => {
                                let span = elem.span;
                                HirTy::new_ptr(elem, Mutability::Mut, span)
                            }
                        };
                        inputs.push(param_ty);
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
                        return Err("function returning function is not valid".to_owned());
                    }
                    CTy::UnsizedArray(_) => {
                        return Err("function returning unsized array is not valid".to_owned());
                    }
                };
                self.extract_decl_type_with_consts(function_ty, false, *declarator, resolver)
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
                    CTy::Function(sig) => CTy::Ty(self.resolver.borrow().maybe_uninit_of(
                        HirTy {
                            kind: HirTyKind::FnPtr(Box::new(sig)),
                            span: rust_span,
                        },
                        rust_span,
                    )?),
                    CTy::UnsizedArray(inner) => CTy::Ty(HirTy::new_ptr(
                        HirTy::new_ptr(inner, Mutability::Mut, rust_span),
                        ptr_mutability,
                        rust_span,
                    )),
                };
                let next_const = qualifiers
                    .iter()
                    .any(|(q, _)| matches!(q, TypeQualifier::Const));
                self.extract_decl_type_with_consts(ptr_or_fn_ptr, next_const, *declarator, resolver)
            }
            Declarator::ArrayDeclarator {
                declarator,
                subscription,
            } => {
                let inner = match current {
                    CTy::Ty(inner) => inner,
                    CTy::Function(_) => {
                        return Err("array of functions is not valid".to_owned());
                    }
                    CTy::UnsizedArray(_) => {
                        return Err("array of unsized arrays is not valid".to_owned());
                    }
                };
                let len = if let Some(len) = subscription.0.raw.constant_len() {
                    (HirTyConst::Literal(len as usize), None)
                } else if subscription.0.raw.is_unsized() {
                    return self.extract_decl_type_with_consts(
                        CTy::UnsizedArray(inner),
                        current_const,
                        *declarator,
                        resolver,
                    );
                } else {
                    let def_id = subscription
                        .0
                        .array_len_const
                        .as_ref()
                        .ok_or_else(|| "missing registered array size constant".to_owned())?;
                    let expr = self
                        .resolver
                        .borrow()
                        .lookup_array_len_const_expr(*def_id)
                        .ok_or_else(|| {
                            "missing registered array size constant expression".to_owned()
                        })?;
                    let literal_len = self.eval_array_len_expr(&expr)?;
                    (HirTyConst::Literal(literal_len), None)
                };
                let array_ty = CTy::Ty(HirTy::new_array(inner, len.0, rust_span));
                let (ty, name, nested_len) = self.extract_decl_type_with_consts(
                    array_ty,
                    current_const,
                    *declarator,
                    resolver,
                )?;
                Ok((ty, name, nested_len.or(len.1)))
            }
        }
    }

    pub(crate) fn lower_rust_function_signature(
        &mut self,
        sig: co2_ast::RustFunctionSignature<LocalResolver>,
    ) -> (String, FunctionSignature, Vec<String>) {
        let name = sig.name.0.1;
        let output = self.lower_rust_ty(sig.ret_ty);
        let mut inputs = Vec::new();
        let mut param_names = Vec::new();
        for param in sig.params {
            inputs.push(self.lower_rust_ty(param.ty));
            param_names.push(param.name.0.1);
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
            param_names,
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
    ) -> Result<usize, String> {
        let value = self.eval_const_expr(expr)?;
        usize::try_from(value)
            .map_err(|_| format!("array size must be a non-negative integer, got {value}"))
    }

    pub(crate) fn eval_const_expr(
        &mut self,
        expr: &Spanned<Expression<LocalResolver>>,
    ) -> Result<i128, String> {
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

impl LocalResolverBase {
    pub(crate) fn has_local_enum_const(&self, def_id: DefId) -> bool {
        self.enum_const_values.contains_key(&def_id)
            || self
                .struct_manager
                .pending_enum_consts
                .iter()
                .any(|pending| pending.def_id == def_id)
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
                    .map(|param| self.hir_ty_of_rust_ty(param))
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
        }
    }

    fn hir_generic_args_of_resolved_path(
        &mut self,
        generic_args: &[Spanned<co2_ast::RustTy<LocalResolver>>],
    ) -> Vec<HirGenericArg> {
        generic_args
            .iter()
            .map(|arg| HirGenericArg::Ty(self.hir_ty_of_rust_ty(arg.clone())))
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
            } => HirTy::adt(
                *def_id,
                self.hir_generic_args_of_resolved_path(generic_args),
                span,
            ),
            crate::DefOrLocal::Const(_) => panic!("invalid const in type position"),
            crate::DefOrLocal::AssocMethod { .. } => {
                panic!("invalid associated method in type position")
            }
            crate::DefOrLocal::Local(_) => panic!("invalid parsing"),
            crate::DefOrLocal::FuncName => panic!("invalid __func__ in type position"),
            crate::DefOrLocal::Prim(primitive_ty) => self.hir_ty_of_prim(*primitive_ty, span),
            crate::DefOrLocal::UnrepresentableType(ty) => match ty {
                CTy::Ty(ty) => ty.clone(),
                CTy::Function(_) => panic!("function is invalid as a type name"),
                CTy::UnsizedArray(_) => panic!("unsized array is invalid as a type name"),
            },
        }
    }

    pub(crate) fn eval_array_len_expr(
        &mut self,
        expr: &Spanned<Expression<LocalResolver>>,
    ) -> Result<usize, String> {
        let value = self.eval_const_expr(expr)?;
        usize::try_from(value)
            .map_err(|_| format!("array size must be a non-negative integer, got {value}"))
    }

    // TODO: this function is probably wrong and should be removed
    pub(crate) fn eval_const_expr(
        &mut self,
        (expr, span): &Spanned<Expression<LocalResolver>>,
    ) -> Result<i128, String> {
        match expr {
            Expression::Constant(Constant::Int(v, _)) => Ok(*v),
            Expression::Constant(Constant::Char(ch)) => Ok(*ch as i128),
            Expression::Identifier((resolved, _)) => match resolved {
                crate::DefOrLocal::Const(def_id) => {
                    if self.has_local_enum_const(*def_id) {
                        self.eval_local_const(*def_id)
                    } else if let Some(val) = self.hir_ctx.dependency_const_value(*def_id) {
                        match val {
                            rustc_public_generative::DependencyConstValue::Bool(b) => Ok(b as i128),
                            rustc_public_generative::DependencyConstValue::Char(c) => Ok(c as i128),
                            rustc_public_generative::DependencyConstValue::I8(i) => Ok(i as i128),
                            rustc_public_generative::DependencyConstValue::I16(i) => Ok(i as i128),
                            rustc_public_generative::DependencyConstValue::I32(i) => Ok(i as i128),
                            rustc_public_generative::DependencyConstValue::I64(i) => Ok(i as i128),
                            rustc_public_generative::DependencyConstValue::I128(i) => Ok(i),
                            rustc_public_generative::DependencyConstValue::Isize(i) => {
                                Ok(i as i128)
                            }
                            rustc_public_generative::DependencyConstValue::U8(u) => Ok(u as i128),
                            rustc_public_generative::DependencyConstValue::U16(u) => Ok(u as i128),
                            rustc_public_generative::DependencyConstValue::U32(u) => Ok(u as i128),
                            rustc_public_generative::DependencyConstValue::U64(u) => Ok(u as i128),
                            rustc_public_generative::DependencyConstValue::U128(u) => Ok(u as i128),
                            rustc_public_generative::DependencyConstValue::Usize(u) => {
                                Ok(u as i128)
                            }
                            rustc_public_generative::DependencyConstValue::F32(_)
                            | rustc_public_generative::DependencyConstValue::F64(_) => {
                                Err("float constant in array size".to_owned())
                            }
                        }
                    } else {
                        Err(format!(
                            "unsupported identifier in constant expression: {:?}",
                            resolved
                        ))
                    }
                }
                crate::DefOrLocal::Def { def_id, .. } => self.eval_local_const(*def_id),
                _ => Err(format!(
                    "unsupported identifier in constant expression: {:?}",
                    resolved
                )),
            },
            Expression::UnaryOp(op, inner) => {
                let inner = self.eval_const_expr(inner)?;
                match op {
                    UnaryOp::Plus => Ok(inner),
                    UnaryOp::Minus => Ok(-inner),
                    UnaryOp::Not => Ok((inner == 0) as i128),
                    UnaryOp::Com => Ok(!inner),
                    _ => Err("unsupported unary op in array size".to_owned()),
                }
            }
            Expression::BinOp(lhs, op, rhs) => {
                let lhs = self.eval_const_expr(lhs)?;
                let rhs = self.eval_const_expr(rhs)?;
                match op {
                    BinOp::Add => Ok(lhs + rhs),
                    BinOp::Sub => Ok(lhs - rhs),
                    BinOp::Mul => Ok(lhs * rhs),
                    BinOp::Div => Ok(lhs / rhs),
                    BinOp::Rem => Ok(lhs % rhs),
                    BinOp::BitOr => Ok(lhs | rhs),
                    BinOp::BitXor => Ok(lhs ^ rhs),
                    BinOp::BitAnd => Ok(lhs & rhs),
                    BinOp::Eq => Ok((lhs == rhs) as i128),
                    BinOp::Lt => Ok((lhs < rhs) as i128),
                    BinOp::Le => Ok((lhs <= rhs) as i128),
                    BinOp::Ne => Ok((lhs != rhs) as i128),
                    BinOp::Ge => Ok((lhs >= rhs) as i128),
                    BinOp::Gt => Ok((lhs > rhs) as i128),
                    BinOp::Shl => Ok(lhs << rhs),
                    BinOp::Shr => Ok(lhs >> rhs),
                    BinOp::And => Ok(((lhs != 0) && (rhs != 0)) as i128),
                    BinOp::Or => Ok(((lhs != 0) || (rhs != 0)) as i128),
                    BinOp::Comma | BinOp::Assign => {
                        Err("unsupported binary op in array size".to_owned())
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
                let target_ty = self.lower_type_name_for_const(*type_name.clone(), *span)?;
                self.cast_const_int(value, &target_ty)
            }
            Expression::SizeofType(type_name) => {
                let ty = self.lower_type_name_for_const(*type_name.clone(), *span)?;
                Ok(self.sizeof_hir_ty(&ty)?.0 as i128)
            }
            Expression::Sizeof(expr) => {
                let ty = self.type_of_expr_for_sizeof(expr)?;
                Ok(self.sizeof_hir_ty(&ty)?.0 as i128)
            }
            Expression::Offsetof {
                ty: type_name,
                field: _,
            } => {
                let ty = self.lower_type_name_for_const(*type_name.clone(), *span)?;
                Ok(self.sizeof_hir_ty(&ty)?.0 as i128)
            }
            _ => Err("unsupported constant expression in array size".to_owned()),
        }
    }

    pub(crate) fn eval_local_const(&mut self, def_id: DefId) -> Result<i128, String> {
        if let Some(val) = self.enum_const_values.get(&def_id) {
            return Ok(*val);
        }

        let mir_info = self
            .struct_manager
            .pending_enum_consts
            .iter()
            .find(|e| e.def_id == def_id)
            .map(|e| &e.mir_info)
            .ok_or_else(|| format!("could not find enum constant {:?}", def_id))?;

        let value = match &mir_info {
            crate::MirOwnerInfo::EnumConstZeroed => 0,
            crate::MirOwnerInfo::EnumConstExplicit { initializer, .. } => {
                self.eval_const_expr(&initializer.clone())?
            }
            crate::MirOwnerInfo::EnumConstPrevPlus(prev_id, _) => {
                let prev_id = *prev_id;
                self.eval_local_const(prev_id)? + 1
            }
            _ => return Err(format!("def {:?} is not an enum constant", def_id)),
        };

        self.enum_const_values.insert(def_id, value);
        Ok(value)
    }

    fn hir_ty_of_dependency_const_value(
        &self,
        value: rustc_public_generative::DependencyConstValue,
        span: rustc_public_generative::rustc_public::ty::Span,
    ) -> HirTy {
        match value {
            rustc_public_generative::DependencyConstValue::Bool(_) => HirTy {
                kind: HirTyKind::Bool,
                span,
            },
            rustc_public_generative::DependencyConstValue::Char(_) => {
                HirTy::signed_ty(IntTy::I32, span)
            }
            rustc_public_generative::DependencyConstValue::I8(_) => {
                HirTy::signed_ty(IntTy::I8, span)
            }
            rustc_public_generative::DependencyConstValue::I16(_) => {
                HirTy::signed_ty(IntTy::I16, span)
            }
            rustc_public_generative::DependencyConstValue::I32(_) => {
                HirTy::signed_ty(IntTy::I32, span)
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
            .map(|value| self.hir_ty_of_dependency_const_value(value, span))
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

    fn cast_const_int(&self, value: i128, target_ty: &HirTy) -> Result<i128, String> {
        match target_ty.kind {
            HirTyKind::Bool => Ok((value != 0) as i128),
            HirTyKind::Char => {
                let codepoint =
                    u32::try_from(value).map_err(|_| format!("invalid char cast value {value}"))?;
                char::from_u32(codepoint)
                    .map(|ch| ch as i128)
                    .ok_or_else(|| format!("invalid char cast value {value}"))
            }
            HirTyKind::Int(IntTy::I8) => Ok((value as i8) as i128),
            HirTyKind::Int(IntTy::I16) => Ok((value as i16) as i128),
            HirTyKind::Int(IntTy::I32) => Ok((value as i32) as i128),
            HirTyKind::Int(IntTy::I64) => Ok((value as i64) as i128),
            HirTyKind::Int(IntTy::I128) => Ok(value),
            HirTyKind::Int(IntTy::Isize) => Ok((value as isize) as i128),
            HirTyKind::Uint(UintTy::U8) => Ok((value as u8) as i128),
            HirTyKind::Uint(UintTy::U16) => Ok((value as u16) as i128),
            HirTyKind::Uint(UintTy::U32) => Ok((value as u32) as i128),
            HirTyKind::Uint(UintTy::U64) => Ok((value as u64) as i128),
            HirTyKind::Uint(UintTy::U128) => Ok((value as u128) as i128),
            HirTyKind::Uint(UintTy::Usize) => Ok((value as usize) as i128),
            _ => Err(format!(
                "unsupported cast target in constant expression: {:?}",
                target_ty.kind
            )),
        }
    }

    fn type_of_expr_for_sizeof(
        &mut self,
        (expr, span): &Spanned<Expression<LocalResolver>>,
    ) -> Result<HirTy, String> {
        let rust_span = self.co2_span_to_rustc(*span);
        match expr {
            Expression::Constant(Constant::String(s)) => Ok(HirTy::new_array(
                HirTy::signed_ty(IntTy::I8, rust_span),
                HirTyConst::Literal(s.chars().count() + 1),
                rust_span,
            )),
            Expression::Identifier((resolved, _)) => match resolved {
                crate::DefOrLocal::Local(local) => self
                    .local_tys
                    .get(local)
                    .cloned()
                    .ok_or_else(|| format!("missing local type for local {local}")),
                crate::DefOrLocal::Def { def_id, .. } => self
                    .maybe_const_eval_named_ty(*def_id, rust_span)
                    .ok_or_else(|| format!("missing global or const type for def {def_id:?}")),
                crate::DefOrLocal::Const(def_id) => self
                    .scalar_const_ty(*def_id, rust_span)
                    .ok_or_else(|| format!("missing scalar constant value for def {def_id:?}")),
                crate::DefOrLocal::AssocMethod { .. } => {
                    Err("associated method path is invalid in sizeof".to_owned())
                }
                crate::DefOrLocal::FuncName => Err("__func__ is invalid in sizeof".to_owned()),
                crate::DefOrLocal::Prim(primitive_ty) => {
                    Ok(self.hir_ty_of_prim(*primitive_ty, rust_span))
                }
                _ => Err("unsupported identifier in sizeof(array size expr)".to_owned()),
            },
            Expression::Field(base, field) => {
                let base_ty = self.type_of_expr_for_sizeof(base)?;
                self.lookup_field_ty_for_sizeof(base_ty, &field.0)
            }
            Expression::Arrow(base, field) => {
                let base_ty = self.type_of_expr_for_sizeof(base)?;
                let pointee = match base_ty.kind {
                    HirTyKind::RawPtr(_, inner) | HirTyKind::Ref(_, _, inner) => *inner,
                    _ => {
                        return Err(
                            "arrow base must be a pointer in sizeof(array size expr)".to_owned()
                        );
                    }
                };
                self.lookup_field_ty_for_sizeof(pointee, &field.0)
            }
            Expression::Subscript(base, _) => {
                let base_ty = self.type_of_expr_for_sizeof(base)?;
                match base_ty.kind {
                    HirTyKind::Array(_, inner)
                    | HirTyKind::RawPtr(_, inner)
                    | HirTyKind::Ref(_, _, inner) => Ok(*inner),
                    _ => Err(
                        "subscript base must be array or pointer in sizeof(array size expr)"
                            .to_owned(),
                    ),
                }
            }
            Expression::Cast { type_name, .. } => {
                self.lower_type_name_for_const(*type_name.clone(), *span)
            }
            Expression::UnaryOp(op, inner) => {
                match op {
                    UnaryOp::Deref => {
                        let inner_ty = self.type_of_expr_for_sizeof(inner)?;
                        match inner_ty.kind {
                            HirTyKind::RawPtr(_, pointee) | HirTyKind::Ref(_, _, pointee) => {
                                Ok(*pointee)
                            }
                            _ => Err("cannot dereference non-pointer in sizeof(array size expr)"
                                .to_owned()),
                        }
                    }
                    UnaryOp::AddrOf => {
                        let inner_ty = self.type_of_expr_for_sizeof(inner)?;
                        Ok(HirTy::new_ptr(inner_ty, Mutability::Mut, rust_span))
                    }
                    UnaryOp::Plus | UnaryOp::Minus | UnaryOp::Not | UnaryOp::Com => {
                        self.type_of_expr_for_sizeof(inner)
                    }
                }
            }
            _ => Err("unsupported sizeof operand in array size".to_owned()),
        }
    }

    fn lookup_field_ty_for_sizeof(
        &self,
        base_ty: HirTy,
        field_name: &str,
    ) -> Result<HirTy, String> {
        let base_ty = self.peel_typedefs_for_sizeof(base_ty);
        let HirTyKind::Adt(def, _) = base_ty.kind else {
            return Err(
                "field access requires struct or union type in sizeof(array size expr)".to_owned(),
            );
        };
        self.adt_field_ty(def, field_name)
            .ok_or_else(|| format!("unknown field `{field_name}` in sizeof(array size expr)"))
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

    fn lower_type_name_for_const(
        &mut self,
        type_name: TypeName<LocalResolver>,
        span: Span,
    ) -> Result<HirTy, String> {
        if type_name.abstract_declarator.is_none() {
            if let [
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
                        CTy::Ty(ty) => return Ok(ty.clone()),
                        CTy::Function(_) => {
                            return Err("function is invalid as a type name".to_owned());
                        }
                        CTy::UnsizedArray(_) => {
                            return Err("unsized array is invalid as a type name".to_owned());
                        }
                    },
                    crate::DefOrLocal::Def { def_id, .. } => {
                        if let Some(ty) = self.maybe_const_eval_named_ty(*def_id, rust_span) {
                            return Ok(ty);
                        }
                    }
                    crate::DefOrLocal::Const(def_id) => {
                        return self.scalar_const_ty(*def_id, rust_span).ok_or_else(|| {
                            format!("missing scalar constant value for def {def_id:?}")
                        });
                    }
                    _ => {}
                }
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
            Some(decl) => self.extract_decl_type(base, base_const, decl)?.0,
        };
        match ty {
            CTy::Ty(ty) => Ok(ty),
            CTy::Function(_) => Err("function is invalid as a type name".to_owned()),
            CTy::UnsizedArray(_) => Err("unsized array is invalid as a type name".to_owned()),
        }
    }

    fn sizeof_hir_ty(&self, ty: &HirTy) -> Result<(usize, usize), String> {
        match &ty.kind {
            HirTyKind::Bool
            | HirTyKind::Char
            | HirTyKind::Int(IntTy::I8)
            | HirTyKind::Uint(UintTy::U8) => Ok((1, 1)),
            HirTyKind::Int(IntTy::I16) | HirTyKind::Uint(UintTy::U16) => Ok((2, 2)),
            HirTyKind::Int(IntTy::I32)
            | HirTyKind::Uint(UintTy::U32)
            | HirTyKind::Float(FloatTy::F32) => Ok((4, 4)),
            HirTyKind::Int(IntTy::I64)
            | HirTyKind::Uint(UintTy::U64)
            | HirTyKind::Int(IntTy::Isize)
            | HirTyKind::Uint(UintTy::Usize)
            | HirTyKind::Float(FloatTy::F64)
            | HirTyKind::RawPtr(..)
            | HirTyKind::Ref(..)
            | HirTyKind::FnPtr(_) => Ok((8, 8)),
            HirTyKind::Int(IntTy::I128)
            | HirTyKind::Uint(UintTy::U128)
            | HirTyKind::Float(FloatTy::F128) => Ok((16, 16)),
            HirTyKind::Float(FloatTy::F16) => Ok((2, 2)),
            HirTyKind::Tuple(inner) if inner.is_empty() => Ok((0, 1)),
            HirTyKind::Array(HirTyConst::Literal(len), inner) => {
                let (elem_size, elem_align) = self.sizeof_hir_ty(inner)?;
                Ok((elem_size * len, elem_align))
            }
            HirTyKind::Adt(def, _) => {
                if let Some((kind, fields)) = self.adt_layout_info(*def) {
                    let mut size = 0usize;
                    let mut align = 1usize;
                    match kind {
                        co2_ast::StructOrUnionKind::Struct => {
                            for field in fields {
                                let (field_size, field_align) = self.sizeof_hir_ty(&field)?;
                                align = align.max(field_align);
                                size = round_up(size, field_align);
                                size += field_size;
                            }
                        }
                        co2_ast::StructOrUnionKind::Union => {
                            for field in fields {
                                let (field_size, field_align) = self.sizeof_hir_ty(&field)?;
                                align = align.max(field_align);
                                size = size.max(field_size);
                            }
                        }
                    }
                    Ok((round_up(size, align), align))
                } else if let Some(ty) = self.typedef_tys.get(def) {
                    self.sizeof_hir_ty(ty)
                } else {
                    Err("unsupported ADT in sizeof(array size expr)".to_owned())
                }
            }
            _ => Err("unsupported type in sizeof(array size expr)".to_owned()),
        }
    }

    pub(crate) fn lower_function_signature(
        &mut self,
        base: CTy,
        base_const: bool,
        declarator: Spanned<Declarator<LocalResolver>>,
    ) -> Result<(String, FunctionSignature, Vec<String>), String> {
        let parsed_param_names = function_param_names(&declarator.0);
        let (decl_ty, name) = self.extract_decl_type(base, base_const, declarator)?;
        let name = name.ok_or_else(|| "missing function name".to_owned())?;
        let CTy::Function(sig) = decl_ty else {
            return Err("it wasn't function".to_owned());
        };
        let names = parsed_param_names
            .unwrap_or_else(|| vec![None; sig.inputs.len()])
            .into_iter()
            .enumerate()
            .map(|(idx, n)| n.unwrap_or_else(|| format!("arg{idx}")))
            .collect();
        Ok((name, sig, names))
    }

    pub(crate) fn lower_value_decl_type_maybe_unsized(
        &mut self,
        base: CTy,
        base_const: bool,
        declarator: Spanned<Declarator<LocalResolver>>,
    ) -> (String, HirTy, bool) {
        let span = declarator.1;
        match self.try_lower_value_decl_type_maybe_unsized(base, base_const, declarator) {
            Ok(x) => x,
            Err(e) => {
                self.terminate_with_error(span, &e);
            }
        }
    }

    pub(crate) fn try_lower_value_decl_type_maybe_unsized(
        &mut self,
        base: CTy,
        base_const: bool,
        declarator: Spanned<Declarator<LocalResolver>>,
    ) -> Result<(String, HirTy, bool), String> {
        let span = declarator.1;
        let (decl_ty, name) = self.extract_decl_type(base, base_const, declarator)?;
        let name = name.ok_or_else(|| "missing declaration name".to_owned())?;
        match decl_ty {
            CTy::Ty(ty) => Ok((name, ty, false)),
            CTy::Function(_) => {
                Err("function is not a first-class declaration type in this context".to_owned())
            }
            CTy::UnsizedArray(ty) => {
                let rust_span = self.co2_span_to_rustc(span);
                Ok((
                    name,
                    HirTy::new_array(ty, HirTyConst::Literal(0), rust_span),
                    true,
                ))
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
        let span = self.co2_span_to_rustc(parser_span);
        let specifier = match CompressedTypeSpecifier::build(specifiers) {
            Ok(s) => s,
            Err(e) => self.terminate_with_error(parser_span, &e),
        };
        let ty = match specifier {
            CompressedTypeSpecifier::Void => HirTy::new_tuple(vec![], span),
            CompressedTypeSpecifier::PrimitiveTy(ty) => self.hir_ty_of_prim(ty, span),
            CompressedTypeSpecifier::Enum(_) => HirTy::signed_ty(IntTy::I32, span),
            CompressedTypeSpecifier::StructOrUnion { kind: _, specifier } => {
                HirTy::adt(specifier.0, vec![], span)
            }
            CompressedTypeSpecifier::TypedefName((path, path_span)) => match path {
                crate::DefOrLocal::UnrepresentableType(ty) => {
                    return ty;
                }
                _ => self.hir_ty_of_resolved_path(&path, path_span),
            },
        };
        CTy::Ty(ty)
    }

    fn extract_decl_type(
        &mut self,
        current: CTy,
        current_const: bool,
        (decl, span): Spanned<Declarator<LocalResolver>>,
    ) -> Result<(CTy, Option<String>), String> {
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
                        let param_base_const = has_const_qualifier_in_decl_specs(&param.0);
                        let param_base = self.base_ty_of_decl(param.0, span);
                        let (param_decl_ty, _) =
                            self.extract_decl_type(param_base, param_base_const, param.1)?;
                        let param_ty = match param_decl_ty {
                            CTy::Ty(ty) => {
                                if let HirTyKind::Array(_, inner) = ty.kind {
                                    HirTy::new_ptr(*inner, Mutability::Mut, ty.span)
                                } else {
                                    ty
                                }
                            }
                            CTy::Function(sig) => {
                                // C adjusts function parameters to function pointers.
                                HirTy {
                                    kind: HirTyKind::FnPtr(Box::new(sig)),
                                    span: rust_span,
                                }
                            }
                            CTy::UnsizedArray(elem) => {
                                let span = elem.span;
                                HirTy::new_ptr(elem, Mutability::Mut, span)
                            }
                        };
                        inputs.push(param_ty);
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
                        return Err("function returning function is not valid".to_owned());
                    }
                    CTy::UnsizedArray(_) => {
                        return Err("function returning unsized array is not valid".to_owned());
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
                    )?),
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
                        return Err("array of functions is not valid".to_owned());
                    }
                    CTy::UnsizedArray(_) => {
                        return Err("array of unsized arrays is not valid".to_owned());
                    }
                };
                let len = if let Some(len) = subscription.0.raw.constant_len() {
                    HirTyConst::Literal(len as usize)
                } else if subscription.0.raw.is_unsized() {
                    return self.extract_decl_type(
                        CTy::UnsizedArray(inner),
                        current_const,
                        *declarator,
                    );
                } else {
                    let def_id = subscription
                        .0
                        .array_len_const
                        .as_ref()
                        .ok_or_else(|| "Can not calculate subscription".to_owned())?;
                    let expr = self
                        .lookup_array_len_const_expr(*def_id)
                        .ok_or_else(|| "Can not calculate subscription".to_owned())?;
                    HirTyConst::Literal(self.eval_array_len_expr(&expr)?)
                };
                let array_ty = CTy::Ty(HirTy::new_array(inner, len, rust_span));
                self.extract_decl_type(array_ty, current_const, *declarator)
            }
        }
    }

    fn maybe_uninit_of(
        &self,
        inner: HirTy,
        span: rustc_public_generative::rustc_public::ty::Span,
    ) -> Result<HirTy, String> {
        let (def, _) = self.resolver.resolve("core::mem::MaybeUninit")?;
        return Ok(HirTy::adt(def, vec![HirGenericArg::Ty(inner)], span));
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
            "usize" => Some(PrimitiveTy::UintTy(UintTy::Usize)),
            "isize" => Some(PrimitiveTy::IntTy(IntTy::Isize)),
            "_Float32" | "_Float32x" => Some(PrimitiveTy::FloatTy(FloatTy::F32)),
            "_Float64" | "_Float64x" => Some(PrimitiveTy::FloatTy(FloatTy::F64)),
            "_Float128" => Some(PrimitiveTy::FloatTy(FloatTy::F128)),
            _ => None,
        }
    }
}
