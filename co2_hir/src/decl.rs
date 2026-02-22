use std::collections::HashMap;

use co2_parser::{
    Constant, Declaration, DeclarationSpecifier, Declarator, Expression, InitDeclarator,
    Initializer, Spanned,
};
use la_arena::Arena;
use rustc_public_generative::rustc_public::{
    mir::{Mutability, Safety},
    ty::{
        Abi, Binder, FnSig, GenericArgKind, GenericArgs, IntTy, RigidTy, Span as RustSpan, Ty,
        TyKind,
    },
};

use crate::expr::{HirExpr, HirExprKind};
use crate::initializer_tree::{InitializerTree, tree_contains_zeroed};
use crate::item::{HirLocal, LocalId};
use crate::resolver::HirCtx;
use crate::stmt::HirStmt;
use crate::ty::{
    adt_field_tys, array_elem_ty, is_maybe_uninit_fn_ptr_ty, is_sized_array_ty, ty_matches_expected,
};

enum TyOrFunction {
    Ty(Ty),
    Function(FnSig),
}

fn ty_contains_fn_ptr(ty: Ty) -> bool {
    if matches!(ty.kind(), TyKind::RigidTy(RigidTy::FnPtr(_))) || is_maybe_uninit_fn_ptr_ty(ty).is_some() {
        return true;
    }
    if let Some(elem) = array_elem_ty(ty) {
        return ty_contains_fn_ptr(elem);
    }
    if let Some(fields) = adt_field_tys(ty) {
        return fields.into_iter().any(ty_contains_fn_ptr);
    }
    false
}

fn tree_to_full_aggregate_expr(tree: &InitializerTree, ty: Ty, span: RustSpan) -> Option<HirExpr> {
    match tree {
        InitializerTree::Leaf(expr) => Some(expr.clone()),
        InitializerTree::Zeroed => None,
        InitializerTree::Middle { children } => {
            let field_tys = adt_field_tys(ty)?;
            if children.len() != field_tys.len() {
                return None;
            }
            let mut args = Vec::with_capacity(children.len());
            for (child, field_ty) in children.iter().zip(field_tys.into_iter()) {
                let expr = tree_to_full_aggregate_expr(child, field_ty, span)?;
                args.push(expr);
            }
            Some(HirExpr {
                kind: HirExprKind::Aggregate { args },
                ty,
                span,
            })
        }
    }
}

#[derive(Clone, Debug)]
pub struct HirDecl {
    pub local: LocalId,
    pub initializer: Option<HirExpr>,
    pub span: RustSpan,
}

impl<R> HirCtx<'_, R> {
    fn maybe_uninit_of(&self, inner: Ty) -> Result<Ty, String> {
        for path in ["core::mem::MaybeUninit", "std::mem::MaybeUninit"] {
            if let Some(ty) = self.resolve_type(path)
                && let TyKind::RigidTy(RigidTy::Adt(adt, _)) = ty.kind()
            {
                return Ok(Ty::from_rigid_kind(RigidTy::Adt(
                    adt,
                    GenericArgs(vec![GenericArgKind::Type(inner)]),
                )));
            }
        }
        Err("failed to resolve core/std MaybeUninit".to_owned())
    }

    pub(crate) fn lower_decl(
        &self,
        decl: Declaration,
        out: &mut Vec<HirStmt>,
        locals: &mut Arena<HirLocal>,
        local_map: &mut HashMap<String, LocalId>,
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
                    let ((name, span), ty) =
                        self.lower_value_decl_type(declaration_specifiers.clone(), declarator)?;

                    let span = self.to_rust_span(span);

                    let local = locals.alloc(HirLocal {
                        name: name.clone(),
                        ty,
                        span,
                    });
                    local_map.insert(name, local);

                    let needs_tree = raw_initializer
                        .as_ref()
                        .is_some_and(|(init, _)| match init {
                            Initializer::List(_) => true,
                            Initializer::Expr((expr, _)) => {
                                is_sized_array_ty(ty)
                                    || matches!(expr, Expression::Constant(Constant::String(_)))
                            }
                        });

                    let mut tree_initializer = None;
                    let initializer = if let Some(init) = initializer {
                        match init.0 {
                            Initializer::Expr(expr) if !needs_tree => {
                                Some(self.lower_expr(expr, locals, local_map)?)
                            }
                            _ if needs_tree => {
                                let tree = self.lower_to_initializer_tree(
                                    ty,
                                    init.clone(),
                                    locals,
                                    local_map,
                                )?;
                                let needs_base_zero = tree_contains_zeroed(&tree);
                                let can_skip_base_zero = !needs_base_zero && ty_contains_fn_ptr(ty);
                                if can_skip_base_zero {
                                    if let Some(aggregate_init) = tree_to_full_aggregate_expr(&tree, ty, span) {
                                        Some(aggregate_init)
                                    } else {
                                        tree_initializer = Some(tree);
                                        None
                                    }
                                } else {
                                    tree_initializer = Some(tree);
                                    Some(HirExpr {
                                        kind: HirExprKind::Zeroed,
                                        ty,
                                        span,
                                    })
                                }
                            }
                            _ => Some(HirExpr {
                                kind: HirExprKind::Zeroed,
                                ty,
                                span,
                            }),
                        }
                    } else if is_sized_array_ty(ty)
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

                    if let Some(tree) = tree_initializer {
                        let lhs = HirExpr {
                            kind: HirExprKind::Local(local),
                            ty,
                            span,
                        };
                        self.emit_initializer_tree_assignments(lhs, &tree, out)?;
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) fn lower_value_decl_type(
        &self,
        declaration_specifiers: Vec<Spanned<DeclarationSpecifier>>,
        declarator: Spanned<Declarator>,
    ) -> Result<(Spanned<String>, Ty), String> {
        let base = self.base_ty_of_decl(declaration_specifiers, declarator.1)?;
        let (decl_ty, name) = self.extract_decl_type(TyOrFunction::Ty(base), declarator)?;
        let name = name.ok_or_else(|| "missing declaration name".to_owned())?;
        match decl_ty {
            TyOrFunction::Ty(ty) => Ok((name, ty)),
            TyOrFunction::Function(_) => {
                Err("function is not a first-class declaration type in this context".to_owned())
            }
        }
    }

    fn base_ty_of_decl(
        &self,
        specifiers: Vec<Spanned<DeclarationSpecifier>>,
        span: co2_parser::Span,
    ) -> Result<Ty, String> {
        for (specifier, _) in &specifiers {
            if let DeclarationSpecifier::TypeSpecifier((type_specifier, _)) = specifier {
                if let co2_parser::TypeSpecifier::StructOrUnion { specifier, .. } = type_specifier {
                    return match specifier {
                        co2_parser::StructOrUnionSpecifier::Declared { ident } => self
                            .resolve_type(&ident.0)
                            .ok_or_else(|| format!("unresolved struct/union tag: {}", ident.0)),
                        co2_parser::StructOrUnionSpecifier::Defined { .. }
                        | co2_parser::StructOrUnionSpecifier::Anonymous { .. } => {
                            let key = specifier
                                .canonical_field_set_key()
                                .ok_or_else(|| {
                                    "struct/union type is only supported when declared at top level".to_owned()
                                })?;
                            self.resolve_type(&key).ok_or_else(|| {
                                format!(
                                    "anonymous struct type is not predeclared at top level: {key}"
                                )
                            })
                        }
                    };
                }
                if let Some(ty) = crate::ty::type_specifier_to_ty(type_specifier)? {
                    return Ok(ty);
                }
                if let co2_parser::TypeSpecifier::TypedefName(path) = type_specifier {
                    let name = path.0.to_pretty();
                    return self
                        .resolve_type(&name)
                        .ok_or_else(|| format!("unresolved typedef path: {name}"));
                }
            }
        }
        Err(crate::ty::no_type_specifier_err(span))
    }

    fn extract_decl_type(
        &self,
        current: TyOrFunction,
        (decl, span): Spanned<Declarator>,
    ) -> Result<(TyOrFunction, Option<Spanned<String>>), String> {
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
                    let (param_decl_ty, _) = self.extract_decl_type(TyOrFunction::Ty(param_base), param.1)?;
                    let mut param_ty = match param_decl_ty {
                        TyOrFunction::Ty(ty) => ty,
                        TyOrFunction::Function(_) => {
                            return Err("function type is invalid in parameter position".to_owned());
                        }
                    };
                    // Function arguments are always decayed to pointer in C.
                    if let Some(elem) = array_elem_ty(param_ty) {
                        param_ty = Ty::new_ptr(elem, Mutability::Mut);
                    }
                    inputs.push(param_ty);
                }
                let function_ty = match current {
                    TyOrFunction::Ty(ret) => {
                        let mut inputs_and_output = inputs;
                        inputs_and_output.push(ret);
                        TyOrFunction::Function(FnSig {
                            inputs_and_output,
                            c_variadic: param_list.ellipsis,
                            safety: Safety::Safe,
                            abi: Abi::Rust,
                        })
                    }
                    TyOrFunction::Function(_) => {
                        return Err("returning function without ptr is not valid".to_owned());
                    }
                };
                self.extract_decl_type(function_ty, *declarator)
            }
            Declarator::PointerDeclarator { declarator, .. } => {
                let ptr_or_fn_ptr = match current {
                    TyOrFunction::Ty(inner) => TyOrFunction::Ty(Ty::new_ptr(inner, Mutability::Mut)),
                    TyOrFunction::Function(sig) => {
                        let fn_ptr = Ty::from_rigid_kind(RigidTy::FnPtr(Binder::dummy(sig)));
                        TyOrFunction::Ty(self.maybe_uninit_of(fn_ptr)?)
                    }
                };
                self.extract_decl_type(ptr_or_fn_ptr, *declarator)
            }
            Declarator::ArrayDeclarator {
                declarator,
                subscription,
            } => {
                let array_ty = match current {
                    TyOrFunction::Ty(inner) => {
                        if let Some(size) = subscription.0.constant_len() {
                            TyOrFunction::Ty(
                                Ty::try_new_array(inner, size)
                                    .map_err(|e| format!("failed to build array type: {e}"))?,
                            )
                        } else {
                            TyOrFunction::Ty(Ty::new_ptr(inner, Mutability::Mut))
                        }
                    }
                    TyOrFunction::Function(_) => {
                        return Err("array of functions is not supported".to_owned());
                    }
                };
                self.extract_decl_type(array_ty, *declarator)
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
