use co2_ast::{DeclarationSpecifier, Declarator, Span, Spanned, TypeSpecifier};
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

    pub(crate) fn base_ty_of_decl(
        &mut self,
        specifiers: Vec<Spanned<DeclarationSpecifier<LocalResolver>>>,
        parser_span: Span,
    ) -> CTy {
        let span = self.co2_span_to_rustc(parser_span);
        for (specifier, _) in specifiers {
            if let DeclarationSpecifier::TypeSpecifier((type_specifier, _)) = specifier {
                let ty = match type_specifier {
                    TypeSpecifier::Int => HirTy::signed_ty(IntTy::I32, span),
                    TypeSpecifier::Bool => HirTy {
                        kind: HirTyKind::Bool,
                        span,
                    },
                    TypeSpecifier::Void => HirTy::new_tuple(vec![], span),
                    TypeSpecifier::Char => HirTy::signed_ty(IntTy::I8, span),
                    TypeSpecifier::Short => HirTy::signed_ty(IntTy::I16, span),
                    TypeSpecifier::Long => HirTy::signed_ty(IntTy::I64, span),
                    TypeSpecifier::Float => HirTy::float_ty(FloatTy::F32, span),
                    TypeSpecifier::Double => HirTy::float_ty(FloatTy::F64, span),
                    TypeSpecifier::Signed | TypeSpecifier::Unsigned => continue,
                    TypeSpecifier::Enum(_) => HirTy::signed_ty(IntTy::I32, span),
                    TypeSpecifier::StructOrUnion { kind: _, specifier } => {
                        HirTy::adt(specifier.0, vec![], span)
                    }
                    TypeSpecifier::TypedefName((path, _)) => {
                        match path {
                            crate::DefOrLocal::Def(def_id) => HirTy::adt(def_id, vec![], span),
                            crate::DefOrLocal::Local(_) => panic!("invalid parsing"),
                            crate::DefOrLocal::Prim(primitive_ty) => match primitive_ty {
                                PrimitiveTy::IntTy(int_ty) => HirTy::signed_ty(int_ty, span),
                                PrimitiveTy::UintTy(uint_ty) => HirTy::unsigned_ty(uint_ty, span),
                                PrimitiveTy::FloatTy(float_ty) => HirTy::float_ty(float_ty, span),
                            },
                            crate::DefOrLocal::UnrepresentableType(ty) => {
                                return ty;
                            }
                        }
                        // let path = path.to_pretty();
                        // if let Some(prim) = PrimitiveTy::parse(&path) {
                        //     match prim {
                        //         PrimitiveTy::IntTy(int_ty) => HirTy::signed_ty(int_ty, span),
                        //         PrimitiveTy::UintTy(uint_ty) => HirTy::unsigned_ty(uint_ty, span),
                        //         PrimitiveTy::FloatTy(float_ty) => HirTy::float_ty(float_ty, span),
                        //     }
                        // } else if let Some((def_id, _)) = self.resolve(&path) {
                        //     HirTy::adt(def_id, vec![], span)
                        // } else if let Some(sig) = self.unrepresentable_typedefs.get(&path) {
                        //     return TyOrFunction::Function(sig.clone());
                        // } else {
                        //     self.terminate_with_error(parser_span, "Failed to resolve type");
                        // }
                    }
                };
                return CTy::Ty(ty);
            }
        }
        self.terminate_with_error(
            parser_span,
            &format!("no suitable type specifier at span {span:?}"),
        )
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
