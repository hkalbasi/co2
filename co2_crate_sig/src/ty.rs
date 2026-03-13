use co2_ast::{DeclarationSpecifier, Declarator, Span, Spanned, TypeSpecifier};
use rustc_public_generative::{FunctionAbi, FunctionSignature, HirTy, HirTyKind};
use rustc_public_generative::{
    HirGenericArg,
    rustc_public::{
        mir::Mutability,
        ty::{FloatTy, IntTy, UintTy},
    },
};

use crate::{CrateSigCtx, LocalResolver, LocalResolverBase};

#[derive(Debug, Clone)]
pub(crate) enum TyOrFunction {
    Ty(HirTy),
    Function(FunctionSignature),
}

impl CrateSigCtx<'_> {
    pub(crate) fn base_ty_of_decl(
        &mut self,
        specifiers: Vec<Spanned<DeclarationSpecifier<LocalResolver>>>,
        parser_span: Span,
    ) -> TyOrFunction {
        self.resolver
            .borrow_mut()
            .base_ty_of_decl(specifiers, parser_span)
    }

    pub(crate) fn lower_function_signature(
        &mut self,
        base: TyOrFunction,
        declarator: Spanned<Declarator<LocalResolver>>,
    ) -> Result<(String, FunctionSignature, Vec<String>), String> {
        self.resolver
            .borrow_mut()
            .lower_function_signature(base, declarator)
    }

    pub(crate) fn try_lower_value_decl_type(
        &mut self,
        base: TyOrFunction,
        declarator: Spanned<Declarator<LocalResolver>>,
    ) -> Result<(String, HirTy), String> {
        self.resolver
            .borrow_mut()
            .try_lower_value_decl_type(base, declarator)
    }

    pub(crate) fn lower_value_decl_type(
        &mut self,
        base: TyOrFunction,
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
        base: TyOrFunction,
        declarator: Spanned<Declarator<LocalResolver>>,
    ) -> Result<(String, FunctionSignature, Vec<String>), String> {
        let parsed_param_names = function_param_names(&declarator.0);
        let (decl_ty, name) = self.extract_decl_type(base, declarator)?;
        let name = name.ok_or_else(|| "missing function name".to_owned())?;
        let TyOrFunction::Function(sig) = decl_ty else {
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
        base: TyOrFunction,
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
        base: TyOrFunction,
        declarator: Spanned<Declarator<LocalResolver>>,
    ) -> Result<(String, HirTy), String> {
        let (decl_ty, name) = self.extract_decl_type(base, declarator)?;
        let name = name.ok_or_else(|| "missing declaration name".to_owned())?;
        match decl_ty {
            TyOrFunction::Ty(ty) => Ok((name, ty)),
            TyOrFunction::Function(_) => {
                Err("function is not a first-class declaration type in this context".to_owned())
            }
        }
    }

    pub(crate) fn base_ty_of_decl(
        &mut self,
        specifiers: Vec<Spanned<DeclarationSpecifier<LocalResolver>>>,
        parser_span: Span,
    ) -> TyOrFunction {
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
                            crate::DefOrLocal::UnrepresentableType(sig) => {
                                return TyOrFunction::Function(sig);
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
                return TyOrFunction::Ty(ty);
            }
        }
        self.terminate_with_error(
            parser_span,
            &format!("no suitable type specifier at span {span:?}"),
        )
    }

    fn extract_decl_type(
        &mut self,
        current: TyOrFunction,
        (decl, span): Spanned<Declarator<LocalResolver>>,
    ) -> Result<(TyOrFunction, Option<String>), String> {
        let rust_span = self.co2_span_to_rustc(span);
        match decl {
            Declarator::Abstract => Ok((current, None)),
            Declarator::Identifier((ident, _)) => Ok((current, Some(ident.1))),
            Declarator::FunctionDeclarator {
                declarator,
                param_list,
            } => {
                let mut inputs = Vec::with_capacity(param_list.parameters.len());
                if !(param_list.parameters.len() == 1
                    && parameter_is_void(&param_list.parameters[0]))
                {
                    for param in param_list.parameters {
                        let param_base = self.base_ty_of_decl(param.0, span);
                        let (param_decl_ty, _) = self.extract_decl_type(param_base, param.1)?;
                        let param_ty = match param_decl_ty {
                            TyOrFunction::Ty(ty) => ty,
                            TyOrFunction::Function(_) => {
                                return Err(
                                    "function type is invalid in parameter position".to_owned()
                                );
                            }
                        };
                        inputs.push(param_ty);
                    }
                }
                let function_ty = match current {
                    TyOrFunction::Ty(ret) => TyOrFunction::Function(FunctionSignature {
                        lifetimes: vec![],
                        inputs,
                        output: ret,
                        abi: FunctionAbi::C,
                        is_unsafe: false,
                        c_variadic: param_list.ellipsis,
                    }),
                    TyOrFunction::Function(_) => {
                        return Err("function returning function is not supported".to_owned());
                    }
                };
                self.extract_decl_type(function_ty, *declarator)
            }
            Declarator::PointerDeclarator { declarator, .. } => {
                let ptr_or_fn_ptr = match current {
                    TyOrFunction::Ty(inner) => {
                        TyOrFunction::Ty(HirTy::new_ptr(inner, Mutability::Mut, rust_span))
                    }
                    TyOrFunction::Function(sig) => TyOrFunction::Ty(self.maybe_uninit_of(
                        HirTy {
                            kind: HirTyKind::FnPtr(Box::new(sig)),
                            span: rust_span,
                        },
                        rust_span,
                    )?),
                };
                self.extract_decl_type(ptr_or_fn_ptr, *declarator)
            }
            Declarator::ArrayDeclarator { declarator, .. } => {
                let array_ty = match current {
                    TyOrFunction::Ty(inner) => {
                        TyOrFunction::Ty(HirTy::new_ptr(inner, Mutability::Mut, rust_span))
                    }
                    TyOrFunction::Function(_) => {
                        return Err("array of functions is not supported".to_owned());
                    }
                };
                self.extract_decl_type(array_ty, *declarator)
            }
        }
    }

    fn maybe_uninit_of(
        &self,
        inner: HirTy,
        span: rustc_public_generative::rustc_public::ty::Span,
    ) -> Result<HirTy, String> {
        if let Some((def, _)) = self.resolver.resolve("core::mem::MaybeUninit") {
            return Ok(HirTy::adt(def, vec![HirGenericArg::Ty(inner)], span));
        }
        Err("failed to resolve core/std MaybeUninit".to_owned())
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
        Declarator::FunctionDeclarator { param_list, .. } => Some(
            param_list
                .parameters
                .iter()
                .map(|param| Some(param.1.0.ident()?.1))
                .collect(),
        ),
        Declarator::PointerDeclarator { declarator, .. }
        | Declarator::ArrayDeclarator { declarator, .. } => function_param_names(&declarator.0),
        _ => None,
    }
}

fn parameter_is_void(
    param: &(
        Vec<Spanned<DeclarationSpecifier<LocalResolver>>>,
        Spanned<Declarator<LocalResolver>>,
    ),
) -> bool {
    let declarator_is_abstract = matches!(param.1.0, Declarator::Abstract);
    let has_void_type = param.0.iter().any(|(spec, _)| {
        matches!(
            spec,
            DeclarationSpecifier::TypeSpecifier((TypeSpecifier::Void, _))
        )
    });
    declarator_is_abstract && has_void_type
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
