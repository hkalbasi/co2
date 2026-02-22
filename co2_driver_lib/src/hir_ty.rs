use co2_parser::{DeclarationSpecifier, Declarator, Span, Spanned, StructOrUnionSpecifier, TypeSpecifier};
use rustc_public_generative::rustc_public::{
    DefId,
    mir::Mutability,
    ty::{AdtDef, IntTy, UintTy},
};
use rustc_public_generative::{FunctionAbi, FunctionSignature, HirStructureCtx, HirTy, HirTyKind};

use crate::span::co2_span_to_rustc;

enum TyOrFunction {
    Ty(HirTy),
    Function(FunctionSignature),
}

pub fn lower_function_signature(
    ctx: &HirStructureCtx,
    declaration_specifiers: Vec<Spanned<DeclarationSpecifier>>,
    declarator: Spanned<Declarator>,
    typedefs: &std::collections::HashMap<String, DefId>,
    typedef_hir_tys: &std::collections::HashMap<String, HirTy>,
) -> Result<(String, FunctionSignature, Vec<String>), String> {
    let parsed_param_names = function_param_names(&declarator.0);
    let base = base_ty_of_decl(ctx, declaration_specifiers, declarator.1, typedefs, typedef_hir_tys)?;
    let (decl_ty, name) = extract_decl_type(
        ctx,
        TyOrFunction::Ty(base),
        declarator,
        typedefs,
        typedef_hir_tys,
    )?;
    let name = name.ok_or_else(|| "missing declaration name".to_owned())?;
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

pub fn lower_value_decl_type(
    ctx: &HirStructureCtx,
    declaration_specifiers: Vec<Spanned<DeclarationSpecifier>>,
    declarator: Spanned<Declarator>,
    typedefs: &std::collections::HashMap<String, DefId>,
    typedef_hir_tys: &std::collections::HashMap<String, HirTy>,
) -> Result<(String, HirTy), String> {
    let base = base_ty_of_decl(ctx, declaration_specifiers, declarator.1, typedefs, typedef_hir_tys)?;
    let (decl_ty, name) = extract_decl_type(
        ctx,
        TyOrFunction::Ty(base),
        declarator,
        typedefs,
        typedef_hir_tys,
    )?;
    let name = name.ok_or_else(|| "missing declaration name".to_owned())?;
    match decl_ty {
        TyOrFunction::Ty(ty) => Ok((name, ty)),
        TyOrFunction::Function(_) => {
            Err("function is not a first-class declaration type in this context".to_owned())
        }
    }
}

fn base_ty_of_decl(
    ctx: &HirStructureCtx,
    specifiers: Vec<Spanned<DeclarationSpecifier>>,
    span: Span,
    typedefs: &std::collections::HashMap<String, DefId>,
    typedef_hir_tys: &std::collections::HashMap<String, HirTy>,
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
                TypeSpecifier::Enum(_) => HirTy::signed_ty(IntTy::I32, span),
                TypeSpecifier::StructOrUnion { specifier, .. } => match specifier {
                    StructOrUnionSpecifier::Declared { ident } => {
                        if let Some(def_id) = typedefs.get(&ident.0) {
                            HirTy::adt(AdtDef(*def_id), vec![], span)
                        } else {
                            return Err(format!("unresolved struct/union tag: {}", ident.0));
                        }
                    }
                    StructOrUnionSpecifier::Defined { .. } | StructOrUnionSpecifier::Anonymous { .. } => {
                        let key = specifier
                            .canonical_field_set_key()
                            .ok_or_else(|| {
                                "struct/union type is only supported when declared at top level".to_owned()
                            })?;
                        if let Some(def_id) = typedefs.get(&key) {
                            HirTy::adt(AdtDef(*def_id), vec![], span)
                        } else {
                            return Err(format!(
                                "anonymous struct type is not predeclared at top level: {key}"
                            ));
                        }
                    }
                },
                TypeSpecifier::TypedefName(path) => {
                    let name = path.0.to_pretty();
                    if let Some(prim) = primitive_hir_type(&name, span) {
                        prim
                    } else if let Some(ty) = typedef_hir_tys.get(&name) {
                        ty.clone()
                    } else if let Some(def_id) = typedefs.get(&name) {
                        HirTy::adt(AdtDef(*def_id), vec![], span)
                    } else if let Some(last) = name.rsplit("::").next() {
                        if let Some(ty) = typedef_hir_tys.get(last) {
                            ty.clone()
                        } else if let Some(def_id) = typedefs.get(last) {
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
    current: TyOrFunction,
    (decl, span): Spanned<Declarator>,
    typedefs: &std::collections::HashMap<String, DefId>,
    typedef_hir_tys: &std::collections::HashMap<String, HirTy>,
) -> Result<(TyOrFunction, Option<String>), String> {
    let rust_span = co2_span_to_rustc(ctx, span);
    match decl {
        Declarator::Abstract => Ok((current, None)),
        Declarator::Identifier((ident, _)) => Ok((current, Some(ident))),
        Declarator::FunctionDeclarator {
            declarator,
            param_list,
        } => {
            let mut inputs = Vec::with_capacity(param_list.parameters.len());
            if !(param_list.parameters.len() == 1
                && parameter_is_void(&param_list.parameters[0]))
            {
                for param in param_list.parameters {
                    let param_base =
                        base_ty_of_decl(ctx, param.0, span, typedefs, typedef_hir_tys)?;
                    let (param_decl_ty, _) = extract_decl_type(
                        ctx,
                        TyOrFunction::Ty(param_base),
                        param.1,
                        typedefs,
                        typedef_hir_tys,
                    )?;
                    let param_ty = match param_decl_ty {
                        TyOrFunction::Ty(ty) => ty,
                        TyOrFunction::Function(_) => {
                            return Err("function type is invalid in parameter position".to_owned());
                        }
                    };
                    inputs.push(param_ty);
                }
            }
            let function_ty = match current {
                TyOrFunction::Ty(ret) => TyOrFunction::Function(FunctionSignature {
                    inputs,
                    output: ret,
                    abi: FunctionAbi::Rust,
                    is_unsafe: false,
                }),
                TyOrFunction::Function(_) => {
                    return Err("function returning function is not supported".to_owned());
                }
            };
            extract_decl_type(ctx, function_ty, *declarator, typedefs, typedef_hir_tys)
        }
        Declarator::PointerDeclarator { declarator, .. } => {
            let ptr_or_fn_ptr = match current {
                TyOrFunction::Ty(inner) => {
                    TyOrFunction::Ty(HirTy::new_ptr(inner, Mutability::Mut, rust_span))
                }
                TyOrFunction::Function(sig) => TyOrFunction::Ty(HirTy {
                    kind: HirTyKind::FnPtr(Box::new(sig)),
                    span: rust_span,
                }),
            };
            extract_decl_type(ctx, ptr_or_fn_ptr, *declarator, typedefs, typedef_hir_tys)
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
            extract_decl_type(ctx, array_ty, *declarator, typedefs, typedef_hir_tys)
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

fn parameter_is_void(param: &(Vec<Spanned<DeclarationSpecifier>>, Spanned<Declarator>)) -> bool {
    let declarator_is_abstract = matches!(param.1.0, Declarator::Abstract);
    let has_void_type = param.0.iter().any(|(spec, _)| {
        matches!(
            spec,
            DeclarationSpecifier::TypeSpecifier((TypeSpecifier::Void, _))
        )
    });
    declarator_is_abstract && has_void_type
}
