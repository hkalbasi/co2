use std::collections::HashMap;

use co2_parser::{BinOp as ParsedBinOp, Constant, Expression, Spanned, UnaryOp as ParsedUnaryOp};
use la_arena::Arena;
use rustc_public_generative::rustc_public::{
    mir::Mutability,
    ty::{IntTy, RigidTy, Span as RustSpan, Ty, TyKind},
};

use crate::decl::call_arg_type_compatible;
use crate::item::{HirLocal, LocalId};
use crate::resolver::{HirCtx, ResolvedValue};
use crate::ty::{is_integer_ty, resolve_field_in_adt, ty_matches_expected};

#[derive(Clone, Debug)]
pub struct HirExpr {
    pub kind: HirExprKind,
    pub ty: Ty,
    pub span: RustSpan,
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
    Subscript {
        base: Box<HirExpr>,
        index: Box<HirExpr>,
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
    Assign {
        lhs: Box<HirExpr>,
        rhs: Box<HirExpr>,
    },
    AddrOf(Box<HirExpr>),
    Deref(Box<HirExpr>),
}

#[derive(Clone, Debug, Copy)]
pub enum HirBinOp {
    Add,
    Sub,
    Mul,
    Or,
}

impl<R> HirCtx<'_, R> {
    pub(crate) fn lower_expr(
        &self,
        (expr, parser_span): Spanned<Expression>,
        locals: &Arena<HirLocal>,
        local_map: &HashMap<String, LocalId>,
    ) -> Result<HirExpr, String> {
        let span = self.to_rust_span(parser_span);
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

                let resolved = self
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
                let func_expr = self.lower_expr((func.0, func.1), locals, local_map)?;
                let sig = func_expr
                    .ty
                    .kind()
                    .fn_sig()
                    .expect("todo: emit type error here")
                    .skip_binder();

                let mut lowered_args = Vec::with_capacity(params.len());
                for param in params {
                    lowered_args.push(self.lower_expr((param.0, param.1), locals, local_map)?);
                }

                if sig.inputs().len() != lowered_args.len() {
                    return Err(format!(
                        "call argument count mismatch: expected {}, got {}",
                        sig.inputs().len(),
                        lowered_args.len()
                    ));
                }

                for (idx, (expected, actual)) in sig.inputs().iter().zip(lowered_args.iter()).enumerate() {
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
                let base = self.lower_expr(*base, locals, local_map)?;
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
            Expression::Subscript(base, index) => {
                let base = self.lower_expr(*base, locals, local_map)?;
                let index = self.lower_expr(*index, locals, local_map)?;
                if !is_integer_ty(index.ty) {
                    return Err(format!("subscript index must be integer, got {:?}", index.ty));
                }
                let TyKind::RigidTy(RigidTy::RawPtr(pointee, _)) = base.ty.kind() else {
                    return Err(format!("subscript base must be pointer type, got {:?}", base.ty));
                };
                Ok(HirExpr {
                    kind: HirExprKind::Subscript {
                        base: Box::new(base),
                        index: Box::new(index),
                    },
                    ty: pointee,
                    span,
                })
            }
            Expression::BinOp(lhs, op, rhs) => {
                if matches!(op, ParsedBinOp::Assign) {
                    let lhs = self.lower_expr(*lhs, locals, local_map)?;
                    if !is_place_expr(&lhs) {
                        return Err("assignment target is not assignable".to_owned());
                    }
                    let rhs = self.lower_expr(*rhs, locals, local_map)?;
                    if !ty_matches_expected(lhs.ty, rhs.ty) {
                        return Err(format!(
                            "assignment type mismatch: lhs={:?}, rhs={:?}",
                            lhs.ty, rhs.ty
                        ));
                    }
                    return Ok(HirExpr {
                        kind: HirExprKind::Assign {
                            lhs: Box::new(lhs.clone()),
                            rhs: Box::new(rhs),
                        },
                        ty: lhs.ty,
                        span,
                    });
                }

                let lhs = self.lower_expr(*lhs, locals, local_map)?;
                let rhs = self.lower_expr(*rhs, locals, local_map)?;
                if lhs.ty != rhs.ty {
                    return Err(format!(
                        "binary op type mismatch: lhs={:?}, rhs={:?}",
                        lhs.ty, rhs.ty
                    ));
                }
                let op = match op {
                    ParsedBinOp::Assign => unreachable!(),
                    ParsedBinOp::Add => HirBinOp::Add,
                    ParsedBinOp::Sub => HirBinOp::Sub,
                    ParsedBinOp::Mul => HirBinOp::Mul,
                    ParsedBinOp::Or => HirBinOp::Or,
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
            Expression::UnaryOp(op, expr) => {
                let inner = self.lower_expr(*expr, locals, local_map)?;
                match op {
                    ParsedUnaryOp::AddrOf => {
                        if !is_place_expr(&inner) {
                            return Err("cannot take address of non-place expression".to_owned());
                        }
                        Ok(HirExpr {
                            kind: HirExprKind::AddrOf(Box::new(inner.clone())),
                            ty: Ty::new_ptr(inner.ty, Mutability::Mut),
                            span,
                        })
                    }
                    ParsedUnaryOp::Deref => {
                        let TyKind::RigidTy(RigidTy::RawPtr(pointee, _)) = inner.ty.kind() else {
                            return Err(format!(
                                "cannot dereference non-pointer type: {:?}",
                                inner.ty
                            ));
                        };
                        Ok(HirExpr {
                            kind: HirExprKind::Deref(Box::new(inner)),
                            ty: pointee,
                            span,
                        })
                    }
                    ParsedUnaryOp::Plus => Ok(inner),
                    ParsedUnaryOp::Minus => {
                        if !is_integer_ty(inner.ty) {
                            return Err("unary `-` expects integer expression".to_owned());
                        }
                        Ok(HirExpr {
                            kind: HirExprKind::Binary {
                                op: HirBinOp::Sub,
                                lhs: Box::new(HirExpr {
                                    kind: HirExprKind::ConstInt(0),
                                    ty: inner.ty,
                                    span,
                                }),
                                rhs: Box::new(inner.clone()),
                            },
                            ty: inner.ty,
                            span,
                        })
                    }
                }
            }
            Expression::InitList(_) => {
                Err("initializer list is only valid in declarations".to_owned())
            }
            Expression::Empty => Err("empty expression is invalid here".to_owned()),
        }
    }
}

pub(crate) fn is_place_expr(expr: &HirExpr) -> bool {
    match &expr.kind {
        HirExprKind::Local(_) => true,
        HirExprKind::Field { base, .. } => is_place_expr(base),
        HirExprKind::Subscript { .. } => true,
        HirExprKind::Deref(_) => true,
        _ => false,
    }
}
