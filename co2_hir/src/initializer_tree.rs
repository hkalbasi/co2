use std::collections::HashMap;

use co2_ast::{Designator, Expression, Initializer, InitializerItem, Spanned};
use co2_crate_sig::{LocalResolver, eval_const_expr_as_usize};
use la_arena::Arena;
use rustc_public_generative::rustc_public::ty::{AdtKind, RigidTy, Ty, TyKind};

use crate::{
    expr::{HirExpr, HirExprKind, coerce_expr_to_type},
    item::{HirLocal, LocalId},
    resolver::HirCtx,
    ty::{adt_field_tys, array_elem_ty, is_array_ty, is_union_ty, resolve_field_path_in_adt},
};

#[derive(Clone, Debug)]
pub(crate) enum InitializerTree {
    Middle { children: Vec<InitializerTree> },
    Leaf(HirExpr),
    Zeroed,
}

#[derive(Debug)]
struct InitializerCursor {
    base_ty: Ty,
    stack: Vec<(usize, Ty)>,
}

impl InitializerCursor {
    fn from_designators(
        ctx: &HirCtx<'_>,
        designators: &[Spanned<Designator<LocalResolver>>],
        base_ty: Ty,
        span: rustc_public_generative::rustc_public::ty::Span,
    ) -> Result<Self, String> {
        let mut current_ty = base_ty;
        let mut cursor = InitializerCursor {
            base_ty,
            stack: vec![],
        };
        for (designator, span) in designators {
            match designator {
                Designator::Subscript(expr) => {
                    let idx = eval_const_expr_as_usize(&ctx.decl_resolver, expr)?;
                    let elem_ty = array_elem_ty(current_ty).ok_or_else(|| {
                        format!("array designator used on non-array type: {:?}", current_ty)
                    })?;
                    cursor.stack.push((idx, elem_ty));
                    current_ty = elem_ty;
                }
                Designator::Range(_, _) => {
                    ctx.terminate_with_error(*span, "unsupported GNU range designator");
                }
                Designator::Field(name) => {
                    let (path, field_ty) = resolve_field_path_in_adt(current_ty, name.0.as_str())
                        .ok_or_else(|| {
                        format!(
                            "field designator `{}` used on non-struct type: {:?}",
                            name.0, current_ty
                        )
                    })?;
                    let mut cursor_ty = current_ty;
                    for index in path {
                        let fields = adt_field_tys(cursor_ty).ok_or_else(|| {
                            format!("designator on non-adt type: {:?}", cursor_ty)
                        })?;
                        let next_ty = *fields.get(index).ok_or_else(|| {
                            format!(
                                "designator field index out of bounds: {} for {:?}",
                                index, cursor_ty
                            )
                        })?;
                        cursor.stack.push((index, next_ty));
                        cursor_ty = next_ty;
                    }
                    assert_eq!(cursor_ty, field_ty);
                }
            }
        }
        if cursor.stack.is_empty() {
            return Err("empty designator list is invalid".to_owned());
        }
        let _ = span;
        Ok(cursor)
    }

    fn ty(&self) -> Ty {
        self.stack.last().map(|(_, ty)| *ty).unwrap_or(self.base_ty)
    }

    fn insert_to_tree(&self, result: &mut InitializerTree, value: InitializerTree) {
        if self.stack.is_empty() {
            return;
        }
        let mut current = result;
        let mut prev_ty = self.base_ty;
        for (index, ty) in &self.stack {
            let children = current.children(prev_ty);
            while children.len() <= *index {
                children.push(InitializerTree::Zeroed);
            }
            current = &mut children[*index];
            prev_ty = *ty;
        }
        *current = value;
    }

    fn go_through(&mut self) -> bool {
        if self.stack.is_empty() {
            return false;
        }
        let ty = self.ty();
        if let Some(fields) = adt_field_tys(ty) {
            if fields.is_empty() {
                todo!()
            }
            self.stack.push((0, fields[0]));
            return true;
        }
        if let Some(elem) = array_elem_ty(ty) {
            self.stack.push((0, elem));
            return true;
        }
        false
    }

    fn go_next(
        &mut self,
        span: rustc_public_generative::rustc_public::ty::Span,
    ) -> Result<(), String> {
        if self.stack.is_empty() {
            return Ok(());
        }
        let (mut idx, _) = self.stack.pop().expect("stack not empty");
        let parent_ty = self.stack.last().map(|(_, ty)| *ty).unwrap_or(self.base_ty);

        if let Some(fields) = adt_field_tys(parent_ty) {
            if is_union_ty(parent_ty) {
                // A union consumes exactly one initializer slot.
                self.go_next(span)?;
                return Ok(());
            }
            idx += 1;
            if idx < fields.len() {
                self.stack.push((idx, fields[idx]));
            } else {
                self.go_next(span)?;
            }
            return Ok(());
        }

        if is_array_ty(parent_ty) {
            let elem_ty = array_elem_ty(parent_ty).expect("array elem type");
            let len = array_len_from_layout(parent_ty).ok_or_else(|| {
                format!("unable to infer array length for initializer from layout: {parent_ty:?}")
            })?;
            idx += 1;
            if idx < len {
                self.stack.push((idx, elem_ty));
            } else {
                self.go_next(span)?;
            }
            return Ok(());
        }

        Err(format!(
            "invalid initializer cursor parent type: {parent_ty:?} at {span:?}"
        ))
    }
}

impl InitializerTree {
    fn children(&mut self, ty: Ty) -> &mut Vec<InitializerTree> {
        match self {
            InitializerTree::Middle { children } => children,
            InitializerTree::Leaf(_) => panic!("leaf does not have children"),
            InitializerTree::Zeroed => {
                let count = children_count_of_ty(ty);
                *self = InitializerTree::Middle {
                    children: vec![InitializerTree::Zeroed; count],
                };
                let InitializerTree::Middle { children } = self else {
                    unreachable!();
                };
                children
            }
        }
    }
}

fn children_count_of_ty(ty: Ty) -> usize {
    let count = match ty.kind() {
        TyKind::RigidTy(rigid_ty) => match rigid_ty {
            RigidTy::Array(_, ty_const) => ty_const.eval_target_usize().unwrap() as usize,
            RigidTy::Adt(def, _) => match def.kind() {
                AdtKind::Struct => adt_field_tys(ty).unwrap().len(),
                _ => 1,
            },
            _ => panic!("Can't go through primitive ty {ty}"),
        },
        _ => todo!(),
    };
    if count == 567_567 { 0 } else { count }
}

pub(crate) fn eval_const_int(expr: &HirExpr) -> Result<i128, String> {
    match &expr.kind {
        HirExprKind::ConstInt(v) => Ok(*v),
        HirExprKind::Path(crate::ResolvedValue::ConstInt(v)) => Ok(*v),
        HirExprKind::Binary { op, lhs, rhs } => {
            let lhs = eval_const_int(lhs)?;
            let rhs = eval_const_int(rhs)?;
            match op {
                crate::expr::HirBinOp::Add => Ok(lhs + rhs),
                crate::expr::HirBinOp::Sub => Ok(lhs - rhs),
                crate::expr::HirBinOp::Mul => Ok(lhs * rhs),
                crate::expr::HirBinOp::Div => lhs
                    .checked_div(rhs)
                    .ok_or_else(|| "constant integer expression division failed".to_owned()),
                crate::expr::HirBinOp::Rem => lhs
                    .checked_rem(rhs)
                    .ok_or_else(|| "constant integer expression remainder failed".to_owned()),
                crate::expr::HirBinOp::BitOr => Ok(lhs | rhs),
                crate::expr::HirBinOp::BitXor => Ok(lhs ^ rhs),
                crate::expr::HirBinOp::BitAnd => Ok(lhs & rhs),
                crate::expr::HirBinOp::Eq => Ok((lhs == rhs) as i128),
                crate::expr::HirBinOp::Lt => Ok((lhs < rhs) as i128),
                crate::expr::HirBinOp::Le => Ok((lhs <= rhs) as i128),
                crate::expr::HirBinOp::Ne => Ok((lhs != rhs) as i128),
                crate::expr::HirBinOp::Ge => Ok((lhs >= rhs) as i128),
                crate::expr::HirBinOp::Gt => Ok((lhs > rhs) as i128),
                crate::expr::HirBinOp::Shl => Ok(lhs << rhs),
                crate::expr::HirBinOp::Shr => Ok(lhs >> rhs),
            }
        }
        HirExprKind::Comma { rhs, .. } => eval_const_int(rhs),
        HirExprKind::Logical { op, lhs, rhs } => {
            let lhs = eval_const_int(lhs)? != 0;
            let rhs = eval_const_int(rhs)? != 0;
            Ok(match op {
                crate::expr::HirLogicalOp::Or => (lhs || rhs) as i128,
                crate::expr::HirLogicalOp::And => (lhs && rhs) as i128,
            })
        }
        HirExprKind::LogicalNot(inner) => Ok((eval_const_int(inner)? == 0) as i128),
        HirExprKind::BitNot(inner) => Ok(!eval_const_int(inner)?),
        HirExprKind::Cast(inner) => eval_const_int(inner),
        HirExprKind::Conditional {
            cond,
            then_expr,
            else_expr,
        } => {
            if eval_const_int(cond)? != 0 {
                eval_const_int(then_expr)
            } else {
                eval_const_int(else_expr)
            }
        }
        _ => Err("designator subscript must be constant integer expression".to_owned()),
    }
}

fn array_len_from_layout(ty: Ty) -> Option<usize> {
    let elem = array_elem_ty(ty)?;
    let total = ty.layout().ok()?.shape().size.bytes();
    let elem_sz = elem.layout().ok()?.shape().size.bytes();
    if elem_sz == 0 {
        return None;
    }
    Some((total / elem_sz) as usize)
}

impl HirCtx<'_> {
    pub(crate) fn lower_to_initializer_tree(
        &self,
        expected_ty: Ty,
        (initializer, span): Spanned<Initializer<LocalResolver>>,
        locals: &mut Arena<HirLocal>,
        local_map: &mut HashMap<usize, LocalId>,
    ) -> InitializerTree {
        match self.try_lower_to_initializer_tree(
            expected_ty,
            (initializer, span),
            locals,
            local_map,
        ) {
            Ok(r) => r,
            Err(e) => {
                self.terminate_with_error(span, &e);
            }
        }
    }

    fn try_lower_to_initializer_tree(
        &self,
        expected_ty: Ty,
        initializer: Spanned<Initializer<LocalResolver>>,
        locals: &mut Arena<HirLocal>,
        local_map: &mut HashMap<usize, LocalId>,
    ) -> Result<InitializerTree, String> {
        let span = self.to_rust_span(initializer.1);
        match initializer.0 {
            Initializer::Expr(expr) => {
                // C string literal can initialize char arrays.
                if is_array_ty(expected_ty)
                    && matches!(expr.0, Expression::Constant(co2_ast::Constant::String(_)))
                {
                    let list = self.initializer_list_from_string(expected_ty, expr.clone());
                    return Ok(self.lower_to_initializer_tree(
                        expected_ty,
                        (Initializer::List(list), initializer.1),
                        locals,
                        local_map,
                    ));
                }
                let expr = self.lower_expr(expr, locals, local_map)?;
                let coerced = coerce_expr_to_type(expr, expected_ty)?;
                Ok(InitializerTree::Leaf(coerced))
            }
            Initializer::List(items) => {
                if adt_field_tys(expected_ty).is_none() && !is_array_ty(expected_ty) {
                    let first = items
                        .into_iter()
                        .next()
                        .ok_or_else(|| "empty initializer list for scalar type".to_owned())?;
                    if first.0.designators.is_some() {
                        return Err("designators are invalid for scalar initializer".to_owned());
                    }
                    return Ok(self.lower_to_initializer_tree(
                        expected_ty,
                        first.0.initializer,
                        locals,
                        local_map,
                    ));
                }

                let mut result = InitializerTree::Middle {
                    children: vec![InitializerTree::Zeroed; children_count_of_ty(expected_ty)],
                };
                let mut cursor = if adt_field_tys(expected_ty).is_some() || is_array_ty(expected_ty)
                {
                    let mut c = InitializerCursor {
                        base_ty: expected_ty,
                        stack: vec![],
                    };
                    if let Some(fields) = adt_field_tys(expected_ty) {
                        if !fields.is_empty() {
                            c.stack.push((0, fields[0]));
                        }
                    } else if is_array_ty(expected_ty) {
                        let elem = array_elem_ty(expected_ty).expect("array elem");
                        c.stack.push((0, elem));
                    }
                    c
                } else {
                    return Err(format!(
                        "invalid initializer list target type: {expected_ty:?}"
                    ));
                };

                for (item, item_span) in items {
                    if let Some(designators) = &item.designators {
                        cursor = InitializerCursor::from_designators(
                            self,
                            designators,
                            expected_ty,
                            self.to_rust_span(item_span),
                        )?;
                    }
                    if cursor.stack.is_empty() {
                        // Overflowing item, emit warning
                        continue;
                    }
                    let node = if let Initializer::Expr(expr) = item.initializer.0 {
                        if matches!(expr.0, Expression::Constant(co2_ast::Constant::String(_))) {
                            loop {
                                let terminal = match cursor.ty().kind() {
                                    TyKind::RigidTy(rigid_ty) => match rigid_ty {
                                        RigidTy::Adt(..) => false,
                                        RigidTy::Array(inner, _) => inner.kind().is_primitive(),
                                        _ => true,
                                    },
                                    _ => false,
                                };
                                if terminal {
                                    break self.lower_to_initializer_tree(
                                        cursor.ty(),
                                        (Initializer::Expr(expr), item_span),
                                        locals,
                                        local_map,
                                    );
                                }
                                if !cursor.go_through() {
                                    self.terminate_with_error(
                                        item_span,
                                        "failed to lower string literal as initializer tree",
                                    );
                                }
                            }
                        } else {
                            let mut expr = self.lower_expr(expr, locals, local_map)?;
                            self.array_to_pointer_decay_if_array(&mut expr);
                            loop {
                                if let Ok(coerced) = coerce_expr_to_type(expr.clone(), cursor.ty())
                                {
                                    break InitializerTree::Leaf(coerced);
                                }
                                if !cursor.go_through() {
                                    self.terminate_with_error(
                                        item_span,
                                        "failed to lower initializer tree",
                                    );
                                }
                            }
                        }
                    } else {
                        self.lower_to_initializer_tree(
                            cursor.ty(),
                            item.initializer,
                            locals,
                            local_map,
                        )
                    };
                    cursor.insert_to_tree(&mut result, node);
                    cursor.go_next(span)?;
                }
                Ok(result)
            }
        }
    }

    fn initializer_list_from_string(
        &self,
        expected_ty: Ty,
        expr: Spanned<Expression<LocalResolver>>,
    ) -> Vec<Spanned<InitializerItem<LocalResolver>>> {
        let Expression::Constant(co2_ast::Constant::String(s)) = expr.0 else {
            return vec![];
        };
        let span = expr.1;
        let items = s
            .chars()
            .chain(['\0'])
            .map(|ch| {
                (
                    InitializerItem {
                        designators: None,
                        initializer: (
                            Initializer::Expr((
                                Expression::Constant(co2_ast::Constant::Int(
                                    ch as i128,
                                    co2_ast::IntegerSuffix::None,
                                )),
                                span,
                            )),
                            span,
                        ),
                    },
                    span,
                )
            })
            .collect::<Vec<_>>();
        let _ = expected_ty;
        items
    }
}
