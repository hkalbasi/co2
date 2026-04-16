use std::collections::HashMap;

use co2_ast::{
    Constant, Declaration, DeclarationSpecifier, Declarator, Expression, InitDeclarator,
    Initializer, Spanned, TypeName, TypeQualifier,
};
use co2_crate_sig::{CompressedTypeSpecifier, LocalResolver, eval_registered_array_len_const};
use la_arena::Arena;
use rustc_public_generative::{
    FunctionAbi, FunctionSignature, HirGenericArg, HirLifetime, HirTy, HirTyConst,
    rustc_public::{
        CrateItem,
        mir::{Mutability, Safety},
        ty::{
            Abi, AdtDef, Binder, FnSig, GenericArgKind, GenericArgs, IntTy, RegionKind, RigidTy,
            Span as RustSpan, Ty, TyConst, TyKind,
        },
    },
};

use crate::resolver::HirCtx;
use crate::stmt::HirStmt;
use crate::ty::{array_elem_ty, is_array_ty};
use crate::{
    expr::coerce_expr_to_type,
    item::{HirLocal, LocalId},
};
use crate::{
    expr::{HirExpr, HirExprKind},
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
    pub(crate) fn maybe_uninit_of(&self, inner: Ty) -> Ty {
        return Ty::from_rigid_kind(RigidTy::Adt(
            self.wellknown_defs.maybe_uninit,
            GenericArgs(vec![GenericArgKind::Type(inner)]),
        ));
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
                            let real_len = crate::infer_array_len_from_initializer(
                                initializer.clone(),
                                elem,
                                self,
                            )
                            .unwrap_or_else(|err| self.terminate_with_error(parser_span, &err));
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
                            let tree_expr = self.initializer_tree_to_expr(&tree, real_ty, parser_span);

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
                    if let Some(resolver) = &self.decl_resolver {
                        let hir_ty = self.ty_to_hir_ty(ty, span);
                        resolver.set_local_ty(name.0 as u32, hir_ty);
                    }

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
                                let parser_span = expr.1;
                                let mut expr = self.lower_expr(expr, locals, local_map)?;
                                if is_array_ty(expr.ty) && !is_array_ty(ty) {
                                    let elem = array_elem_ty(expr.ty)
                                        .expect("array type should have element type");
                                    expr = HirExpr {
                                        kind: HirExprKind::ArrayToPointer(Box::new(expr.clone())),
                                        ty: Ty::new_ptr(elem, Mutability::Mut),
                                        span: expr.span,
                                    };
                                }
                                let expr = match coerce_expr_to_type(expr, ty) {
                                    Ok(it) => it,
                                    Err(err) => self.terminate_with_error(parser_span, &err),
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
                            _ => Some(HirExpr {
                                kind: HirExprKind::Zeroed,
                                ty,
                                span,
                            }),
                        }
                    } else {
                        Some(HirExpr {
                            kind: HirExprKind::Zeroed,
                            ty,
                            span,
                        })
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
        let base = self.base_ty_of_decl(declaration_specifiers, declarator.1);
        let base_const = has_const_qualifier_in_decl_specs(&base.1);
        let (decl_ty, name) = self.extract_decl_type(base.0, base_const, declarator)?;
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
        let base = self.base_ty_of_decl(specifiers, span);
        let ty = match type_name.abstract_declarator {
            None => base.0,
            Some(decl) => {
                let (ty, name) = self.extract_decl_type(
                    base.0,
                    has_const_qualifier_in_decl_specs(&base.1),
                    decl,
                )?;
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
    ) -> (CTy, Vec<Spanned<DeclarationSpecifier<LocalResolver>>>) {
        let specifier = match CompressedTypeSpecifier::build(specifiers.clone()) {
            Ok(s) => s,
            Err(e) => self.terminate_with_error(span, &e),
        };

        let ty = match specifier {
            CompressedTypeSpecifier::Void => Ty::new_tuple(&[]),
            CompressedTypeSpecifier::PrimitiveTy(primitive_ty) => prim_ty_to_ty(primitive_ty),
            CompressedTypeSpecifier::StructOrUnion { kind: _, specifier } => {
                Ty::from_rigid_kind(RigidTy::Adt(AdtDef(specifier.0), GenericArgs(vec![])))
            }
            CompressedTypeSpecifier::Enum(_) => Ty::signed_ty(IntTy::I32),
            CompressedTypeSpecifier::TypedefName(path) => {
                return match &path.0 {
                    co2_crate_sig::DefOrLocal::Def(def_id) => {
                        (CTy::Ty(CrateItem(*def_id).ty()), specifiers)
                    }
                    co2_crate_sig::DefOrLocal::Const(_) => {
                        panic!("Invalid const in type position")
                    }
                    co2_crate_sig::DefOrLocal::AssocMethod { .. } => {
                        panic!("Invalid associated method in type position")
                    }
                    co2_crate_sig::DefOrLocal::Local(_) => {
                        panic!("Invalid local in type position")
                    }
                    co2_crate_sig::DefOrLocal::FuncName => {
                        panic!("Invalid __func__ in type position")
                    }
                    co2_crate_sig::DefOrLocal::Prim(primitive_ty) => {
                        (CTy::Ty(prim_ty_to_ty(*primitive_ty)), specifiers)
                    }
                    co2_crate_sig::DefOrLocal::UnrepresentableType(sig_ty) => {
                        (self.sig_cty_to_cty(sig_ty), specifiers)
                    }
                };
            }
        };

        (CTy::Ty(ty), specifiers)
    }

    fn extract_decl_type(
        &self,
        current: CTy,
        current_const: bool,
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
                let c_variadic = param_list.effective_ellipsis();
                if !param_list.empty_params() {
                    for param in param_list.parameters {
                        let param_base = self.base_ty_of_decl(param.0, span);
                        let param_base_const = has_const_qualifier_in_decl_specs(&param_base.1);
                        let (param_decl_ty, _) =
                            self.extract_decl_type(param_base.0, param_base_const, param.1)?;
                        let mut param_ty = match param_decl_ty {
                            CTy::Ty(ty) => ty,
                            CTy::Function(_) => {
                                return Err(
                                    "function type is invalid in parameter position".to_owned()
                                );
                            }
                            CTy::UnsizedArray(elem) => Ty::new_ptr(elem, Mutability::Mut),
                        };
                        // Function arguments are always decayed to pointer in C.
                        if let Some(elem) = array_elem_ty(param_ty) {
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
                        return Err("returning function without ptr is not valid".to_owned());
                    }
                    CTy::UnsizedArray(_) => {
                        return Err("returning unsized array is not valid".to_owned());
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
                self.extract_decl_type(ptr_or_fn_ptr, next_const, *declarator)
            }
            Declarator::ArrayDeclarator {
                declarator,
                subscription,
            } => {
                let array_ty = match current {
                    CTy::Ty(inner) => {
                        if let Some(size) = subscription.0.raw.constant_len() {
                            CTy::Ty(
                                Ty::try_new_array(inner, size as u64)
                                    .map_err(|e| format!("failed to build array type: {e}"))?,
                            )
                        } else if subscription.0.raw.is_unsized() {
                            CTy::UnsizedArray(inner)
                        } else {
                            let len = self
                                .decl_resolver
                                .as_ref()
                                .and_then(|resolver| {
                                    subscription
                                        .0
                                        .array_len_const
                                        .as_ref()
                                        .copied()
                                        .map(|def_id| (resolver, def_id))
                                })
                                .ok_or_else(|| "Can not calculate subscription".to_owned())
                                .and_then(|(resolver, def_id)| {
                                    eval_registered_array_len_const(resolver, def_id)
                                        .map_err(|err| format!("Can not calculate subscription: {err}"))
                                })?;
                            CTy::Ty(
                                Ty::try_new_array(inner, len as u64)
                                    .map_err(|e| format!("failed to build array type: {e}"))?,
                            )
                        }
                    }
                    CTy::Function(_) => {
                        return Err("array of functions is invalid".to_owned());
                    }
                    CTy::UnsizedArray(_) => {
                        return Err("array of unsized arrays is invalid".to_owned());
                    }
                };
                self.extract_decl_type(array_ty, current_const, *declarator)
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

    fn ty_to_hir_ty(&self, ty: Ty, span: RustSpan) -> HirTy {
        match ty.kind() {
            TyKind::RigidTy(RigidTy::Bool) => HirTy {
                kind: rustc_public_generative::HirTyKind::Bool,
                span,
            },
            TyKind::RigidTy(RigidTy::Char) => HirTy {
                kind: rustc_public_generative::HirTyKind::Char,
                span,
            },
            TyKind::RigidTy(RigidTy::Int(int_ty)) => HirTy::signed_ty(int_ty, span),
            TyKind::RigidTy(RigidTy::Uint(uint_ty)) => HirTy::unsigned_ty(uint_ty, span),
            TyKind::RigidTy(RigidTy::Float(float_ty)) => HirTy::float_ty(float_ty, span),
            TyKind::RigidTy(RigidTy::Tuple(items)) => HirTy::new_tuple(
                items.iter().map(|ty| self.ty_to_hir_ty(*ty, span)).collect(),
                span,
            ),
            TyKind::RigidTy(RigidTy::RawPtr(inner, mutability)) => {
                HirTy::new_ptr(self.ty_to_hir_ty(inner, span), mutability, span)
            }
            TyKind::RigidTy(RigidTy::Array(inner, len)) => {
                let len = len
                    .eval_target_usize()
                    .expect("local array type should have concrete length")
                    as usize;
                HirTy::new_array(
                    self.ty_to_hir_ty(inner, span),
                    HirTyConst::Literal(len),
                    span,
                )
            }
            TyKind::RigidTy(RigidTy::Adt(def, args)) => HirTy::adt(
                def.0,
                args.0
                    .iter()
                    .map(|arg| match arg {
                        GenericArgKind::Type(ty) => {
                            HirGenericArg::Ty(self.ty_to_hir_ty(*ty, span))
                        }
                        GenericArgKind::Lifetime(_) => HirGenericArg::Lifetime(HirLifetime::Static),
                        _ => panic!("unsupported generic arg in local C type"),
                    })
                    .collect(),
                span,
            ),
            TyKind::RigidTy(RigidTy::FnPtr(sig)) => {
                let sig = sig.skip_binder();
                let abi = match sig.abi {
                    Abi::C { .. } => FunctionAbi::C,
                    Abi::Rust => FunctionAbi::Rust,
                    _ => panic!("unsupported fn ptr abi in local C type: {:?}", sig.abi),
                };
                HirTy {
                    kind: rustc_public_generative::HirTyKind::FnPtr(Box::new(
                        FunctionSignature {
                            lifetimes: vec![],
                            inputs: sig
                                .inputs()
                                .iter()
                                .map(|ty| self.ty_to_hir_ty(*ty, span))
                                .collect(),
                            output: self.ty_to_hir_ty(sig.output(), span),
                            abi,
                            is_unsafe: matches!(sig.safety, Safety::Unsafe),
                            c_variadic: sig.c_variadic,
                        },
                    )),
                    span,
                }
            }
            TyKind::RigidTy(RigidTy::Ref(region, inner, mutability)) => HirTy::new_ref(
                self.ty_to_hir_ty(inner, span),
                mutability,
                match region.kind {
                    RegionKind::ReStatic => HirLifetime::Static,
                    _ => panic!("unsupported region in local C ref type: {:?}", region.kind),
                },
                span,
            ),
            other => panic!("unsupported local C type for array-size evaluation: {other:?}"),
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
        co2_crate_sig::PrimitiveTy::IntTy(int_ty) => Ty::from_rigid_kind(RigidTy::Int(int_ty)),
        co2_crate_sig::PrimitiveTy::UintTy(uint_ty) => Ty::from_rigid_kind(RigidTy::Uint(uint_ty)),
        co2_crate_sig::PrimitiveTy::FloatTy(float_ty) => {
            Ty::from_rigid_kind(RigidTy::Float(float_ty))
        }
    }
}
