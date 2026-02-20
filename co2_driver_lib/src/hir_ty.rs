use co2_parser::{DeclarationSpecifier, Declarator, Span, Spanned, TypeSpecifier};
use rustc_public_generative::rustc_public::{
    DefId,
    mir::Mutability,
    ty::{AdtDef, IntTy, UintTy},
};
use rustc_public_generative::{FunctionAbi, FunctionSignature, HirStructureCtx, HirTy, HirTyKind};

use crate::span::co2_span_to_rustc;

pub fn lower_function_signature(
    ctx: &HirStructureCtx,
    declaration_specifiers: Vec<Spanned<DeclarationSpecifier>>,
    declarator: Spanned<Declarator>,
    typedefs: &std::collections::HashMap<String, DefId>,
) -> Result<(String, FunctionSignature, Vec<String>), String> {
    let parsed_param_names = function_param_names(&declarator.0);
    let (name, ty) = lower_value_decl_type(ctx, declaration_specifiers, declarator, typedefs)?;
    let HirTyKind::FnPtr(sig) = ty.kind else {
        return Err("it wasn't function".to_owned());
    };
    let sig = *sig;
    let names = parsed_param_names
        .unwrap_or_else(|| vec![None; sig.inputs.len()])
        .into_iter()
        .enumerate()
        .map(|(idx, n)| n.unwrap_or_else(|| format!("arg{idx}")))
        .collect();
    Ok((name, sig, names))
}

pub fn lower_value_decl_type(
    ctx: &HirStructureCtx,
    declaration_specifiers: Vec<Spanned<DeclarationSpecifier>>,
    declarator: Spanned<Declarator>,
    typedefs: &std::collections::HashMap<String, DefId>,
) -> Result<(String, HirTy), String> {
    let base = base_ty_of_decl(ctx, declaration_specifiers, declarator.1, typedefs)?;
    let (decl_ty, name) = extract_decl_type(ctx, base, declarator, typedefs)?;
    let name = name.ok_or_else(|| "missing declaration name".to_owned())?;
    Ok((name, decl_ty))
}

fn base_ty_of_decl(
    ctx: &HirStructureCtx,
    specifiers: Vec<Spanned<DeclarationSpecifier>>,
    span: Span,
    typedefs: &std::collections::HashMap<String, DefId>,
) -> Result<HirTy, String> {
    let span = co2_span_to_rustc(ctx, span);
    for (specifier, _) in &specifiers {
        if let DeclarationSpecifier::TypeSpecifier((type_specifier, _)) = specifier {
            let ty = match type_specifier {
                TypeSpecifier::Int => HirTy::signed_ty(IntTy::I32, span),
                TypeSpecifier::Void => HirTy::new_tuple(vec![], span),
                TypeSpecifier::Char => HirTy::signed_ty(IntTy::I8, span),
                TypeSpecifier::Short => HirTy::signed_ty(IntTy::I16, span),
                TypeSpecifier::Long => HirTy::signed_ty(IntTy::I64, span),
                TypeSpecifier::Float => return Err("float is not supported".to_owned()),
                TypeSpecifier::Double => return Err("double is not supported".to_owned()),
                TypeSpecifier::Signed | TypeSpecifier::Unsigned => continue,
                TypeSpecifier::StructOrUnion { .. } => {
                    return Err("struct/union types are not supported yet".to_owned());
                }
                TypeSpecifier::TypedefName(path) => {
                    let name = path.0.to_pretty();
                    if let Some(prim) = primitive_hir_type(&name, span) {
                        prim
                    } else if let Some(def_id) = typedefs.get(&name) {
                        HirTy::adt(AdtDef(*def_id), vec![], span)
                    } else if let Some(last) = name.rsplit("::").next() {
                        if let Some(def_id) = typedefs.get(last) {
                            HirTy::adt(AdtDef(*def_id), vec![], span)
                        } else {
                            return Err(format!("unresolved typedef path: {name}"));
                        }
                    } else {
                        return Err(format!("unresolved typedef path: {name}"));
                    }
                }
            };
            return Ok(ty);
        }
    }
    Err(format!("no suitable type specifier at span {span:?}"))
}

fn extract_decl_type(
    ctx: &HirStructureCtx,
    base: HirTy,
    (decl, span): Spanned<Declarator>,
    typedefs: &std::collections::HashMap<String, DefId>,
) -> Result<(HirTy, Option<String>), String> {
    let rust_span = co2_span_to_rustc(ctx, span);
    match decl {
        Declarator::Abstract => Ok((base, None)),
        Declarator::Identifier((ident, _)) => Ok((base, Some(ident))),
        Declarator::FunctionDeclarator {
            declarator,
            param_list,
        } => {
            let mut inputs = Vec::with_capacity(param_list.parameters.len());
            for param in param_list.parameters {
                let param_base = base_ty_of_decl(ctx, param.0, span, typedefs)?;
                let (param_ty, _) = extract_decl_type(ctx, param_base, param.1, typedefs)?;
                inputs.push(param_ty);
            }

            let (ret, name) = extract_decl_type(ctx, base, *declarator, typedefs)?;
            Ok((
                HirTy {
                    kind: HirTyKind::FnPtr(Box::new(FunctionSignature {
                        inputs,
                        output: ret,
                        abi: FunctionAbi::Rust,
                        is_unsafe: false,
                    })),
                    span: rust_span,
                },
                name,
            ))
        }
        Declarator::PointerDeclarator { declarator, .. } => {
            let (inner, name) = extract_decl_type(ctx, base, *declarator, typedefs)?;
            Ok((HirTy::new_ptr(inner, Mutability::Mut, rust_span), name))
        }
        Declarator::ArrayDeclarator { declarator, .. } => {
            let (inner, name) = extract_decl_type(ctx, base, *declarator, typedefs)?;
            Ok((HirTy::new_ptr(inner, Mutability::Mut, rust_span), name))
        }
    }
}

fn primitive_hir_type(
    name: &str,
    span: rustc_public_generative::rustc_public::ty::Span,
) -> Option<HirTy> {
    match name {
        "u8" => Some(HirTy::unsigned_ty(UintTy::U8, span)),
        "i8" => Some(HirTy::signed_ty(IntTy::I8, span)),
        "u16" => Some(HirTy::unsigned_ty(UintTy::U16, span)),
        "i16" => Some(HirTy::signed_ty(IntTy::I16, span)),
        "u32" => Some(HirTy::unsigned_ty(UintTy::U32, span)),
        "i32" => Some(HirTy::signed_ty(IntTy::I32, span)),
        "u64" => Some(HirTy::unsigned_ty(UintTy::U64, span)),
        "i64" => Some(HirTy::signed_ty(IntTy::I64, span)),
        "usize" => Some(HirTy::usize_ty(span)),
        "isize" => Some(HirTy::signed_ty(IntTy::Isize, span)),
        _ => None,
    }
}

fn function_param_names(decl: &Declarator) -> Option<Vec<Option<String>>> {
    match decl {
        Declarator::FunctionDeclarator { param_list, .. } => Some(
            param_list
                .parameters
                .iter()
                .map(|param| declarator_name(&param.1.0))
                .collect(),
        ),
        Declarator::PointerDeclarator { declarator, .. }
        | Declarator::ArrayDeclarator { declarator, .. } => function_param_names(&declarator.0),
        _ => None,
    }
}

fn declarator_name(decl: &Declarator) -> Option<String> {
    match decl {
        Declarator::Identifier((name, _)) => Some(name.clone()),
        Declarator::PointerDeclarator { declarator, .. }
        | Declarator::ArrayDeclarator { declarator, .. }
        | Declarator::FunctionDeclarator { declarator, .. } => declarator_name(&declarator.0),
        Declarator::Abstract => None,
    }
}
