#![feature(rustc_private)]

use std::collections::HashMap;

pub use co2_parser::Span;
use co2_parser::{
    BinOp as ParsedBinOp, CompoundStatement, Constant, Declaration, DeclarationSpecifier,
    Declarator, Expression, InitDeclarator, Spanned, Statement,
    StatementOrDeclaration, Token, TypeSpecifier, parse_compound_statement,
};
use la_arena::{Arena, Idx};
use rustc_public_generative::rustc_public::{
    CrateDefType,     mir::Mutability,
    mir::Safety,
    ty::{Abi, Binder, FnDef, FnSig, IntTy, RigidTy, Ty, TyKind, UintTy, VariantIdx},
};

#[derive(Clone, Debug)]
pub enum ResolvedValue {
    Fn(FnDef),
}
impl ResolvedValue {
    fn ty(&self) -> Ty {
        match self {
            ResolvedValue::Fn(fn_def) => fn_def.ty(),
        }
    }
}

pub trait GlobalResolver {
    fn resolve_value(&self, path: &str) -> Option<ResolvedValue>;
    fn resolve_type(&self, path: &str) -> Option<Ty>;
}

#[derive(Clone, Debug)]
pub struct HirLocal {
    pub name: String,
    pub ty: Ty,
}

pub type LocalId = Idx<HirLocal>;

#[derive(Clone, Debug)]
pub struct HirBody {
    pub locals: Arena<HirLocal>,
    pub params: Vec<LocalId>,
    pub stmts: Vec<HirStmt>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum HirStmt {
    Decl(HirDecl),
    Expr(HirExpr),
    Return(Option<HirExpr>, Span),
}

#[derive(Clone, Debug)]
pub struct HirDecl {
    pub local: LocalId,
    pub initializer: Option<HirExpr>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct HirExpr {
    pub kind: HirExprKind,
    pub ty: Ty,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum HirExprKind {
    Local(LocalId),
    ConstInt(i64),
    ConstStr(String),
    Field {
        base: Box<HirExpr>,
        index: usize,
    },
    Binary {
        op: HirBinOp,
        lhs: Box<HirExpr>,
        rhs: Box<HirExpr>,
    },
    Aggregate {
        args: Vec<HirExpr>,
    },
    Path(ResolvedValue),
    Call {
        func: Box<HirExpr>,
        args: Vec<HirExpr>,
    },
}

#[derive(Clone, Debug, Copy)]
pub enum HirBinOp {
    Add,
    Sub,
    Mul,
}

pub fn lower_function_body(
    tokens: &[Spanned<Token>],
    source_name: &str,
    source: &'static str,
    def: FnDef,
    param_names: &[String],
    resolver: &dyn GlobalResolver,
) -> Result<HirBody, String> {
    let parsed = parse_compound_statement(tokens, source_name.to_owned(), source)
        .ok_or_else(|| "failed to parse function body".to_owned())?;
    lower_compound_statement(parsed, &def.fn_sig().skip_binder(), param_names, resolver)
}

// pub fn lower_function_signature(
//     declaration_specifiers: Vec<Spanned<DeclarationSpecifier>>,
//     declarator: Spanned<Declarator>,
//     resolver: &dyn GlobalResolver,
// ) -> Result<(String, FunctionSig), String> {
//     let base = base_ty_of_decl(declaration_specifiers, declarator.1, resolver)?;
//     let (decl_ty, name) = extract_decl_type(base, declarator, resolver)?;
//     let name = name.ok_or_else(|| "missing function name".to_owned())?;
//     let DeclTy::Function(sig) = decl_ty else {
//         return Err("declarator is not a function".to_owned());
//     };
//     Ok((name, sig))
// }

fn lower_value_decl_type(
    declaration_specifiers: Vec<Spanned<DeclarationSpecifier>>,
    declarator: Spanned<Declarator>,
    resolver: &dyn GlobalResolver,
) -> Result<(String, Ty), String> {
    let base = base_ty_of_decl(declaration_specifiers, declarator.1, resolver)?;
    let (decl_ty, name) = extract_decl_type(base, declarator, resolver)?;
    let name = name.ok_or_else(|| "missing declaration name".to_owned())?;
    Ok((name, decl_ty))
}

fn lower_compound_statement(
    (compound, span): Spanned<CompoundStatement>,
    sig: &FnSig,
    param_names: &[String],
    resolver: &dyn GlobalResolver,
) -> Result<HirBody, String> {
    let mut locals = Arena::new();
    let mut params = Vec::new();
    let mut local_map: HashMap<String, LocalId> = HashMap::new();

    locals.alloc(HirLocal {
        name: "_ret".to_owned(),
        ty: sig.output(),
    });

    for (idx, ty) in sig.inputs().iter().enumerate() {
        let name = param_names
            .get(idx)
            .cloned()
            .unwrap_or_else(|| format!("arg{idx}"));
        let id = locals.alloc(HirLocal {
            name: name.clone(),
            ty: *ty,
        });
        params.push(id);
        local_map.insert(name, id);
    }

    let mut stmts = Vec::new();
    for (stmt_or_decl, stmt_span) in compound.statements {
        match stmt_or_decl {
            StatementOrDeclaration::Statement((stmt, span)) => {
                lower_stmt(
                    stmt,
                    span,
                    &mut stmts,
                    &mut locals,
                    &mut local_map,
                    resolver,
                )?;
            }
            StatementOrDeclaration::Declaration((decl, _)) => {
                lower_decl(
                    decl,
                    stmt_span,
                    &mut stmts,
                    &mut locals,
                    &mut local_map,
                    resolver,
                )?;
            }
        }
    }

    Ok(HirBody {
        locals,
        params,
        stmts,
        span,
    })
}

fn lower_stmt(
    stmt: Statement,
    span: Span,
    out: &mut Vec<HirStmt>,
    locals: &mut Arena<HirLocal>,
    local_map: &mut HashMap<String, LocalId>,
    resolver: &dyn GlobalResolver,
) -> Result<(), String> {
    match stmt {
        Statement::Return(expr) => {
            if let Some(expr) = expr {
                let expr = lower_expr((expr.0, expr.1), locals, local_map, resolver)?;
                out.push(HirStmt::Return(Some(expr), span));
            } else {
                out.push(HirStmt::Return(None, span));
            }
        }
        Statement::Expression(expr) => {
            let expr = lower_expr(expr, locals, local_map, resolver)?;
            out.push(HirStmt::Expr(expr));
        }
    }
    Ok(())
}

fn lower_decl(
    decl: Declaration,
    span: Span,
    out: &mut Vec<HirStmt>,
    locals: &mut Arena<HirLocal>,
    local_map: &mut HashMap<String, LocalId>,
    resolver: &dyn GlobalResolver,
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
                let (name, ty) =
                    lower_value_decl_type(declaration_specifiers.clone(), declarator, resolver)?;

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
                                .map(|item| lower_expr(item, locals, local_map, resolver))
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
                                let mut reordered: Vec<Option<HirExpr>> =
                                    vec![None; field_tys.len()];
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
                        other => Some(lower_expr(
                            (other, init_expr.1),
                            locals,
                            local_map,
                            resolver,
                        )?),
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

fn coerce_expr_to_type(expr: HirExpr, expected_ty: Ty) -> Result<HirExpr, String> {
    if expr.ty == expected_ty {
        return Ok(expr);
    }

    if matches!(expr.kind, HirExprKind::ConstInt(_))
        && is_integer_ty(expr.ty)
        && is_integer_ty(expected_ty)
    {
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

fn is_integer_ty(ty: Ty) -> bool {
    matches!(
        ty.kind(),
        TyKind::RigidTy(RigidTy::Int(_)) | TyKind::RigidTy(RigidTy::Uint(_))
    )
}

fn lower_expr(
    (expr, span): Spanned<Expression>,
    locals: &Arena<HirLocal>,
    local_map: &HashMap<String, LocalId>,
    resolver: &dyn GlobalResolver,
) -> Result<HirExpr, String> {
    match expr {
        Expression::Identifier(path) => {
            let path_str = path.0.to_pretty();
            if let Some(local) = local_map.get(&path_str) {
                let local_decl = &locals[*local];
                return Ok(HirExpr {
                    kind: HirExprKind::Local(*local),
                    ty: local_decl.ty,
                    span,
                });
            }

            let resolved = resolver
                .resolve_value(&path_str)
                .ok_or_else(|| format!("unresolved value path: {path_str}"))?;
            Ok(HirExpr {
                kind: HirExprKind::Path(resolved.clone()),
                ty: resolved.ty(),
                span,
            })
        }
        Expression::Constant(Constant::Int(v)) => Ok(HirExpr {
            kind: HirExprKind::ConstInt(v as i64),
            ty: Ty::signed_ty(IntTy::I32),
            span,
        }),
        Expression::Constant(Constant::String(s)) => Ok(HirExpr {
            kind: HirExprKind::ConstStr(s),
            ty: Ty::new_ptr(Ty::signed_ty(IntTy::I8), Mutability::Mut),
            span,
        }),
        Expression::Call { func, params } => {
            let func_expr = lower_expr((func.0, func.1), locals, local_map, resolver)?;
            let sig = func_expr
                .ty
                .kind()
                .fn_sig()
                .expect("todo: emit type error here")
                .skip_binder();

            let mut lowered_args = Vec::with_capacity(params.len());
            for param in params {
                lowered_args.push(lower_expr((param.0, param.1), locals, local_map, resolver)?);
            }

            if sig.inputs().len() != lowered_args.len() {
                return Err(format!(
                    "call argument count mismatch: expected {}, got {}",
                    sig.inputs().len(),
                    lowered_args.len()
                ));
            }

            for (idx, (expected, actual)) in
                sig.inputs().iter().zip(lowered_args.iter()).enumerate()
            {
                if !call_arg_type_compatible(*expected, actual.ty) {
                    return Err(format!(
                        "call argument type mismatch at index {idx}: expected {expected:?}, got {:?}",
                        actual.ty
                    ));
                }
            }

            Ok(HirExpr {
                kind: HirExprKind::Call {
                    func: Box::new(func_expr),
                    args: lowered_args,
                },
                ty: sig.output(),
                span,
            })
        }
        Expression::Field(base, field) => {
            let base = lower_expr(*base, locals, local_map, resolver)?;
            let (index, field_ty) = resolve_field_in_adt(base.ty, &field.0)
                .ok_or_else(|| format!("unknown field `{}`", field.0))?;
            Ok(HirExpr {
                kind: HirExprKind::Field {
                    base: Box::new(base),
                    index,
                },
                ty: field_ty,
                span,
            })
        }
        Expression::BinOp(lhs, op, rhs) => {
            let lhs = lower_expr(*lhs, locals, local_map, resolver)?;
            let rhs = lower_expr(*rhs, locals, local_map, resolver)?;
            if lhs.ty != rhs.ty {
                return Err(format!(
                    "binary op type mismatch: lhs={:?}, rhs={:?}",
                    lhs.ty, rhs.ty
                ));
            }
            let op = match op {
                ParsedBinOp::Add => HirBinOp::Add,
                ParsedBinOp::Sub => HirBinOp::Sub,
                ParsedBinOp::Mul => HirBinOp::Mul,
            };
            Ok(HirExpr {
                kind: HirExprKind::Binary {
                    op,
                    lhs: Box::new(lhs.clone()),
                    rhs: Box::new(rhs),
                },
                ty: lhs.ty,
                span,
            })
        }
        Expression::InitList(_) => Err("initializer list is only valid in declarations".to_owned()),
        Expression::Subscript(_, _) => Err("subscript is not supported yet".to_owned()),
        Expression::Empty => Err("empty expression is invalid here".to_owned()),
    }
}

fn call_arg_type_compatible(expected: Ty, actual: Ty) -> bool {
    if expected == actual {
        return true;
    }
    ty_matches_expected(expected, actual)
}

fn ty_matches_expected(expected: Ty, actual: Ty) -> bool {
    if expected == actual {
        return true;
    }
    match (expected.kind(), actual.kind()) {
        (TyKind::Param(_), _) => true,
        (TyKind::RigidTy(RigidTy::Ref(_, exp_inner, _)), _) => {
            ty_matches_expected(exp_inner, actual)
        }
        (
            TyKind::RigidTy(RigidTy::Adt(exp_adt, exp_args)),
            TyKind::RigidTy(RigidTy::Adt(act_adt, act_args)),
        ) => {
            if exp_adt != act_adt || exp_args.0.len() != act_args.0.len() {
                return false;
            }
            exp_args
                .0
                .iter()
                .zip(act_args.0.iter())
                .all(|(e, a)| match (e, a) {
                    (rustc_public_generative::rustc_public::ty::GenericArgKind::Type(et), rustc_public_generative::rustc_public::ty::GenericArgKind::Type(at)) => {
                        ty_matches_expected(*et, *at)
                    }
                    _ => e == a,
                })
        }
        _ => false,
    }
}

fn base_ty_of_decl(
    specifiers: Vec<Spanned<DeclarationSpecifier>>,
    span: Span,
    resolver: &dyn GlobalResolver,
) -> Result<Ty, String> {
    for (specifier, _) in &specifiers {
        if let DeclarationSpecifier::TypeSpecifier((type_specifier, _)) = specifier {
            let ty = match type_specifier {
                TypeSpecifier::Int => Ty::signed_ty(IntTy::I32),
                TypeSpecifier::Void => Ty::new_tuple(&[]),
                TypeSpecifier::Char => Ty::signed_ty(IntTy::I8),
                TypeSpecifier::Short => Ty::signed_ty(IntTy::I16),
                TypeSpecifier::Long => Ty::signed_ty(IntTy::I64),
                TypeSpecifier::Float => return Err("float is not supported".to_owned()),
                TypeSpecifier::Double => return Err("double is not supported".to_owned()),
                TypeSpecifier::Signed | TypeSpecifier::Unsigned => continue,
                TypeSpecifier::StructOrUnion { .. } => {
                    return Err("struct/union types are not supported yet".to_owned());
                }
                TypeSpecifier::TypedefName(path) => {
                    let name = path.0.to_pretty();
                    resolver
                        .resolve_type(&name)
                        .ok_or_else(|| format!("unresolved typedef path: {name}"))?
                }
            };
            return Ok(ty);
        }
    }
    Err(format!("no suitable type specifier at span {span:?}"))
}

fn extract_decl_type(
    base: Ty,
    (decl, span): Spanned<Declarator>,
    resolver: &dyn GlobalResolver,
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
                let param_base = base_ty_of_decl(param.0, span, resolver)?;
                let (param_ty, _) = extract_decl_type(param_base, param.1, resolver)?;
                inputs.push(param_ty);
            }

            let (ret, name) = extract_decl_type(base, *declarator, resolver)?;
            let mut inputs_and_output = inputs;
            inputs_and_output.push(ret);
            let sig = FnSig {
                inputs_and_output,
                c_variadic: false,
                safety: Safety::Safe,
                abi: Abi::Rust,
            };
            Ok((
                Ty::from_rigid_kind(RigidTy::FnPtr(Binder::dummy(sig))),
                name,
            ))
        }
        Declarator::PointerDeclarator { declarator, .. } => {
            let (inner, name) = extract_decl_type(base, *declarator, resolver)?;
            Ok((Ty::new_ptr(inner, Mutability::Mut), name))
        }
        Declarator::ArrayDeclarator { declarator, .. } => {
            let (inner, name) = extract_decl_type(base, *declarator, resolver)?;
            Ok((Ty::new_ptr(inner, Mutability::Mut), name))
        }
    }
}

fn resolve_field_in_adt(base: Ty, field: &str) -> Option<(usize, Ty)> {
    let TyKind::RigidTy(RigidTy::Adt(adt, args)) = base.kind() else {
        return None;
    };
    let variant = adt.variant(variant_idx(0))?;
    let fields = variant.fields();
    for (idx, field_def) in fields.iter().enumerate() {
        if field_def.name.to_string() == field {
            return Some((idx, field_def.ty_with_args(&args)));
        }
    }
    None
}

fn adt_field_tys(base: Ty) -> Option<Vec<Ty>> {
    let TyKind::RigidTy(RigidTy::Adt(adt, args)) = base.kind() else {
        return None;
    };
    let variant = adt.variant(variant_idx(0))?;
    Some(
        variant
            .fields()
            .into_iter()
            .map(|f| f.ty_with_args(&args))
            .collect(),
    )
}

fn variant_idx(id: usize) -> VariantIdx {
    unsafe { std::mem::transmute::<usize, VariantIdx>(id) }
}

pub fn primitive_type(name: &str) -> Option<Ty> {
    match name {
        "u8" => Some(Ty::unsigned_ty(UintTy::U8)),
        "i8" => Some(Ty::signed_ty(IntTy::I8)),
        "u32" => Some(Ty::unsigned_ty(UintTy::U32)),
        "i32" => Some(Ty::signed_ty(IntTy::I32)),
        "usize" => Some(Ty::usize_ty()),
        "isize" => Some(Ty::signed_ty(IntTy::Isize)),
        "void" => Some(Ty::new_tuple(&[])),
        _ => None,
    }
}
