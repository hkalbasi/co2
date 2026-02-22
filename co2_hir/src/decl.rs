use std::collections::HashMap;

use co2_parser::{Declaration, DeclarationSpecifier, Declarator, Expression, InitDeclarator, Spanned};
use la_arena::Arena;
use rustc_public_generative::rustc_public::{
    mir::{Mutability, Safety},
    ty::{Abi, Binder, FnSig, RigidTy, Span as RustSpan, Ty},
};

use crate::expr::{HirExpr, HirExprKind};
use crate::item::{HirLocal, LocalId};
use crate::resolver::HirCtx;
use crate::stmt::HirStmt;
use crate::ty::{adt_field_tys, is_integer_ty, ty_matches_expected};

#[derive(Clone, Debug)]
pub struct HirDecl {
    pub local: LocalId,
    pub initializer: Option<HirExpr>,
    pub span: RustSpan,
}

impl<R> HirCtx<'_, R> {
    pub(crate) fn lower_decl(
        &self,
        decl: Declaration,
        parser_span: co2_parser::Span,
        out: &mut Vec<HirStmt>,
        locals: &mut Arena<HirLocal>,
        local_map: &mut HashMap<String, LocalId>,
    ) -> Result<(), String> {
        let span = self.to_rust_span(parser_span);
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
                    let (name, ty) =
                        self.lower_value_decl_type(declaration_specifiers.clone(), declarator)?;

                    let local = locals.alloc(HirLocal {
                        name: name.clone(),
                        ty,
                    });
                    local_map.insert(name, local);

                    let initializer = if let Some(init_expr) = initializer {
                        match init_expr.0 {
                            Expression::InitList(items) => {
                                let field_tys = adt_field_tys(ty).ok_or_else(|| {
                                    format!("initializer list requires ADT type: {ty:?}")
                                })?;
                                if field_tys.len() != items.len() {
                                    return Err(format!(
                                        "initializer field count mismatch: expected {}, got {}",
                                        field_tys.len(),
                                        items.len()
                                    ));
                                }
                                let lowered_items = items
                                    .into_iter()
                                    .map(|item| self.lower_expr(item, locals, local_map))
                                    .collect::<Result<Vec<_>, _>>()?;
                                let mut args = Vec::with_capacity(lowered_items.len());
                                let mut positional_ok = true;
                                for (idx, lowered) in lowered_items.iter().enumerate() {
                                    let expected_ty = field_tys[idx];
                                    match coerce_expr_to_type(lowered.clone(), expected_ty) {
                                        Ok(coerced) => args.push(coerced),
                                        Err(_) => {
                                            positional_ok = false;
                                            break;
                                        }
                                    }
                                }

                                if !positional_ok {
                                    args.clear();
                                    let mut assigned = vec![false; field_tys.len()];
                                    let mut reordered: Vec<Option<HirExpr>> = vec![None; field_tys.len()];
                                    for lowered in lowered_items {
                                        let mut placed = false;
                                        for (idx, expected_ty) in field_tys.iter().enumerate() {
                                            if assigned[idx] {
                                                continue;
                                            }
                                            if let Ok(coerced) =
                                                coerce_expr_to_type(lowered.clone(), *expected_ty)
                                            {
                                                reordered[idx] = Some(coerced);
                                                assigned[idx] = true;
                                                placed = true;
                                                break;
                                            }
                                        }
                                        if !placed {
                                            return Err(format!(
                                                "initializer type mismatch for ADT {ty:?}: no compatible field for {:?}",
                                                lowered.ty
                                            ));
                                        }
                                    }
                                    for item in reordered {
                                        args.push(item.expect("missing reordered initializer"));
                                    }
                                }
                                Some(HirExpr {
                                    kind: HirExprKind::Aggregate { args },
                                    ty,
                                    span,
                                })
                            }
                            other => Some(self.lower_expr((other, init_expr.1), locals, local_map)?),
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
            }
        }
        Ok(())
    }

    pub(crate) fn lower_value_decl_type(
        &self,
        declaration_specifiers: Vec<Spanned<DeclarationSpecifier>>,
        declarator: Spanned<Declarator>,
    ) -> Result<(String, Ty), String> {
        let base = self.base_ty_of_decl(declaration_specifiers, declarator.1)?;
        let (decl_ty, name) = self.extract_decl_type(base, declarator)?;
        let name = name.ok_or_else(|| "missing declaration name".to_owned())?;
        Ok((name, decl_ty))
    }

    fn base_ty_of_decl(
        &self,
        specifiers: Vec<Spanned<DeclarationSpecifier>>,
        span: co2_parser::Span,
    ) -> Result<Ty, String> {
        for (specifier, _) in &specifiers {
            if let DeclarationSpecifier::TypeSpecifier((type_specifier, _)) = specifier {
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
        base: Ty,
        (decl, span): Spanned<Declarator>,
    ) -> Result<(Ty, Option<String>), String> {
        match decl {
            Declarator::Abstract => Ok((base, None)),
            Declarator::Identifier((ident, _)) => Ok((base, Some(ident))),
            Declarator::FunctionDeclarator {
                declarator,
                param_list,
            } => {
                let mut inputs = Vec::with_capacity(param_list.parameters.len());
                for param in param_list.parameters {
                    let param_base = self.base_ty_of_decl(param.0, span)?;
                    let (param_ty, _) = self.extract_decl_type(param_base, param.1)?;
                    inputs.push(param_ty);
                }

                let (ret, name) = self.extract_decl_type(base, *declarator)?;
                let mut inputs_and_output = inputs;
                inputs_and_output.push(ret);
                let sig = FnSig {
                    inputs_and_output,
                    c_variadic: false,
                    safety: Safety::Safe,
                    abi: Abi::Rust,
                };
                Ok((Ty::from_rigid_kind(RigidTy::FnPtr(Binder::dummy(sig))), name))
            }
            Declarator::PointerDeclarator { declarator, .. } => {
                let (inner, name) = self.extract_decl_type(base, *declarator)?;
                Ok((Ty::new_ptr(inner, Mutability::Mut), name))
            }
            Declarator::ArrayDeclarator { declarator, .. } => {
                let (inner, name) = self.extract_decl_type(base, *declarator)?;
                Ok((Ty::new_ptr(inner, Mutability::Mut), name))
            }
        }
    }
}

fn coerce_expr_to_type(expr: HirExpr, expected_ty: Ty) -> Result<HirExpr, String> {
    if expr.ty == expected_ty {
        return Ok(expr);
    }

    if matches!(expr.kind, HirExprKind::ConstInt(_)) && is_integer_ty(expr.ty) && is_integer_ty(expected_ty) {
        return Ok(HirExpr {
            kind: expr.kind,
            ty: expected_ty,
            span: expr.span,
        });
    }

    Err(format!(
        "initializer type mismatch: expected {expected_ty:?}, got {:?}",
        expr.ty
    ))
}

pub(crate) fn call_arg_type_compatible(expected: Ty, actual: Ty) -> bool {
    if expected == actual {
        return true;
    }
    ty_matches_expected(expected, actual)
}
