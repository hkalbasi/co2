use std::collections::HashMap;

use co2_ast::{
    Constant, Declaration, DeclarationSpecifier, Declarator, Expression, InitDeclarator,
    Initializer, Spanned, TypeName,
};
use co2_crate_sig::LocalResolver;
use la_arena::Arena;
use rustc_public_generative::{
    HirTy,
    rustc_public::{
        CrateItem,
        mir::{Mutability, Safety},
        ty::{
            Abi, AdtDef, Binder, FnSig, GenericArgKind, GenericArgs, IntTy, RigidTy,
            Span as RustSpan, Ty, TyConst, TyKind,
        },
    },
};

use crate::{expr::coerce_expr_to_type, item::{HirLocal, LocalId}};
use crate::resolver::HirCtx;
use crate::stmt::HirStmt;
use crate::ty::{array_elem_ty, is_array_ty, is_maybe_uninit_fn_ptr_ty, ty_matches_expected};
use crate::{
    expr::{HirExpr, HirExprKind},
    initializer_tree::InitializerTree,
};

pub enum CTy {
    Ty(Ty),
    Function(FnSig),
    UnsizedArray(Ty),
}

#[derive(Clone, Debug)]
pub struct HirDecl {
    pub local: LocalId,
    pub initializer: Option<HirExpr>,
    pub span: RustSpan,
}

impl HirCtx<'_> {
    fn maybe_uninit_of(&self, inner: Ty) -> Result<Ty, String> {
        return Ok(Ty::from_rigid_kind(RigidTy::Adt(
            AdtDef(self.maybe_uninit_def),
            GenericArgs(vec![GenericArgKind::Type(inner)]),
        )));
    }

    pub(crate) fn lower_decl(
        &self,
        decl: Declaration<LocalResolver>,
        out: &mut Vec<HirStmt>,
        locals: &mut Arena<HirLocal>,
        local_map: &mut HashMap<usize, LocalId>,
    ) -> Result<(), String> {
        match decl {
            Declaration::FunctionDefinition { .. } => {
                return Err("nested function declaration is not supported".to_owned());
            }
            Declaration::Declaration {
                declaration_specifiers,
                declarators,
            } => {
                for init in declarators {
                    let InitDeclarator {
                        declarator,
                        initializer,
                    } = init.0;
                    let raw_initializer = initializer.clone();
                    let ((name, parser_span), ty) =
                        self.lower_value_decl_type(declaration_specifiers.clone(), declarator);
                    let ty = match ty {
                        CTy::Ty(ty) => ty,
                        CTy::Function(_) => continue,
                        CTy::UnsizedArray(elem) => {
                            let Some(initializer) = initializer else {
                                self.terminate_with_error(
                                    parser_span,
                                    "Unsized array without initializer is invalid",
                                );
                            };
                            let fake_ty = Ty::from_rigid_kind(RigidTy::Array(
                                elem,
                                TyConst::try_from_target_usize(567_567).unwrap(),
                            ));
                            let tree = self.lower_to_initializer_tree(
                                fake_ty,
                                initializer,
                                locals,
                                local_map,
                            );
                            let real_len = if let InitializerTree::Middle { children } = &tree {
                                children.len() as u64
                            } else {
                                self.terminate_with_error(
                                    parser_span,
                                    "invalid initializer for unsized array",
                                );
                            };
                            let real_ty = Ty::from_rigid_kind(RigidTy::Array(
                                elem,
                                TyConst::try_from_target_usize(real_len).unwrap(),
                            ));
                            let tree_expr =
                                self.initializer_tree_to_expr(&tree, real_ty, parser_span);

                            let span = self.to_rust_span(parser_span);
                            let local = locals.alloc(HirLocal {
                                name: name.1.clone(),
                                ty: real_ty,
                                span,
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
                                let expr = self.lower_expr(expr, locals, local_map)?;
                                let expr = coerce_expr_to_type(expr, ty)?;
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
                            _ => Some(HirExpr {
                                kind: HirExprKind::Zeroed,
                                ty,
                                span,
                            }),
                        }
                    } else if is_array_ty(ty)
                        || matches!(ty.kind(), TyKind::RigidTy(RigidTy::Adt(_, _)))
                    {
                        Some(HirExpr {
                            kind: HirExprKind::Zeroed,
                            ty,
                            span,
                        })
                    } else if matches!(ty.kind(), TyKind::RigidTy(RigidTy::RawPtr(_, _)))
                        || is_maybe_uninit_fn_ptr_ty(ty).is_some()
                    {
                        Some(HirExpr {
                            kind: HirExprKind::Cast(Box::new(HirExpr {
                                kind: HirExprKind::ConstInt(0),
                                ty: Ty::signed_ty(IntTy::I32),
                                span,
                            })),
                            ty,
                            span,
                        })
                    } else {
                        None
                    };

                    out.push(HirStmt::Decl(HirDecl {
                        local,
                        initializer,
                        span,
                    }));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn try_lower_value_decl_type(
        &self,
        declaration_specifiers: Vec<Spanned<DeclarationSpecifier<LocalResolver>>>,
        declarator: Spanned<Declarator<LocalResolver>>,
    ) -> Result<(Spanned<(usize, String)>, CTy), String> {
        let base = self.base_ty_of_decl(declaration_specifiers, declarator.1)?;
        let (decl_ty, name) = self.extract_decl_type(base, declarator)?;
        let name = name.ok_or_else(|| "missing declaration name".to_owned())?;
        Ok((name, decl_ty))
    }

    pub(crate) fn lower_value_decl_type(
        &self,
        declaration_specifiers: Vec<Spanned<DeclarationSpecifier<LocalResolver>>>,
        declarator: Spanned<Declarator<LocalResolver>>,
    ) -> (Spanned<(usize, String)>, CTy) {
        let span = declarator.1;
        match self.try_lower_value_decl_type(declaration_specifiers, declarator) {
            Ok(x) => x,
            Err(e) => self.terminate_with_error(span, &e),
        }
    }

    pub(crate) fn lower_type_name(
        &self,
        type_name: TypeName<LocalResolver>,
        span: co2_ast::Span,
    ) -> Result<Ty, String> {
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
        let base = self.base_ty_of_decl(specifiers, span)?;
        let ty = match type_name.abstract_declarator {
            None => base,
            Some(decl) => {
                let (ty, name) = self.extract_decl_type(base, decl)?;
                if let Some((_, span)) = name {
                    self.terminate_with_error(span, "type name should not have name");
                }
                ty
            }
        };
        match ty {
            CTy::Ty(ty) => Ok(ty),
            CTy::Function(_) => {
                self.terminate_with_error(span, "Function is invalid as a type name");
            }
            CTy::UnsizedArray(_) => {
                self.terminate_with_error(span, "Unsized array is invalid as a type name");
            }
        }
    }

    fn base_ty_of_decl(
        &self,
        specifiers: Vec<Spanned<DeclarationSpecifier<LocalResolver>>>,
        span: co2_ast::Span,
    ) -> Result<CTy, String> {
        for (specifier, _) in &specifiers {
            if let DeclarationSpecifier::TypeSpecifier((type_specifier, _)) = specifier {
                if let co2_ast::TypeSpecifier::StructOrUnion { kind: _, specifier } = type_specifier
                {
                    return Ok(CTy::Ty(Ty::from_rigid_kind(RigidTy::Adt(
                        AdtDef(specifier.0),
                        GenericArgs(vec![]),
                    ))));
                }
                if let Some(ty) = crate::ty::type_specifier_to_ty(type_specifier)? {
                    return Ok(CTy::Ty(ty));
                }
                if let co2_ast::TypeSpecifier::TypedefName(path) = type_specifier {
                    return Ok(match &path.0 {
                        co2_crate_sig::DefOrLocal::Def(def_id) => CTy::Ty(CrateItem(*def_id).ty()),
                        co2_crate_sig::DefOrLocal::Local(_) => {
                            panic!("Invalid local in type position")
                        }
                        co2_crate_sig::DefOrLocal::Prim(primitive_ty) => {
                            CTy::Ty(match *primitive_ty {
                                co2_crate_sig::PrimitiveTy::IntTy(int_ty) => {
                                    Ty::from_rigid_kind(RigidTy::Int(int_ty))
                                }
                                co2_crate_sig::PrimitiveTy::UintTy(uint_ty) => {
                                    Ty::from_rigid_kind(RigidTy::Uint(uint_ty))
                                }
                                co2_crate_sig::PrimitiveTy::FloatTy(float_ty) => {
                                    Ty::from_rigid_kind(RigidTy::Float(float_ty))
                                }
                            })
                        }
                        co2_crate_sig::DefOrLocal::UnrepresentableType(sig_ty) => {
                            self.sig_cty_to_cty(sig_ty)
                        }
                    });
                }
            }
        }
        Err(crate::ty::no_type_specifier_err(span))
    }

    fn extract_decl_type(
        &self,
        current: CTy,
        (decl, span): Spanned<Declarator<LocalResolver>>,
    ) -> Result<(CTy, Option<Spanned<(usize, String)>>), String> {
        match decl {
            Declarator::Abstract => Ok((current, None)),
            Declarator::Identifier(ident) => Ok((current, Some(ident))),
            Declarator::FunctionDeclarator {
                declarator,
                param_list,
            } => {
                let mut inputs = Vec::with_capacity(param_list.parameters.len());
                for param in param_list.parameters {
                    let param_base = self.base_ty_of_decl(param.0, span)?;
                    let (param_decl_ty, _) = self.extract_decl_type(param_base, param.1)?;
                    let mut param_ty = match param_decl_ty {
                        CTy::Ty(ty) => ty,
                        CTy::Function(_) => {
                            return Err("function type is invalid in parameter position".to_owned());
                        }
                        CTy::UnsizedArray(elem) => Ty::new_ptr(elem, Mutability::Mut),
                    };
                    // Function arguments are always decayed to pointer in C.
                    if let Some(elem) = array_elem_ty(param_ty) {
                        param_ty = Ty::new_ptr(elem, Mutability::Mut);
                    }
                    inputs.push(param_ty);
                }
                let function_ty = match current {
                    CTy::Ty(ret) => {
                        let mut inputs_and_output = inputs;
                        inputs_and_output.push(ret);
                        CTy::Function(FnSig {
                            inputs_and_output,
                            c_variadic: param_list.ellipsis,
                            safety: Safety::Safe,
                            abi: Abi::C { unwind: false },
                        })
                    }
                    CTy::Function(_) => {
                        return Err("returning function without ptr is not valid".to_owned());
                    }
                    CTy::UnsizedArray(_) => {
                        return Err("returning unsized array is not valid".to_owned());
                    }
                };
                self.extract_decl_type(function_ty, *declarator)
            }
            Declarator::PointerDeclarator { declarator, .. } => {
                let ptr_or_fn_ptr = match current {
                    CTy::Ty(inner) => CTy::Ty(Ty::new_ptr(inner, Mutability::Mut)),
                    CTy::Function(sig) => {
                        let fn_ptr = Ty::from_rigid_kind(RigidTy::FnPtr(Binder::dummy(sig)));
                        CTy::Ty(self.maybe_uninit_of(fn_ptr)?)
                    }
                    CTy::UnsizedArray(elem) => CTy::Ty(Ty::new_ptr(
                        Ty::new_ptr(elem, Mutability::Mut),
                        Mutability::Mut,
                    )),
                };
                self.extract_decl_type(ptr_or_fn_ptr, *declarator)
            }
            Declarator::ArrayDeclarator {
                declarator,
                subscription,
            } => {
                let array_ty = match current {
                    CTy::Ty(inner) => {
                        if let Some(size) = subscription.0.constant_len() {
                            CTy::Ty(
                                Ty::try_new_array(inner, size)
                                    .map_err(|e| format!("failed to build array type: {e}"))?,
                            )
                        } else if subscription.0.is_unsized() {
                            CTy::UnsizedArray(inner)
                        } else {
                            return Err("Can not calculate subscription".to_owned());
                        }
                    }
                    CTy::Function(_) => {
                        return Err("array of functions is invalid".to_owned());
                    }
                    CTy::UnsizedArray(_) => {
                        return Err("array of unsized arrays is invalid".to_owned());
                    }
                };
                self.extract_decl_type(array_ty, *declarator)
            }
        }
    }

    fn hir_ty_to_ty(&self, hir_ty: &HirTy) -> Ty {
        match &hir_ty.kind {
            rustc_public_generative::HirTyKind::Bool => Ty::bool_ty(),
            rustc_public_generative::HirTyKind::Char => todo!(),
            &rustc_public_generative::HirTyKind::Int(int_ty) => Ty::signed_ty(int_ty),
            &rustc_public_generative::HirTyKind::Uint(uint_ty) => Ty::unsigned_ty(uint_ty),
            &rustc_public_generative::HirTyKind::Float(float_ty) => {
                Ty::from_rigid_kind(RigidTy::Float(float_ty))
            }
            _ => todo!(),
        }
    }

    fn sig_cty_to_cty(&self, sig_ty: &co2_crate_sig::CTy) -> CTy {
        match sig_ty {
            co2_crate_sig::CTy::Ty(hir_ty) => CTy::Ty(self.hir_ty_to_ty(hir_ty)),
            co2_crate_sig::CTy::Function(_) => todo!(),
            co2_crate_sig::CTy::UnsizedArray(hir_ty) => {
                CTy::UnsizedArray(self.hir_ty_to_ty(hir_ty))
            }
        }
    }
}

pub(crate) fn call_arg_type_compatible(expected: Ty, actual: Ty) -> bool {
    if expected == actual {
        return true;
    }
    ty_matches_expected(expected, actual)
}
