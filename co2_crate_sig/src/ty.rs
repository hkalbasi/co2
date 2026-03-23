use co2_ast::{DeclarationSpecifier, Declarator, Span, Spanned, StructOrUnionKind, TypeResolver, TypeSpecifier};
use rustc_public_generative::{FunctionAbi, FunctionSignature, HirTy, HirTyConst, HirTyKind};
use rustc_public_generative::{
    HirGenericArg,
    rustc_public::{
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
        let specifiers = specifiers.into_iter().filter_map(|x| match x.0 {
            DeclarationSpecifier::TypeSpecifier(s) => Some(s.0),
            DeclarationSpecifier::TypeQualifier(_) => None,
            DeclarationSpecifier::StorageSpecifier(_) => None,
        }).collect::<Vec<_>>();
        if specifiers.is_empty() {
            return Err("no type specifier found".to_owned());
        }
        if let [specifier] = specifiers.as_slice() {
            'b: {
                return Ok(match specifier {
                    TypeSpecifier::Void => CompressedTypeSpecifier::Void,
                    TypeSpecifier::Float => CompressedTypeSpecifier::PrimitiveTy(PrimitiveTy::FloatTy(FloatTy::F32)),
                    TypeSpecifier::Bool => CompressedTypeSpecifier::PrimitiveTy(PrimitiveTy::IntTy(IntTy::I8)),
                    &TypeSpecifier::StructOrUnion { kind, specifier } => CompressedTypeSpecifier::StructOrUnion { kind, specifier },
                    &TypeSpecifier::Enum(e) => CompressedTypeSpecifier::Enum(e),
                    TypeSpecifier::TypedefName(t) => CompressedTypeSpecifier::TypedefName(t.clone()),
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
                TypeSpecifier::Int
                | TypeSpecifier::Char
                | TypeSpecifier::Double => {
                    if base.is_some() {
                        return Err("duplicate base specifier found".to_owned());
                    }
                    base = Some(match spec {
                        TypeSpecifier::Int => Base::Int,
                        TypeSpecifier::Char => Base::Char,
                        TypeSpecifier::Double => Base::Double,
                        _ => unreachable!(),
                    });
                },
                TypeSpecifier::Short => short += 1,
                TypeSpecifier::Long => long += 1,
                TypeSpecifier::Signed |
                TypeSpecifier::Unsigned => {
                    if base.is_some() {
                        return Err("duplicate sign specifier found".to_owned());
                    }
                    signed = Some(matches!(spec, TypeSpecifier::Signed));
                },
                TypeSpecifier::Bool
                | TypeSpecifier::Void
                | TypeSpecifier::Float
                | TypeSpecifier::StructOrUnion { .. }
                | TypeSpecifier::Enum(_)
                | TypeSpecifier::TypedefName(_) => {
                    return Err("This specifier should be used alone".to_owned());
                },
            }
        }
        let base = base.unwrap_or(Base::Int);
        Ok(CompressedTypeSpecifier::PrimitiveTy(match base {
            Base::Int => {
                let signed = signed.unwrap_or(true);
                match (long, short, signed) {
                    (1.., 1.., _) => {
                        return Err("Mixed short and long".to_owned())
                    }
                    (3.., _, _) => {
                        return Err("long repeated too many times".to_owned())
                    }
                    (_, 2.., _) => {
                        return Err("short repeated too many times".to_owned())
                    }
                    (0, 0, true) => PrimitiveTy::IntTy(IntTy::I32),
                    (0, 0, false) => PrimitiveTy::UintTy(UintTy::U32),
                    (0, 1, true) => PrimitiveTy::IntTy(IntTy::I16),
                    (0, 1, false) => PrimitiveTy::UintTy(UintTy::U16),
                    (1..=2, 0, true) => PrimitiveTy::IntTy(IntTy::I64),
                    (1..=2, 0, false) => PrimitiveTy::UintTy(UintTy::U64),
                }
            },
            Base::Double => {
                if short > 0 {
                    return Err("short double is invalid".to_owned())
                }
                if signed.is_some() {
                    return Err("signedness for double is invalid".to_owned());
                }
                PrimitiveTy::FloatTy(if long > 0 {
                    FloatTy::F128
                } else {
                    FloatTy::F64
                })
            },
            Base::Char => {
                if short > 0 || long > 0 {
                    return Err("short and long char is invalid".to_owned())
                }
                match signed {
                    Some(true) => PrimitiveTy::IntTy(IntTy::I8),
                    Some(false) => PrimitiveTy::UintTy(UintTy::U8),
                    None => PrimitiveTy::IntTy(IntTy::I8),
                }
            },
        }))
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
        declarator: Spanned<Declarator<LocalResolver>>,
    ) -> Result<(String, FunctionSignature, Vec<String>), String> {
        self.resolver
            .borrow_mut()
            .lower_function_signature(base, declarator)
    }

    pub(crate) fn lower_value_decl_ctype(
        &mut self,
        base: CTy,
        declarator: Spanned<Declarator<LocalResolver>>,
    ) -> (String, CTy) {
        let span = declarator.1;
        match self
            .resolver
            .borrow_mut()
            .extract_decl_type(base, declarator)
        {
            Ok((ty, name)) => {
                let Some(name) = name else {
                    self.terminate_with_error(span, "Unexpected abstract declarator");
                };
                (name, ty)
            }
            Err(e) => {
                self.terminate_with_error(span, &e);
            }
        }
    }

    pub(crate) fn lower_value_decl_type(
        &mut self,
        base: CTy,
        declarator: Spanned<Declarator<LocalResolver>>,
    ) -> (String, HirTy) {
        self.resolver
            .borrow_mut()
            .lower_value_decl_type(base, declarator)
    }
}

impl LocalResolverBase {
    pub(crate) fn lower_function_signature(
        &mut self,
        base: CTy,
        declarator: Spanned<Declarator<LocalResolver>>,
    ) -> Result<(String, FunctionSignature, Vec<String>), String> {
        let parsed_param_names = function_param_names(&declarator.0);
        let (decl_ty, name) = self.extract_decl_type(base, declarator)?;
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

    pub(crate) fn lower_value_decl_type(
        &mut self,
        base: CTy,
        declarator: Spanned<Declarator<LocalResolver>>,
    ) -> (String, HirTy) {
        let span = declarator.1;
        match self.try_lower_value_decl_type(base, declarator) {
            Ok(x) => x,
            Err(e) => {
                self.terminate_with_error(span, &e);
            }
        }
    }

    pub(crate) fn try_lower_value_decl_type(
        &mut self,
        base: CTy,
        declarator: Spanned<Declarator<LocalResolver>>,
    ) -> Result<(String, HirTy), String> {
        let (decl_ty, name) = self.extract_decl_type(base, declarator)?;
        let name = name.ok_or_else(|| "missing declaration name".to_owned())?;
        match decl_ty {
            CTy::Ty(ty) => Ok((name, ty)),
            CTy::Function(_) => {
                Err("function is not a first-class declaration type in this context".to_owned())
            }
            CTy::UnsizedArray(_) => Err(
                "unsized array is not a first-class declaration type in this context".to_owned(),
            ),
        }
    }

    fn hir_ty_of_prim(
        &mut self,
        primitive_ty: PrimitiveTy,
        span: rustc_public_generative::rustc_public::ty::Span,
    ) -> HirTy {
        match primitive_ty {
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
            CompressedTypeSpecifier::PrimitiveTy(ty) => {
                self.hir_ty_of_prim(ty, span)
            }
            CompressedTypeSpecifier::Enum(_) => HirTy::signed_ty(IntTy::I32, span),
            CompressedTypeSpecifier::StructOrUnion { kind: _, specifier } => {
                HirTy::adt(specifier.0, vec![], span)
            }
            CompressedTypeSpecifier::TypedefName((path, _)) => {
                match path {
                    crate::DefOrLocal::Def(def_id) => HirTy::adt(def_id, vec![], span),
                    crate::DefOrLocal::Local(_) => panic!("invalid parsing"),
                    crate::DefOrLocal::Prim(primitive_ty) => self.hir_ty_of_prim(primitive_ty, span),
                    crate::DefOrLocal::UnrepresentableType(ty) => {
                        return ty;
                    }
                }
            }   
        };
        CTy::Ty(ty)
    }

    fn extract_decl_type(
        &mut self,
        current: CTy,
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
                        let param_base = self.base_ty_of_decl(param.0, span);
                        let (param_decl_ty, _) = self.extract_decl_type(param_base, param.1)?;
                        let param_ty = match param_decl_ty {
                            CTy::Ty(ty) => {
                                if let HirTyKind::Array(_, inner) = ty.kind {
                                    HirTy::new_ptr(*inner, Mutability::Mut, ty.span)
                                } else {
                                    ty
                                }
                            }
                            CTy::Function(_) => {
                                return Err(
                                    "function type is invalid in parameter position".to_owned()
                                );
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
                self.extract_decl_type(function_ty, *declarator)
            }
            Declarator::PointerDeclarator { declarator, .. } => {
                let ptr_or_fn_ptr = match current {
                    CTy::Ty(inner) => CTy::Ty(HirTy::new_ptr(inner, Mutability::Mut, rust_span)),
                    CTy::Function(sig) => CTy::Ty(self.maybe_uninit_of(
                        HirTy {
                            kind: HirTyKind::FnPtr(Box::new(sig)),
                            span: rust_span,
                        },
                        rust_span,
                    )?),
                    CTy::UnsizedArray(inner) => CTy::Ty(HirTy::new_ptr(
                        HirTy::new_ptr(inner, Mutability::Mut, rust_span),
                        Mutability::Mut,
                        rust_span,
                    )),
                };
                self.extract_decl_type(ptr_or_fn_ptr, *declarator)
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
                let len = if let Some(len) = subscription.0.constant_len() {
                    HirTyConst::Literal(len as usize)
                } else if subscription.0.is_unsized() {
                    return self.extract_decl_type(CTy::UnsizedArray(inner), *declarator);
                } else {
                    // self.terminate_with_error(subscription.1, "Can not handle complex subscriptions");
                    HirTyConst::Literal(555)
                };
                let array_ty = CTy::Ty(HirTy::new_array(inner, len, rust_span));
                self.extract_decl_type(array_ty, *declarator)
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
        co2_ast::print_errors_and_terminate(
            self.source_name.clone(),
            self.source,
            vec![co2_ast::Rich::custom(span, msg)],
        );
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
            "_Float128" => Some(PrimitiveTy::FloatTy(FloatTy::F128)),
            _ => None,
        }
    }
}
