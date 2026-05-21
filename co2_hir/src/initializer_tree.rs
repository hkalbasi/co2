use std::collections::HashMap;

use co2_ast::{Designator, Expression, Initializer, InitializerItem, Spanned};
use co2_crate_sig::LocalResolver;
use la_arena::Arena;
use rustc_public_generative::rustc_public::ty::{AdtKind, IntTy, RigidTy, Ty, TyKind, UintTy};

use crate::{
    expr::{HirExpr, HirExprKind, coerce_expr_to_type},
    item::{HirLocal, LocalId},
    resolver::HirCtx,
    ty::{adt_field_tys, array_elem_ty, is_array_ty, is_union_ty},
};

fn spanned_error(span: co2_ast::Span, msg: impl Into<String>) -> (co2_ast::Span, String) {
    (span, msg.into())
}

fn invalid_span() -> co2_ast::Span {
    co2_ast::Span {
        start: 0,
        end: 0,
        context: co2_ast::FileId::INVALID,
    }
}

#[derive(Clone, Debug)]
pub(crate) enum InitializerTree {
    Middle { children: Vec<InitializerTree> },
    Leaf(HirExpr),
    Zeroed,
}

#[derive(Clone, Debug)]
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
        locals: &mut Arena<HirLocal>,
        local_map: &mut HashMap<usize, LocalId>,
    ) -> Result<Self, (co2_ast::Span, String)> {
        let mut current_ty = base_ty;
        let mut cursor = InitializerCursor {
            base_ty,
            stack: vec![],
        };
        for (designator, span) in designators {
            match designator {
                Designator::Subscript(expr) => {
                    let idx = ctx.eval_array_len_expr_in_scope(expr, locals, local_map)?;
                    let elem_ty = array_elem_ty(current_ty).ok_or_else(|| {
                        spanned_error(
                            *span,
                            format!("array designator used on non-array type: {current_ty:?}"),
                        )
                    })?;
                    cursor.stack.push((idx, elem_ty));
                    current_ty = elem_ty;
                }
                Designator::Range(_, _) => {
                    ctx.terminate_with_error(*span, "unsupported GNU range designator");
                }
                Designator::Field(name) => {
                    let (path, field_ty) = ctx
                        .resolve_logical_field_path(current_ty, name.0.as_str())
                        .ok_or_else(|| {
                            spanned_error(
                                *span,
                                format!(
                                    "field designator `{}` used on non-struct type: {:?}",
                                    name.0, current_ty
                                ),
                            )
                        })?;
                    let mut cursor_ty = current_ty;
                    for index in path {
                        let fields = ctx.adt_logical_field_tys(cursor_ty).ok_or_else(|| {
                            spanned_error(
                                *span,
                                format!("designator on non-adt type: {cursor_ty:?}"),
                            )
                        })?;
                        let next_ty = *fields.get(index).ok_or_else(|| {
                            spanned_error(
                                *span,
                                format!(
                                    "designator field index out of bounds: {index} for {cursor_ty:?}"
                                ),
                            )
                        })?;
                        cursor.stack.push((index, next_ty));
                        cursor_ty = next_ty;
                    }
                    assert_eq!(cursor_ty, field_ty);
                    current_ty = field_ty;
                }
            }
        }
        if cursor.stack.is_empty() {
            return Err(spanned_error(
                invalid_span(),
                "empty designator list is invalid",
            ));
        }
        let _ = span;
        Ok(cursor)
    }

    fn ty(&self) -> Ty {
        self.stack.last().map_or(self.base_ty, |(_, ty)| *ty)
    }

    fn insert_to_tree(
        &self,
        ctx: &HirCtx<'_>,
        result: &mut InitializerTree,
        value: InitializerTree,
    ) {
        if self.stack.is_empty() {
            return;
        }
        let mut current = result;
        let mut prev_ty = self.base_ty;
        for (index, ty) in &self.stack {
            let children = current.children(ctx, prev_ty);
            while children.len() <= *index {
                children.push(InitializerTree::Zeroed);
            }
            current = &mut children[*index];
            prev_ty = *ty;
        }
        *current = value;
    }

    fn go_through(&mut self, ctx: &HirCtx<'_>) -> bool {
        if self.stack.is_empty() {
            return false;
        }
        let ty = self.ty();
        if let Some(fields) = ctx.adt_logical_field_tys(ty) {
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
        ctx: &HirCtx<'_>,
        span: rustc_public_generative::rustc_public::ty::Span,
    ) -> Result<(), (co2_ast::Span, String)> {
        if self.stack.is_empty() {
            return Ok(());
        }
        let (mut idx, _) = self.stack.pop().expect("stack not empty");
        let parent_ty = self.stack.last().map_or(self.base_ty, |(_, ty)| *ty);

        if let Some(fields) = ctx.adt_logical_field_tys(parent_ty) {
            if is_union_ty(parent_ty) {
                // A union consumes exactly one initializer slot.
                self.go_next(ctx, span)?;
                return Ok(());
            }
            idx += 1;
            if idx < fields.len() {
                self.stack.push((idx, fields[idx]));
            } else {
                self.go_next(ctx, span)?;
            }
            return Ok(());
        }

        if is_array_ty(parent_ty) {
            let elem_ty = array_elem_ty(parent_ty).expect("array elem type");
            let len = array_len_from_layout(parent_ty).ok_or_else(|| {
                spanned_error(
                    invalid_span(),
                    format!(
                        "unable to infer array length for initializer from layout: {parent_ty:?}"
                    ),
                )
            })?;
            idx += 1;
            if idx < len {
                self.stack.push((idx, elem_ty));
            } else {
                self.go_next(ctx, span)?;
            }
            return Ok(());
        }

        Err(spanned_error(
            invalid_span(),
            format!("invalid initializer cursor parent type: {parent_ty:?} at {span:?}"),
        ))
    }
}

impl InitializerTree {
    fn children(&mut self, ctx: &HirCtx<'_>, ty: Ty) -> &mut Vec<InitializerTree> {
        match self {
            InitializerTree::Middle { children } => children,
            InitializerTree::Leaf(_) => panic!("leaf does not have children"),
            InitializerTree::Zeroed => {
                let count = children_count_of_ty(ctx, ty);
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

fn children_count_of_ty(ctx: &HirCtx<'_>, ty: Ty) -> usize {
    let count = match ty.kind() {
        TyKind::RigidTy(rigid_ty) => match rigid_ty {
            RigidTy::Array(_, ty_const) => ty_const.eval_target_usize().unwrap() as usize,
            RigidTy::Adt(def, _) => match def.kind() {
                AdtKind::Struct => ctx
                    .adt_logical_field_tys(ty)
                    .unwrap_or_else(|| adt_field_tys(ty).unwrap())
                    .len(),
                _ => 1,
            },
            _ => panic!("Can't go through primitive ty {ty}"),
        },
        _ => todo!(),
    };
    if count == 567_567 { 0 } else { count }
}

pub(crate) fn eval_const_int(expr: &HirExpr) -> Option<i128> {
    match &expr.kind {
        HirExprKind::ConstInt(v) | HirExprKind::Path(crate::ResolvedValue::ConstInt(v)) => Some(*v),
        HirExprKind::Binary { op, lhs, rhs } => {
            let lhs = eval_const_int(lhs)?;
            let rhs = eval_const_int(rhs)?;
            match op {
                crate::expr::HirBinOp::Add => Some(lhs + rhs),
                crate::expr::HirBinOp::Sub => Some(lhs - rhs),
                crate::expr::HirBinOp::Mul => Some(lhs * rhs),
                crate::expr::HirBinOp::Div => lhs.checked_div(rhs),
                crate::expr::HirBinOp::Rem => lhs.checked_rem(rhs),
                crate::expr::HirBinOp::BitOr => Some(lhs | rhs),
                crate::expr::HirBinOp::BitXor => Some(lhs ^ rhs),
                crate::expr::HirBinOp::BitAnd => Some(lhs & rhs),
                crate::expr::HirBinOp::Eq => Some(i128::from(lhs == rhs)),
                crate::expr::HirBinOp::Lt => Some(i128::from(lhs < rhs)),
                crate::expr::HirBinOp::Le => Some(i128::from(lhs <= rhs)),
                crate::expr::HirBinOp::Ne => Some(i128::from(lhs != rhs)),
                crate::expr::HirBinOp::Ge => Some(i128::from(lhs >= rhs)),
                crate::expr::HirBinOp::Gt => Some(i128::from(lhs > rhs)),
                crate::expr::HirBinOp::Shl => Some(lhs << rhs),
                crate::expr::HirBinOp::Shr => Some(lhs >> rhs),
            }
        }
        HirExprKind::Comma { rhs, .. } => eval_const_int(rhs),
        HirExprKind::Logical { op, lhs, rhs } => {
            let lhs = eval_const_int(lhs)? != 0;
            let rhs = eval_const_int(rhs)? != 0;
            Some(match op {
                crate::expr::HirLogicalOp::Or => i128::from(lhs || rhs),
                crate::expr::HirLogicalOp::And => i128::from(lhs && rhs),
            })
        }
        HirExprKind::LogicalNot(inner) => Some(i128::from(eval_const_int(inner)? == 0)),
        HirExprKind::BitNot(inner) => Some(!eval_const_int(inner)?),
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
        _ => None,
    }
}

fn array_len_from_layout(ty: Ty) -> Option<usize> {
    let elem = array_elem_ty(ty)?;
    let total = ty.layout().ok()?.shape().size.bytes();
    let elem_sz = elem.layout().ok()?.shape().size.bytes();
    if elem_sz == 0 {
        return None;
    }
    Some(total / elem_sz)
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
            Err(err) => self.terminate_with_spanned_error(err),
        }
    }

    fn try_lower_to_initializer_tree(
        &self,
        expected_ty: Ty,
        initializer: Spanned<Initializer<LocalResolver>>,
        locals: &mut Arena<HirLocal>,
        local_map: &mut HashMap<usize, LocalId>,
    ) -> Result<InitializerTree, (co2_ast::Span, String)> {
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
                let expr_ty = expr.ty;
                let coerced = coerce_expr_to_type(expr, expected_ty).ok_or_else(|| {
                    spanned_error(
                        initializer.1,
                        format!(
                            "initializer type mismatch: expected {}, got {}",
                            self.format_ty(expected_ty),
                            self.format_ty(expr_ty)
                        ),
                    )
                })?;
                Ok(InitializerTree::Leaf(coerced))
            }
            Initializer::List(items) => {
                // Redundant outer braces are allowed for aggregates: `S s = { {a,b} }` == `S s = {a,b}`.
                // For arrays this only holds when the inner items do NOT contain field designators.
                // Field designators mean the inner `{}` initializes a struct *element*, not the array
                // itself.  E.g. `T arr[] = { {.field=v} }` — inner `{}` initializes arr[0], so we
                // must NOT strip the outer brace.
                // Subscript/range designators (or no designators) mean the inner `{}` IS the array
                // initializer wrapped in a redundant extra brace pair — strip it.
                if let [
                    (
                        InitializerItem {
                            designators: None,
                            initializer: (Initializer::List(nested), nested_span),
                        },
                        _,
                    ),
                ] = items.as_slice()
                {
                    let inner_has_field_desig = nested.iter().any(|(item, _)| {
                        item.designators.as_ref().is_some_and(|desigs| {
                            desigs
                                .iter()
                                .any(|(d, _)| matches!(d, Designator::Field(_)))
                        })
                    });
                    if !is_array_ty(expected_ty) || !inner_has_field_desig {
                        return self.try_lower_to_initializer_tree(
                            expected_ty,
                            (Initializer::List(nested.clone()), *nested_span),
                            locals,
                            local_map,
                        );
                    }
                }
                if self.adt_logical_field_tys(expected_ty).is_none() && !is_array_ty(expected_ty) {
                    let first = items.into_iter().next().ok_or_else(|| {
                        spanned_error(initializer.1, "empty initializer list for scalar type")
                    })?;
                    if first.0.designators.is_some() {
                        return Err(spanned_error(
                            initializer.1,
                            "designators are invalid for scalar initializer",
                        ));
                    }
                    return Ok(self.lower_to_initializer_tree(
                        expected_ty,
                        first.0.initializer,
                        locals,
                        local_map,
                    ));
                }

                let mut result = InitializerTree::Middle {
                    children: vec![
                        InitializerTree::Zeroed;
                        children_count_of_ty(self, expected_ty)
                    ],
                };
                let mut cursor = if self.adt_logical_field_tys(expected_ty).is_some()
                    || is_array_ty(expected_ty)
                {
                    let mut c = InitializerCursor {
                        base_ty: expected_ty,
                        stack: vec![],
                    };
                    if let Some(fields) = self.adt_logical_field_tys(expected_ty) {
                        if !fields.is_empty() {
                            c.stack.push((0, fields[0]));
                            if is_union_ty(expected_ty)
                                && let Some(sub_fields) = self.adt_logical_field_tys(fields[0])
                                && !sub_fields.is_empty()
                            {
                                c.stack.push((0, sub_fields[0]));
                            }
                        }
                    } else if is_array_ty(expected_ty) {
                        let elem = array_elem_ty(expected_ty).expect("array elem");
                        c.stack.push((0, elem));
                    }
                    c
                } else {
                    return Err(spanned_error(
                        initializer.1,
                        format!("invalid initializer list target type: {expected_ty:?}"),
                    ));
                };

                for (item, item_span) in items {
                    let mut repeated_range = None;
                    if let Some(designators) = &item.designators {
                        let range_tail = designators.split_last().and_then(|(last, prefix)| {
                            if prefix.iter().any(|(designator, _)| {
                                matches!(designator, Designator::Range(_, _))
                            }) {
                                self.terminate_with_error(
                                    item_span,
                                    "GNU range designator is only supported as the last designator",
                                );
                            }
                            match &last.0 {
                                Designator::Range(start, end) => Some((prefix, start, end, last.1)),
                                _ => None,
                            }
                        });
                        if let Some((prefix, start, end, range_span)) = range_tail {
                            cursor = if prefix.is_empty() {
                                InitializerCursor {
                                    base_ty: expected_ty,
                                    stack: vec![],
                                }
                            } else {
                                InitializerCursor::from_designators(
                                    self,
                                    prefix,
                                    expected_ty,
                                    self.to_rust_span(item_span),
                                    locals,
                                    local_map,
                                )?
                            };
                            let start_idx =
                                self.eval_array_len_expr_in_scope(start, locals, local_map)?;
                            let end_idx =
                                self.eval_array_len_expr_in_scope(end, locals, local_map)?;
                            if end_idx < start_idx {
                                self.terminate_with_error(
                                    range_span,
                                    "GNU range designator end must be >= start",
                                );
                            }
                            let elem_ty = array_elem_ty(cursor.ty()).ok_or_else(|| {
                                spanned_error(
                                    item_span,
                                    format!(
                                        "GNU range designator used on non-array type: {:?}",
                                        cursor.ty()
                                    ),
                                )
                            })?;
                            repeated_range = Some((start_idx, end_idx, elem_ty));
                        } else {
                            cursor = InitializerCursor::from_designators(
                                self,
                                designators,
                                expected_ty,
                                self.to_rust_span(item_span),
                                locals,
                                local_map,
                            )?;
                        }
                    }
                    if cursor.stack.is_empty() && repeated_range.is_none() {
                        // Overflowing item, emit warning
                        continue;
                    }
                    let mut range_value_cursor = cursor.clone();
                    if let Some((start_idx, _, elem_ty)) = repeated_range {
                        range_value_cursor.stack.push((start_idx, elem_ty));
                    }
                    let value_cursor = if repeated_range.is_some() {
                        &mut range_value_cursor
                    } else {
                        &mut cursor
                    };
                    let node = if let Initializer::Expr(expr) = item.initializer.0 {
                        if matches!(expr.0, Expression::Constant(co2_ast::Constant::String(_))) {
                            loop {
                                let terminal = match value_cursor.ty().kind() {
                                    TyKind::RigidTy(rigid_ty) => match rigid_ty {
                                        RigidTy::Adt(..) => false,
                                        RigidTy::Array(inner, _) => inner.kind().is_primitive(),
                                        _ => true,
                                    },
                                    _ => false,
                                };
                                if terminal {
                                    break self.lower_to_initializer_tree(
                                        value_cursor.ty(),
                                        (Initializer::Expr(expr), item_span),
                                        locals,
                                        local_map,
                                    );
                                }
                                if !value_cursor.go_through(self) {
                                    self.terminate_with_error(
                                        item_span,
                                        "failed to lower string literal as initializer tree",
                                    );
                                }
                            }
                        } else {
                            let mut expr = self.lower_expr(expr, locals, local_map)?;
                            self.array_to_pointer_decay_if_array(&mut expr);
                            self.fn_def_to_c_fn_ptr_decay_if_fn_def(&mut expr);
                            loop {
                                if let Some(coerced) =
                                    coerce_expr_to_type(expr.clone(), value_cursor.ty())
                                {
                                    break InitializerTree::Leaf(coerced);
                                }
                                let expected_ty = value_cursor.ty();
                                let expr_ty = expr.ty;
                                if !value_cursor.go_through(self) {
                                    return Err(spanned_error(
                                        item_span,
                                        format!(
                                            "initializer type mismatch: expected {}, got {}",
                                            self.format_ty(expected_ty),
                                            self.format_ty(expr_ty)
                                        ),
                                    ));
                                }
                            }
                        }
                    } else {
                        self.lower_to_initializer_tree(
                            value_cursor.ty(),
                            item.initializer,
                            locals,
                            local_map,
                        )
                    };
                    if let Some((start_idx, end_idx, elem_ty)) = repeated_range {
                        for idx in start_idx..=end_idx {
                            let mut range_cursor = cursor.clone();
                            range_cursor.stack.push((idx, elem_ty));
                            range_cursor.insert_to_tree(self, &mut result, node.clone());
                        }
                        cursor.stack.push((end_idx, elem_ty));
                    } else {
                        cursor.insert_to_tree(self, &mut result, node);
                    }
                    cursor.go_next(self, span)?;
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
        let is_byte_string = matches!(
            array_elem_ty(expected_ty).map(|ty| ty.kind()),
            Some(TyKind::RigidTy(
                RigidTy::Int(IntTy::I8) | RigidTy::Uint(UintTy::U8)
            ))
        );
        let items = if is_byte_string {
            s.iter()
                .copied()
                .chain([0u8])
                .map(|byte| {
                    (
                        InitializerItem {
                            designators: None,
                            initializer: (
                                Initializer::Expr((
                                    Expression::Constant(co2_ast::Constant::Int(
                                        i128::from(byte),
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
                .collect::<Vec<_>>()
        } else {
            String::from_utf8_lossy(&s)
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
                .collect::<Vec<_>>()
        };
        let _ = expected_ty;
        items
    }
}
