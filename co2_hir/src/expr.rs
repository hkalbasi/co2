use std::collections::HashMap;

use co2_ast::{
    BinOp as ParsedBinOp, Constant, Expression, IntegerSuffix, Spanned, Statement,
    StatementOrDeclaration, UnaryOp as ParsedUnaryOp, UpdateOp as ParsedUpdateOp,
};
use co2_crate_sig::LocalResolver;
use la_arena::Arena;
use rustc_public_generative::rustc_public::{
    mir::Mutability,
    ty::{FloatTy, IntTy, RigidTy, Span as RustSpan, Ty, TyKind, UintTy},
};

use crate::item::{HirLocal, LocalId};
use crate::resolver::{HirCtx, ResolvedValue};
use crate::stmt::HirStmt;
use crate::ty::{
    adt_field_tys, array_elem_ty, callable_sig, common_numeric_ty, is_array_ty, is_condition_ty,
    is_maybe_uninit_fn_ptr_ty, is_numeric_ty, needs_implicit_cast, resolve_field_path_in_adt,
    ty_matches_expected,
};
use crate::{initializer_tree::InitializerTree, ty::common_ternary_ty};

#[derive(Clone)]
pub struct HirExpr {
    pub kind: HirExprKind,
    pub ty: Ty,
    pub span: RustSpan,
}

impl std::fmt::Debug for HirExpr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.kind {
            HirExprKind::ConstInt(i) => write!(f, "{i}"),
            _ => write!(f, "expr"),
        }
    }
}

#[derive(Clone, Debug)]
pub enum HirExprKind {
    Local(LocalId),
    ConstInt(i128),
    ConstFloat(f64),
    ConstStr(String),
    ArrayToPointer(Box<HirExpr>),
    Zeroed,
    Field {
        base: Box<HirExpr>,
        index: usize,
    },
    PtrOffset {
        base: Box<HirExpr>,
        index: Box<HirExpr>,
    },
    PtrDiff {
        lhs: Box<HirExpr>,
        rhs: Box<HirExpr>,
    },
    Binary {
        op: HirBinOp,
        lhs: Box<HirExpr>,
        rhs: Box<HirExpr>,
    },
    Comma {
        lhs: Box<HirExpr>,
        rhs: Box<HirExpr>,
    },
    Logical {
        op: HirLogicalOp,
        lhs: Box<HirExpr>,
        rhs: Box<HirExpr>,
    },
    LogicalNot(Box<HirExpr>),
    BitNot(Box<HirExpr>),
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
    AssignWithBinOp {
        lhs: Box<HirExpr>,
        rhs: Box<HirExpr>,
        op: HirBinOp,
        binop_ty: Ty,
        return_semantic: ReturnSemantic,
    },
    AssignPtrOffset {
        lhs: Box<HirExpr>,
        rhs: Box<HirExpr>,
        return_semantic: ReturnSemantic,
    },
    AddrOf(Box<HirExpr>),
    Deref(Box<HirExpr>),
    Cast(Box<HirExpr>),
    Conditional {
        cond: Box<HirExpr>,
        then_expr: Box<HirExpr>,
        else_expr: Box<HirExpr>,
    },
    StatementExpr {
        statements: Vec<HirStmt>,
        tail: Box<HirExpr>,
    },
    
    VaStart(Box<HirExpr>),
    VaArg(Box<HirExpr>),
    VaEnd(Box<HirExpr>),
}

#[derive(Clone, Debug, Copy)]
pub enum ReturnSemantic {
    /// For x += 1 and ++x
    AfterAssign,
    /// For x++
    BeforeAssign,
}

#[derive(Clone, Debug, Copy)]
pub enum HirBinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    BitOr,
    BitXor,
    BitAnd,
    Eq,
    Lt,
    Le,
    Ne,
    Ge,
    Gt,
    Shl,
    Shr,
}

impl HirBinOp {
    fn is_comparison(&self) -> bool {
        matches!(
            self,
            HirBinOp::Eq | HirBinOp::Lt | HirBinOp::Le | HirBinOp::Ne | HirBinOp::Ge | HirBinOp::Gt
        )
    }
}

#[derive(Clone, Debug, Copy)]
pub enum HirLogicalOp {
    Or,
    And,
}

impl HirCtx<'_> {
    fn array_to_pointer_decay_if_array(&self, expr: &mut HirExpr) {
        if !is_array_ty(expr.ty) {
            return;
        }
        *expr = self.array_to_pointer_decay(expr.clone());
    }

    fn array_to_pointer_decay(&self, expr: HirExpr) -> HirExpr {
        let elem = array_elem_ty(expr.ty).expect("Expr is not array");
        HirExpr {
            kind: HirExprKind::ArrayToPointer(Box::new(expr.clone())),
            ty: Ty::new_ptr(elem, Mutability::Mut),
            span: expr.span,
        }
    }

    fn emit_cast(&self, expr: HirExpr, ty: Ty) -> HirExpr {
        if expr.ty == ty {
            expr
        } else {
            HirExpr {
                span: expr.span,
                ty,
                kind: HirExprKind::Cast(Box::new(expr)),
            }
        }
    }

    pub(crate) fn lower_expr(
        &self,
        (expr, parser_span): Spanned<Expression<LocalResolver>>,
        locals: &mut Arena<HirLocal>,
        local_map: &mut HashMap<usize, LocalId>,
    ) -> Result<HirExpr, String> {
        let span = self.to_rust_span(parser_span);
        match expr {
            Expression::Identifier(path) => match path.0 {
                co2_crate_sig::DefOrLocal::Def(def_id) => {
                    let resolved = self.resolve_value(def_id);
                    Ok(HirExpr {
                        kind: HirExprKind::Path(resolved.clone()),
                        ty: resolved.ty(),
                        span,
                    })
                }
                co2_crate_sig::DefOrLocal::Local(l) => {
                    let Some(&local) = local_map.get(&(l as usize)) else {
                        self.terminate_with_error(
                            parser_span,
                            &format!("Invalid local {l}. Available locals are {:#?}", local_map),
                        );
                    };
                    let local_decl = &locals[local];
                    return Ok(HirExpr {
                        kind: HirExprKind::Local(local),
                        ty: local_decl.ty,
                        span,
                    });
                }
                co2_crate_sig::DefOrLocal::Prim(_)
                | co2_crate_sig::DefOrLocal::UnrepresentableType(_) => {
                    panic!("Invalid type in expression")
                }
            },
            Expression::Constant(Constant::Int(v, suffix)) => Ok(HirExpr {
                kind: HirExprKind::ConstInt(v),
                ty: int_suffix_ty(suffix, v),
                span,
            }),
            Expression::Constant(Constant::Float(v)) => Ok(HirExpr {
                kind: HirExprKind::ConstFloat(v),
                ty: Ty::from_rigid_kind(RigidTy::Float(FloatTy::F64)),
                span,
            }),
            Expression::Constant(Constant::Char(ch)) => Ok(HirExpr {
                kind: HirExprKind::ConstInt(ch as i128),
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
                let Some(sig) = callable_sig(func_expr.ty) else {
                    self.terminate_with_error(parser_span, "Type is not callable");
                };

                let sig = sig.skip_binder();

                let mut lowered_args = Vec::with_capacity(params.len());
                for param in params {
                    let mut arg = self.lower_expr((param.0, param.1), locals, local_map)?;
                    self.array_to_pointer_decay_if_array(&mut arg);
                    lowered_args.push(arg);
                }

                if sig.inputs().len() != lowered_args.len() && !sig.c_variadic {
                    self.terminate_with_error(
                        parser_span,
                        &format!(
                            "call argument count mismatch: expected {}, got {}",
                            sig.inputs().len(),
                            lowered_args.len()
                        ),
                    );
                }

                for (idx, actual) in lowered_args.iter_mut().enumerate() {
                    let expected = match sig.inputs().get(idx) {
                        Some(ty) => *ty,
                        None => {
                            if actual.ty.kind().is_adt() {
                                *actual = HirExpr {
                                    kind: HirExprKind::AddrOf(Box::new(actual.clone())),
                                    ty: Ty::new_ptr(actual.ty, Mutability::Mut),
                                    span: actual.span,
                                };
                                continue;
                            }
                            ty_passed_to_variadic(actual.ty)
                        },
                    };
                    if needs_implicit_cast(expected, actual.ty) {
                        *actual = HirExpr {
                            kind: HirExprKind::Cast(Box::new(actual.clone())),
                            ty: expected,
                            span: actual.span,
                        };
                    }
                    if !ty_matches_expected(expected, actual.ty) {
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
                let (path, field_ty) = resolve_field_path_in_adt(base.ty, &field.0)
                    .ok_or_else(|| format!("unknown field `{}` on type {:?}", field.0, base.ty))?;
                self.project_field_path(base, &path, field_ty, span)
            }
            Expression::Arrow(base, field) => {
                let mut base = self.lower_expr(*base, locals, local_map)?;
                self.array_to_pointer_decay_if_array(&mut base);
                let TyKind::RigidTy(RigidTy::RawPtr(pointee, _)) = base.ty.kind() else {
                    return Err(format!(
                        "arrow base must be pointer type, got {:?}",
                        base.ty
                    ));
                };
                let deref_base = HirExpr {
                    kind: HirExprKind::Deref(Box::new(base)),
                    ty: pointee,
                    span,
                };
                let (path, field_ty) = resolve_field_path_in_adt(deref_base.ty, &field.0)
                    .ok_or_else(|| {
                        format!("unknown field `{}` on type {:?}", field.0, deref_base.ty)
                    })?;
                self.project_field_path(deref_base, &path, field_ty, span)
            }
            Expression::Subscript(base, index) => {
                let mut base = self.lower_expr(*base, locals, local_map)?;
                self.array_to_pointer_decay_if_array(&mut base);
                let index = self.lower_expr(*index, locals, local_map)?;
                if !is_numeric_ty(index.ty) {
                    return Err(format!(
                        "subscript index must be integer, got {:?}",
                        index.ty
                    ));
                }
                let TyKind::RigidTy(RigidTy::RawPtr(pointee, _)) = base.ty.kind() else {
                    return Err(format!(
                        "subscript base must be pointer type, got {:?}",
                        base.ty
                    ));
                };
                let ptr_ty = base.ty;
                let ptr_offset = HirExpr {
                    kind: HirExprKind::PtrOffset {
                        base: Box::new(base),
                        index: Box::new(index),
                    },
                    ty: ptr_ty,
                    span,
                };
                Ok(HirExpr {
                    kind: HirExprKind::Deref(Box::new(ptr_offset)),
                    ty: pointee,
                    span,
                })
            }
            Expression::BinOp(lhs, op, rhs) => {
                if matches!(op, ParsedBinOp::Comma) {
                    let lhs = Box::new(self.lower_expr(*lhs, locals, local_map)?);
                    let rhs = Box::new(self.lower_expr(*rhs, locals, local_map)?);
                    let ty = rhs.ty;
                    return Ok(HirExpr {
                        kind: HirExprKind::Comma { lhs, rhs },
                        ty,
                        span,
                    });
                }
                if matches!(op, ParsedBinOp::Assign) {
                    let lhs = self.lower_expr(*lhs, locals, local_map)?;
                    if !is_place_expr(&lhs) {
                        return Err("assignment target is not assignable".to_owned());
                    }
                    if is_array_ty(lhs.ty) {
                        return Err("Type error - can not run binop on arrays.".to_owned());
                    }
                    let mut rhs = self.lower_expr(*rhs, locals, local_map)?;
                    self.array_to_pointer_decay_if_array(&mut rhs);
                    if needs_implicit_cast(lhs.ty, rhs.ty) {
                        rhs = HirExpr {
                            kind: HirExprKind::Cast(Box::new(rhs.clone())),
                            ty: lhs.ty,
                            span: rhs.span,
                        };
                    }
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
                self.lower_binop_from_lowered(lhs, rhs, op, span, false)
            }
            Expression::AssignWithOp { lhs, op, rhs } => {
                let lhs = self.lower_expr(*lhs, locals, local_map)?;
                if !is_place_expr(&lhs) {
                    return Err("assignment target is not assignable".to_owned());
                }
                let rhs = self.lower_expr(*rhs, locals, local_map)?;
                let ty = lhs.ty;
                let lowered = self.lower_binop_from_lowered(lhs.clone(), rhs, op, span, true)?;
                Ok(HirExpr {
                    kind: match lowered.kind {
                        HirExprKind::Binary { op, lhs, rhs } => HirExprKind::AssignWithBinOp {
                            lhs,
                            rhs,
                            op,
                            binop_ty: lowered.ty,
                            return_semantic: ReturnSemantic::AfterAssign,
                        },
                        HirExprKind::PtrOffset { base, index } => HirExprKind::AssignPtrOffset {
                            lhs: base,
                            rhs: index,
                            return_semantic: ReturnSemantic::AfterAssign,
                        },
                        _ => return Err("invalid assignment operation".to_owned()),
                    },
                    ty,
                    span,
                })
            }
            Expression::Update {
                expr,
                op,
                is_postfix,
            } => {
                let parser_span = expr.1;
                let lhs = self.lower_expr(*expr, locals, local_map)?;
                if !is_place_expr(&lhs) {
                    return Err("update target is not assignable".to_owned());
                }
                let ty = lhs.ty;
                let rhs = {
                    let (ty, kind) = match lhs.ty.kind() {
                        TyKind::RigidTy(rigid_ty) => match rigid_ty {
                            RigidTy::Int(_) | RigidTy::Uint(_) => {
                                (lhs.ty, HirExprKind::ConstInt(1))
                            }
                            RigidTy::Float(_) => (
                                Ty::from_rigid_kind(RigidTy::Float(FloatTy::F64)),
                                HirExprKind::ConstFloat(1.),
                            ),
                            RigidTy::RawPtr(..) => (Ty::usize_ty(), HirExprKind::ConstInt(1)),
                            _ => {
                                self.terminate_with_error(parser_span, "Invalid type for ++ and --")
                            }
                        },
                        _ => todo!(),
                    };
                    HirExpr { kind, ty, span }
                };
                let return_semantic = if is_postfix {
                    ReturnSemantic::BeforeAssign
                } else {
                    ReturnSemantic::AfterAssign
                };
                let bin_op = match op {
                    ParsedUpdateOp::Inc => ParsedBinOp::Add,
                    ParsedUpdateOp::Dec => ParsedBinOp::Sub,
                };

                let lowered = self.lower_update_binop(lhs.clone(), rhs, bin_op, span)?;
                match lowered.kind {
                    HirExprKind::Binary { op, lhs, rhs } => Ok(HirExpr {
                        kind: HirExprKind::AssignWithBinOp {
                            lhs,
                            rhs,
                            op,
                            binop_ty: lowered.ty,
                            return_semantic,
                        },
                        ty,
                        span,
                    }),
                    HirExprKind::PtrOffset { base, index } => Ok(HirExpr {
                        kind: HirExprKind::AssignPtrOffset {
                            lhs: base,
                            rhs: index,
                            return_semantic,
                        },
                        ty,
                        span,
                    }),
                    _ => Err("invalid update expression lowering".to_owned()),
                }
            }
            Expression::Sizeof(expr) => {
                let inner = self.lower_expr(*expr, locals, local_map)?;
                let size = inner
                    .ty
                    .layout()
                    .map_err(|e| format!("failed to compute layout for sizeof: {e}"))?
                    .shape()
                    .size
                    .bytes();
                Ok(HirExpr {
                    kind: HirExprKind::ConstInt(size as i128),
                    ty: Ty::signed_ty(IntTy::I32),
                    span,
                })
            }
            Expression::Cast { type_name, expr } => {
                let mut inner = self.lower_expr(*expr, locals, local_map)?;
                self.array_to_pointer_decay_if_array(&mut inner);
                let target_ty = self.lower_type_name(*type_name, parser_span)?;
                let src_is_int = is_numeric_ty(inner.ty);
                let dst_is_int = is_numeric_ty(target_ty);
                let src_is_ptr_like = matches!(
                    inner.ty.kind(),
                    TyKind::RigidTy(RigidTy::RawPtr(_, _) | RigidTy::FnPtr(_))
                ) || is_maybe_uninit_fn_ptr_ty(inner.ty).is_some();
                let dst_is_ptr_like = matches!(
                    target_ty.kind(),
                    TyKind::RigidTy(RigidTy::RawPtr(_, _) | RigidTy::FnPtr(_))
                ) || is_maybe_uninit_fn_ptr_ty(target_ty).is_some();
                let src_is_fn_item =
                    matches!(inner.ty.kind(), TyKind::RigidTy(RigidTy::FnDef(_, _)));
                if !((src_is_int && dst_is_int)
                    || (src_is_ptr_like && dst_is_ptr_like)
                    || (src_is_int && dst_is_ptr_like)
                    || (src_is_ptr_like && dst_is_int)
                    || (src_is_fn_item
                        && (matches!(target_ty.kind(), TyKind::RigidTy(RigidTy::FnPtr(_)))
                            || is_maybe_uninit_fn_ptr_ty(target_ty).is_some())))
                {
                    return Err(format!(
                        "unsupported cast from {:?} to {:?}",
                        inner.ty, target_ty
                    ));
                }
                Ok(HirExpr {
                    kind: HirExprKind::Cast(Box::new(inner)),
                    ty: target_ty,
                    span,
                })
            }
            Expression::CompoundLiteral {
                type_name,
                initializer,
            } => {
                let target_ty = self.lower_type_name(*type_name, parser_span)?;
                let tree =
                    self.lower_to_initializer_tree(target_ty, *initializer, locals, local_map);
                let init_expr = self.initializer_tree_to_expr(&tree, target_ty, parser_span);
                Ok(init_expr)
            }
            Expression::SizeofType(type_name) => {
                let ty = self.lower_type_name(*type_name, parser_span)?;
                let size = ty
                    .layout()
                    .map_err(|e| format!("failed to compute layout for sizeof(type): {e}"))?
                    .shape()
                    .size
                    .bytes();
                Ok(HirExpr {
                    kind: HirExprKind::ConstInt(size as i128),
                    ty: Ty::signed_ty(IntTy::I32),
                    span,
                })
            }
            Expression::UnaryOp(op, expr) => {
                let inner = self.lower_expr(*expr, locals, local_map)?;
                match op {
                    ParsedUnaryOp::AddrOf => {
                        if is_array_ty(inner.ty) {
                            return Ok(self.array_to_pointer_decay(inner));
                        }
                        if matches!(inner.ty.kind(), TyKind::RigidTy(RigidTy::FnDef(_, _))) {
                            // In C, `&f` for a function designator has function-pointer type.
                            let sig = inner
                                .ty
                                .kind()
                                .fn_sig()
                                .expect("FnDef should have fn signature");
                            return Ok(HirExpr {
                                kind: HirExprKind::Cast(Box::new(inner)),
                                ty: Ty::from_rigid_kind(RigidTy::FnPtr(sig)),
                                span,
                            });
                        }
                        if !is_place_expr(&inner) {
                            if matches!(
                                inner.kind,
                                HirExprKind::Aggregate { .. } | HirExprKind::Zeroed
                            ) {
                                return Ok(HirExpr {
                                    kind: HirExprKind::AddrOf(Box::new(inner.clone())),
                                    ty: Ty::new_ptr(inner.ty, Mutability::Mut),
                                    span,
                                });
                            }
                            self.terminate_with_error(
                                parser_span,
                                "cannot take address of non-place expression",
                            );
                        }
                        Ok(HirExpr {
                            kind: HirExprKind::AddrOf(Box::new(inner.clone())),
                            ty: Ty::new_ptr(inner.ty, Mutability::Mut),
                            span,
                        })
                    }
                    ParsedUnaryOp::Deref => {
                        let mut inner = inner;
                        self.array_to_pointer_decay_if_array(&mut inner);
                        if is_maybe_uninit_fn_ptr_ty(inner.ty).is_some() {
                            return Ok(inner);
                        }
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
                    ParsedUnaryOp::Not => {
                        if !is_condition_ty(inner.ty) {
                            return Err(format!(
                                "unary `!` expects scalar-like expression, got {:?}",
                                inner.ty
                            ));
                        }
                        Ok(HirExpr {
                            kind: HirExprKind::LogicalNot(Box::new(inner)),
                            ty: Ty::signed_ty(IntTy::I32),
                            span,
                        })
                    }
                    ParsedUnaryOp::Com => {
                        if !is_numeric_ty(inner.ty) {
                            return Err("unary `~` expects integer expression".to_owned());
                        }
                        Ok(HirExpr {
                            kind: HirExprKind::BitNot(Box::new(inner.clone())),
                            ty: inner.ty,
                            span,
                        })
                    }
                    ParsedUnaryOp::Minus => {
                        if !is_numeric_ty(inner.ty) {
                            return Err("unary `-` expects integer expression".to_owned());
                        }
                        Ok(HirExpr {
                            kind: HirExprKind::Binary {
                                op: HirBinOp::Sub,
                                lhs: Box::new(HirExpr {
                                    kind: match inner.ty.kind() {
                                        TyKind::RigidTy(RigidTy::Int(_) | RigidTy::Uint(_)) => {
                                            HirExprKind::ConstInt(0)
                                        }
                                        TyKind::RigidTy(RigidTy::Float(_)) => {
                                            HirExprKind::ConstFloat(0.)
                                        }
                                        _ => unreachable!(),
                                    },
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
            Expression::GnuStatementExpr { body } => {
                let mut parsed = body.0;
                let Some((last_item, _)) = parsed.statements.pop() else {
                    return Err(
                        "gnu statement expression requires a final expression statement".to_owned(),
                    );
                };
                let StatementOrDeclaration::Statement((Statement::Expression(tail), _)) = last_item
                else {
                    return Err(
                        "gnu statement expression final statement must be an expression".to_owned(),
                    );
                };

                let mut scoped_map = local_map.clone();
                let mut lowered_statements = Vec::new();
                for (stmt_or_decl, _) in parsed.statements {
                    match stmt_or_decl {
                        StatementOrDeclaration::Declaration((decl, _)) => {
                            self.lower_decl(decl, &mut lowered_statements, locals, &mut scoped_map)?
                        }
                        StatementOrDeclaration::Statement((stmt, span)) => self.lower_stmt(
                            stmt,
                            span,
                            &mut lowered_statements,
                            locals,
                            &mut scoped_map,
                        )?,
                    }
                }
                let tail = self.lower_expr(tail, locals, &mut scoped_map)?;
                Ok(HirExpr {
                    kind: HirExprKind::StatementExpr {
                        statements: lowered_statements,
                        tail: Box::new(tail.clone()),
                    },
                    ty: tail.ty,
                    span,
                })
            }
            Expression::VaStart { args, last_param: _ } => {
                let args = Box::new(self.lower_expr(*args, locals, local_map)?);
                Ok(HirExpr {
                    kind: HirExprKind::VaStart(args),
                    ty: Ty::new_tuple(&[]),
                    span,
                })
            },
            Expression::VaArg { args, type_name } => {
                let args = Box::new(self.lower_expr(*args, locals, local_map)?);
                let ty = self.lower_type_name(type_name, parser_span)?;
                Ok(HirExpr {
                    kind: HirExprKind::VaArg(args),
                    ty,
                    span,
                })
            },
            Expression::VaEnd { args } => {
                let args = Box::new(self.lower_expr(*args, locals, local_map)?);
                Ok(HirExpr {
                    kind: HirExprKind::VaEnd(args),
                    ty: Ty::new_tuple(&[]),
                    span,
                })
            },
            Expression::Empty => Err("empty expression is invalid here".to_owned()),
            Expression::Conditional {
                cond,
                then_expr,
                else_expr,
            } => {
                let cond = self.lower_expr(*cond, locals, local_map)?;
                let mut then_expr = self.lower_expr(*then_expr, locals, local_map)?;
                let mut else_expr = self.lower_expr(*else_expr, locals, local_map)?;

                self.array_to_pointer_decay_if_array(&mut then_expr);
                self.array_to_pointer_decay_if_array(&mut else_expr);

                let Some(common_ty) = common_ternary_ty(then_expr.ty, else_expr.ty) else {
                    self.terminate_with_error(
                        parser_span,
                        &format!(
                            "ternary operator branches have mismatched types: {:?} vs {:?}",
                            then_expr.ty, else_expr.ty,
                        ),
                    );
                };

                Ok(HirExpr {
                    kind: HirExprKind::Conditional {
                        cond: Box::new(cond),
                        then_expr: Box::new(self.emit_cast(then_expr, common_ty)),
                        else_expr: Box::new(self.emit_cast(else_expr, common_ty)),
                    },
                    ty: common_ty,
                    span,
                })
            }
        }
    }

    fn project_field_path(
        &self,
        mut base: HirExpr,
        path: &[usize],
        field_ty: Ty,
        span: RustSpan,
    ) -> Result<HirExpr, String> {
        for index in path {
            let Some(field_tys) = adt_field_tys(base.ty) else {
                return Err(format!("field projection on non-adt type: {:?}", base.ty));
            };
            let Some(next_ty) = field_tys.get(*index).copied() else {
                return Err(format!(
                    "field index out of bounds: {} for {:?}",
                    index, base.ty
                ));
            };
            base = HirExpr {
                kind: HirExprKind::Field {
                    base: Box::new(base),
                    index: *index,
                },
                ty: next_ty,
                span,
            };
        }
        if base.ty != field_ty {
            return Err(format!(
                "resolved field type mismatch: projected {:?}, expected {:?}",
                base.ty, field_ty
            ));
        }
        Ok(base)
    }
}

fn ty_passed_to_variadic(ty: Ty) -> Ty {
    match ty.kind() {
        TyKind::RigidTy(rigid_ty) => {
            let rigid_ty = match rigid_ty {
                RigidTy::Int(int_ty) => RigidTy::Int(match int_ty {
                    IntTy::I8 => IntTy::I32,
                    IntTy::I16 => IntTy::I32,
                    IntTy::I32 => IntTy::I32,
                    IntTy::I64 => IntTy::I64,
                    IntTy::Isize => IntTy::Isize,
                    IntTy::I128 => IntTy::I128,
                }),
                RigidTy::Uint(uint_ty) => RigidTy::Uint(uint_ty),
                RigidTy::Float(float_ty) => RigidTy::Float(match float_ty {
                    FloatTy::F16 => FloatTy::F64,
                    FloatTy::F32 => FloatTy::F64,
                    FloatTy::F64 => FloatTy::F64,
                    FloatTy::F128 => FloatTy::F128,
                }),
                _ => rigid_ty,
            };
            Ty::from_rigid_kind(rigid_ty)
        }
        _ => ty,
    }
}

impl HirCtx<'_> {
    fn lower_binop_from_lowered(
        &self,
        mut lhs: HirExpr,
        mut rhs: HirExpr,
        op: ParsedBinOp,
        span: RustSpan,
        is_assignment: bool,
    ) -> Result<HirExpr, String> {
        let op = match op {
            ParsedBinOp::Comma | ParsedBinOp::Assign => unreachable!(),
            ParsedBinOp::Add => HirBinOp::Add,
            ParsedBinOp::Sub => HirBinOp::Sub,
            ParsedBinOp::Mul => HirBinOp::Mul,
            ParsedBinOp::Div => HirBinOp::Div,
            ParsedBinOp::Rem => HirBinOp::Rem,
            ParsedBinOp::BitOr => HirBinOp::BitOr,
            ParsedBinOp::BitXor => HirBinOp::BitXor,
            ParsedBinOp::BitAnd => HirBinOp::BitAnd,
            ParsedBinOp::Eq => HirBinOp::Eq,
            ParsedBinOp::Lt => HirBinOp::Lt,
            ParsedBinOp::Le => HirBinOp::Le,
            ParsedBinOp::Ne => HirBinOp::Ne,
            ParsedBinOp::Ge => HirBinOp::Ge,
            ParsedBinOp::Gt => HirBinOp::Gt,
            ParsedBinOp::Shl => HirBinOp::Shl,
            ParsedBinOp::Shr => HirBinOp::Shr,
            ParsedBinOp::And | ParsedBinOp::Or => {
                let logical_op = match op {
                    ParsedBinOp::And => HirLogicalOp::And,
                    ParsedBinOp::Or => HirLogicalOp::Or,
                    _ => unreachable!(),
                };
                return Ok(HirExpr {
                    kind: HirExprKind::Logical {
                        op: logical_op,
                        lhs: Box::new(lhs),
                        rhs: Box::new(rhs),
                    },
                    ty: Ty::signed_ty(IntTy::I32),
                    span,
                });
            }
        };

        if is_assignment && is_array_ty(lhs.ty) {
            return Err("Type error - can not run binop on arrays.".to_owned());
        } else {
            self.array_to_pointer_decay_if_array(&mut lhs);
        }
        self.array_to_pointer_decay_if_array(&mut rhs);

        if matches!(op, HirBinOp::Add | HirBinOp::Sub) {
            let lhs_is_ptr = matches!(lhs.ty.kind(), TyKind::RigidTy(RigidTy::RawPtr(_, _)));
            let rhs_is_ptr = matches!(rhs.ty.kind(), TyKind::RigidTy(RigidTy::RawPtr(_, _)));
            if matches!(op, HirBinOp::Add) {
                match (lhs_is_ptr, rhs_is_ptr) {
                    (true, false) if is_numeric_ty(rhs.ty) => {
                        return Ok(HirExpr {
                            kind: HirExprKind::PtrOffset {
                                base: Box::new(lhs.clone()),
                                index: Box::new(rhs),
                            },
                            ty: lhs.ty,
                            span,
                        });
                    }
                    (false, true) if is_numeric_ty(lhs.ty) && !is_assignment => {
                        return Ok(HirExpr {
                            kind: HirExprKind::PtrOffset {
                                base: Box::new(rhs.clone()),
                                index: Box::new(lhs),
                            },
                            ty: rhs.ty,
                            span,
                        });
                    }
                    (true, true) => {
                        return Err("type error: adding two pointers is not supported".to_owned());
                    }
                    _ => {}
                }
            } else {
                match (lhs_is_ptr, rhs_is_ptr) {
                    (true, false) if is_numeric_ty(rhs.ty) => {
                        let rhs_ty = rhs.ty;
                        let neg_rhs = HirExpr {
                            kind: HirExprKind::Binary {
                                op: HirBinOp::Sub,
                                lhs: Box::new(HirExpr {
                                    kind: HirExprKind::ConstInt(0),
                                    ty: rhs_ty,
                                    span,
                                }),
                                rhs: Box::new(rhs),
                            },
                            ty: rhs_ty,
                            span,
                        };
                        return Ok(HirExpr {
                            kind: HirExprKind::PtrOffset {
                                base: Box::new(lhs.clone()),
                                index: Box::new(neg_rhs),
                            },
                            ty: lhs.ty,
                            span,
                        });
                    }
                    (true, true) if !is_assignment => {
                        return Ok(HirExpr {
                            kind: HirExprKind::PtrDiff {
                                lhs: Box::new(lhs),
                                rhs: Box::new(rhs),
                            },
                            ty: Ty::signed_ty(IntTy::Isize),
                            span,
                        });
                    }
                    _ => {}
                }
            }
        }

        if is_numeric_ty(lhs.ty) && is_numeric_ty(rhs.ty) {
            if lhs.ty != rhs.ty {
                if let Some(common_ty) = common_numeric_ty(lhs.ty, rhs.ty) {
                    if !is_assignment {
                        lhs = HirExpr {
                            kind: HirExprKind::Cast(Box::new(lhs.clone())),
                            ty: common_ty,
                            span: lhs.span,
                        };
                    }
                    rhs = HirExpr {
                        kind: HirExprKind::Cast(Box::new(rhs.clone())),
                        ty: common_ty,
                        span: rhs.span,
                    };
                }
            }
        }

        if op.is_comparison() && lhs.ty != rhs.ty {
            let common_ty = Ty::usize_ty();
            lhs = HirExpr {
                kind: HirExprKind::Cast(Box::new(lhs.clone())),
                ty: common_ty,
                span: lhs.span,
            };
            rhs = HirExpr {
                kind: HirExprKind::Cast(Box::new(rhs.clone())),
                ty: common_ty,
                span: rhs.span,
            };
        }

        if lhs.ty != rhs.ty && !is_assignment {
            return Err(format!(
                "binary op type mismatch: lhs={:?}, rhs={:?}",
                lhs.ty, rhs.ty
            ));
        }

        let ty = if op.is_comparison() {
            Ty::signed_ty(IntTy::I32)
        } else {
            rhs.ty
        };
        Ok(HirExpr {
            kind: HirExprKind::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            },
            ty,
            span,
        })
    }

    fn lower_update_binop(
        &self,
        lhs: HirExpr,
        rhs: HirExpr,
        op: ParsedBinOp,
        span: RustSpan,
    ) -> Result<HirExpr, String> {
        self.lower_binop_from_lowered(lhs, rhs, op, span, true)
    }

    pub(crate) fn initializer_tree_to_expr(
        &self,
        tree: &InitializerTree,
        ty: Ty,
        parser_span: co2_ast::Span,
    ) -> HirExpr {
        let span = self.to_rust_span(parser_span);
        match tree {
            InitializerTree::Leaf(expr) => expr.clone(),
            InitializerTree::Zeroed => HirExpr {
                kind: HirExprKind::Zeroed,
                ty,
                span,
            },
            InitializerTree::Middle { children } => {
                if let Some(elem_ty) = array_elem_ty(ty) {
                    let mut args = Vec::with_capacity(children.len());
                    for child in children {
                        let expr = self.initializer_tree_to_expr(child, elem_ty, parser_span);
                        args.push(expr);
                    }
                    HirExpr {
                        kind: HirExprKind::Aggregate { args },
                        ty,
                        span,
                    }
                } else {
                    let Some(field_tys) = adt_field_tys(ty) else {
                        self.terminate_with_error(parser_span, "Can't compute adt fields");
                    };
                    let mut args = Vec::with_capacity(children.len());
                    for (child, field_ty) in children.iter().zip(field_tys) {
                        let expr = self.initializer_tree_to_expr(child, field_ty, parser_span);
                        args.push(expr);
                    }
                    HirExpr {
                        kind: HirExprKind::Aggregate { args },
                        ty,
                        span,
                    }
                }
            }
        }
    }
}

pub(crate) fn is_place_expr(expr: &HirExpr) -> bool {
    match &expr.kind {
        HirExprKind::Local(_) => true,
        HirExprKind::Field { base, .. } => is_place_expr(base),
        HirExprKind::Path(ResolvedValue::Static { .. }) => true,
        HirExprKind::PtrOffset { .. } => false,
        HirExprKind::PtrDiff { .. } => false,
        HirExprKind::Logical { .. } => false,
        HirExprKind::LogicalNot(_) => false,
        HirExprKind::BitNot(_) => false,
        HirExprKind::Deref(_) => true,
        _ => false,
    }
}

fn int_suffix_ty(suffix: IntegerSuffix, value: i128) -> Ty {
    match suffix {
        IntegerSuffix::None => {
            let value = value.abs();
            if value <= (i32::MAX as i128) {
                Ty::signed_ty(IntTy::I32)
            } else if value <= (u32::MAX as i128) {
                Ty::unsigned_ty(UintTy::U32)
            } else if value <= (i64::MAX as i128) {
                Ty::signed_ty(IntTy::I64)
            } else if value <= (u64::MAX as i128) {
                Ty::unsigned_ty(UintTy::U64)
            } else {
                Ty::signed_ty(IntTy::I128)
            }
        },
        IntegerSuffix::Long | IntegerSuffix::LongLong => Ty::signed_ty(IntTy::I64),
        IntegerSuffix::Unsigned => Ty::unsigned_ty(UintTy::U32),
        IntegerSuffix::UnsignedLong | IntegerSuffix::UnsignedLongLong => {
            Ty::unsigned_ty(UintTy::U64)
        }
    }
}

pub(crate) fn coerce_expr_to_type(expr: HirExpr, expected_ty: Ty) -> Result<HirExpr, String> {
    if expr.ty == expected_ty {
        return Ok(expr);
    }
    if needs_implicit_cast(expected_ty, expr.ty) {
        return Ok(HirExpr {
            kind: HirExprKind::Cast(Box::new(expr.clone())),
            ty: expected_ty,
            span: expr.span,
        });
    }
    Err(format!(
        "initializer type mismatch: expected {expected_ty:?}, got {:?}",
        expr.ty
    ))
}
