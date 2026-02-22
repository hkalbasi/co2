use std::collections::HashMap;

use co2_parser::{Designator, Expression, Initializer, InitializerItem, Spanned};
use la_arena::Arena;
use rustc_public_generative::rustc_public::{
    mir::Mutability,
    ty::{IntTy, RigidTy, Ty, TyKind},
};

use crate::{
    expr::{HirExpr, HirExprKind},
    item::{HirLocal, LocalId},
    resolver::HirCtx,
    stmt::HirStmt,
    ty::{
        adt_field_tys, array_elem_ty, is_array_ty, is_integer_ty, is_maybe_uninit_fn_ptr_ty,
        resolve_field_in_adt, ty_matches_expected,
    },
};

#[derive(Clone, Debug)]
pub(crate) enum InitializerTree {
    Middle { children: Vec<InitializerTree> },
    Leaf(HirExpr),
    Zeroed,
}

pub(crate) fn tree_contains_zeroed(tree: &InitializerTree) -> bool {
    match tree {
        InitializerTree::Zeroed => true,
        InitializerTree::Leaf(_) => false,
        InitializerTree::Middle { children } => children.iter().any(tree_contains_zeroed),
    }
}

#[derive(Debug)]
struct InitializerCursor {
    base_ty: Ty,
    stack: Vec<(usize, Ty)>,
}

impl InitializerCursor {
    fn from_designators<R>(
        ctx: &HirCtx<'_, R>,
        designators: &[Spanned<Designator>],
        base_ty: Ty,
        span: rustc_public_generative::rustc_public::ty::Span,
        locals: &mut Arena<HirLocal>,
        local_map: &mut HashMap<String, LocalId>,
    ) -> Result<Self, String> {
        let mut current_ty = base_ty;
        let mut cursor = InitializerCursor {
            base_ty,
            stack: vec![],
        };
        for (designator, _) in designators {
            match designator {
                Designator::Subscript(expr) => {
                    let idx_expr = ctx.lower_expr(expr.clone(), locals, local_map)?;
                    if !is_integer_ty(idx_expr.ty) {
                        return Err("array designator index must be integer".to_owned());
                    }
                    let idx = eval_const_int(&idx_expr)? as usize;
                    let elem_ty = array_elem_ty(current_ty).ok_or_else(|| {
                        format!("array designator used on non-array type: {:?}", current_ty)
                    })?;
                    cursor.stack.push((idx, elem_ty));
                    current_ty = elem_ty;
                }
                Designator::Field(name) => {
                    let (index, field_ty) = resolve_field_in_adt(current_ty, name.0.as_str())
                        .ok_or_else(|| {
                            format!(
                                "field designator `{}` used on non-struct type: {:?}",
                                name.0, current_ty
                            )
                        })?;
                    cursor.stack.push((index, field_ty));
                    current_ty = field_ty;
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
        let mut current = result;
        for (index, _) in &self.stack {
            let children = current.children();
            while children.len() <= *index {
                children.push(InitializerTree::Zeroed);
            }
            current = &mut children[*index];
        }
        *current = value;
    }

    fn go_through_primitive(&mut self) -> Result<(), String> {
        if self.stack.is_empty() {
            return Ok(());
        }
        loop {
            let ty = self.ty();
            if let Some(fields) = adt_field_tys(ty) {
                if fields.is_empty() {
                    return Ok(());
                }
                self.stack.push((0, fields[0]));
                continue;
            }
            if let Some(elem) = array_elem_ty(ty) {
                self.stack.push((0, elem));
                continue;
            }
            return Ok(());
        }
    }

    fn go_next(&mut self, span: rustc_public_generative::rustc_public::ty::Span) -> Result<(), String> {
        if self.stack.is_empty() {
            return Ok(());
        }
        let (mut idx, _) = self.stack.pop().expect("stack not empty");
        let parent_ty = self.stack.last().map(|(_, ty)| *ty).unwrap_or(self.base_ty);

        if let Some(fields) = adt_field_tys(parent_ty) {
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

        Err(format!("invalid initializer cursor parent type: {parent_ty:?} at {span:?}"))
    }
}

impl InitializerTree {
    fn children(&mut self) -> &mut Vec<InitializerTree> {
        match self {
            InitializerTree::Middle { children } => children,
            InitializerTree::Leaf(_) => panic!("leaf does not have children"),
            InitializerTree::Zeroed => {
                *self = InitializerTree::Middle { children: vec![] };
                let InitializerTree::Middle { children } = self else {
                    unreachable!();
                };
                children
            }
        }
    }
}

fn eval_const_int(expr: &HirExpr) -> Result<i64, String> {
    match &expr.kind {
        HirExprKind::ConstInt(v) => Ok(*v),
        HirExprKind::Binary { op, lhs, rhs } => match op {
            crate::expr::HirBinOp::Add => Ok(eval_const_int(lhs)? + eval_const_int(rhs)?),
            crate::expr::HirBinOp::Sub => Ok(eval_const_int(lhs)? - eval_const_int(rhs)?),
            _ => Err("designator subscript must be constant integer expression".to_owned()),
        },
        _ => Err("designator subscript must be constant integer expression".to_owned()),
    }
}

fn coerce_expr_to_type(expr: HirExpr, expected_ty: Ty) -> Result<HirExpr, String> {
    if expr.ty == expected_ty {
        return Ok(expr);
    }
    if is_maybe_uninit_fn_ptr_ty(expected_ty).is_some()
        && matches!(
            expr.ty.kind(),
            TyKind::RigidTy(RigidTy::FnDef(_, _) | RigidTy::FnPtr(_) | RigidTy::Int(_) | RigidTy::Uint(_))
        )
    {
        return Ok(HirExpr {
            kind: HirExprKind::Cast(Box::new(expr.clone())),
            ty: expected_ty,
            span: expr.span,
        });
    }
    if matches!(expected_ty.kind(), TyKind::RigidTy(RigidTy::FnPtr(_)))
        && matches!(expr.ty.kind(), TyKind::RigidTy(RigidTy::FnDef(_, _)))
    {
        return Ok(HirExpr {
            kind: HirExprKind::Cast(Box::new(expr.clone())),
            ty: expected_ty,
            span: expr.span,
        });
    }
    if ty_matches_expected(expected_ty, expr.ty) {
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

fn array_len_from_layout(ty: Ty) -> Option<usize> {
    let elem = array_elem_ty(ty)?;
    let total = ty.layout().ok()?.shape().size.bytes();
    let elem_sz = elem.layout().ok()?.shape().size.bytes();
    if elem_sz == 0 {
        return None;
    }
    Some((total / elem_sz) as usize)
}

impl<R> HirCtx<'_, R> {
    pub(crate) fn lower_to_initializer_tree(
        &self,
        expected_ty: Ty,
        initializer: Spanned<Initializer>,
        locals: &mut Arena<HirLocal>,
        local_map: &mut HashMap<String, LocalId>,
    ) -> Result<InitializerTree, String> {
        let span = self.to_rust_span(initializer.1);
        match initializer.0 {
            Initializer::Expr(expr) => {
                // C string literal can initialize char arrays.
                if is_array_ty(expected_ty)
                    && matches!(expr.0, Expression::Constant(co2_parser::Constant::String(_)))
                {
                    let list = self.initializer_list_from_string(expected_ty, expr.clone());
                    return self.lower_to_initializer_tree(
                        expected_ty,
                        (Initializer::List(list), initializer.1),
                        locals,
                        local_map,
                    );
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
                    return self.lower_to_initializer_tree(
                        expected_ty,
                        first.0.initializer,
                        locals,
                        local_map,
                    );
                }

                // Preserve legacy behavior used by existing tests:
                // when struct list has no designators, allow type-based field matching fallback.
                if let Some(field_tys) = adt_field_tys(expected_ty)
                    && items.iter().all(|(it, _)| {
                        it.designators.is_none() && matches!(it.initializer.0, Initializer::Expr(_))
                    })
                {
                    let mut lowered = Vec::with_capacity(items.len());
                    for (it, _) in &items {
                        let Initializer::Expr(expr) = &it.initializer.0 else {
                            unreachable!();
                        };
                        lowered.push(self.lower_expr(
                            expr.clone(),
                            locals,
                            local_map,
                        )?);
                    }

                    let mut positional_children: Vec<InitializerTree> = vec![InitializerTree::Zeroed; field_tys.len()];
                    let mut positional_ok = lowered.len() <= field_tys.len();
                    if positional_ok {
                        for (idx, val) in lowered.iter().enumerate() {
                            match coerce_expr_to_type(val.clone(), field_tys[idx]) {
                                Ok(coerced) => positional_children[idx] = InitializerTree::Leaf(coerced),
                                Err(_) => {
                                    positional_ok = false;
                                    break;
                                }
                            }
                        }
                    }
                    if positional_ok {
                        return Ok(InitializerTree::Middle {
                            children: positional_children,
                        });
                    }

                    let mut reordered_children: Vec<InitializerTree> = vec![InitializerTree::Zeroed; field_tys.len()];
                    let mut used = vec![false; field_tys.len()];
                    for val in lowered {
                        let mut placed = false;
                        for (idx, field_ty) in field_tys.iter().enumerate() {
                            if used[idx] {
                                continue;
                            }
                            if let Ok(coerced) = coerce_expr_to_type(val.clone(), *field_ty) {
                                reordered_children[idx] = InitializerTree::Leaf(coerced);
                                used[idx] = true;
                                placed = true;
                                break;
                            }
                        }
                        if !placed {
                            return Err(format!(
                                "initializer type mismatch for ADT {expected_ty:?}: no compatible field for {:?}",
                                val.ty
                            ));
                        }
                    }
                    return Ok(InitializerTree::Middle {
                        children: reordered_children,
                    });
                }

                let mut result = InitializerTree::Middle { children: vec![] };
                let mut cursor = if adt_field_tys(expected_ty).is_some() || is_array_ty(expected_ty) {
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
                    return Err(format!("invalid initializer list target type: {expected_ty:?}"));
                };

                for (item, item_span) in items {
                    if let Some(designators) = &item.designators {
                        cursor = InitializerCursor::from_designators(
                            self,
                            designators,
                            expected_ty,
                            self.to_rust_span(item_span),
                            locals,
                            local_map,
                        )?;
                    }
                    if matches!(item.initializer.0, Initializer::Expr(_)) {
                        cursor.go_through_primitive()?;
                    }
                    let node = self.lower_to_initializer_tree(
                        cursor.ty(),
                        item.initializer,
                        locals,
                        local_map,
                    )?;
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
        expr: Spanned<Expression>,
    ) -> Vec<Spanned<InitializerItem>> {
        let Expression::Constant(co2_parser::Constant::String(s)) = expr.0 else {
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
                                Expression::Constant(co2_parser::Constant::Int(
                                    ch as i64,
                                    co2_parser::IntegerSuffix::None,
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

    pub(crate) fn emit_initializer_tree_assignments(
        &self,
        lhs: HirExpr,
        tree: &InitializerTree,
        out: &mut Vec<HirStmt>,
    ) -> Result<(), String> {
        match tree {
            InitializerTree::Zeroed => Ok(()),
            InitializerTree::Leaf(expr) => {
                out.push(HirStmt::Expr(HirExpr {
                    kind: HirExprKind::Assign {
                        lhs: Box::new(lhs.clone()),
                        rhs: Box::new(expr.clone()),
                    },
                    ty: lhs.ty,
                    span: lhs.span,
                }));
                Ok(())
            }
            InitializerTree::Middle { children } => {
                if let Some(fields) = adt_field_tys(lhs.ty) {
                    for (idx, child) in children.iter().enumerate() {
                        if idx >= fields.len() {
                            return Err("too many struct initializer elements".to_owned());
                        }
                        let child_lhs = HirExpr {
                            kind: HirExprKind::Field {
                                base: Box::new(lhs.clone()),
                                index: idx,
                            },
                            ty: fields[idx],
                            span: lhs.span,
                        };
                        self.emit_initializer_tree_assignments(child_lhs, child, out)?;
                    }
                    return Ok(());
                }
                if is_array_ty(lhs.ty) {
                    let elem_ty = array_elem_ty(lhs.ty).expect("array elem");
                    let ptr_ty = Ty::new_ptr(elem_ty, Mutability::Mut);
                    for (idx, child) in children.iter().enumerate() {
                        let idx_expr = HirExpr {
                            kind: HirExprKind::ConstInt(idx as i64),
                            ty: Ty::signed_ty(IntTy::I32),
                            span: lhs.span,
                        };
                        let ptr = HirExpr {
                            kind: HirExprKind::PtrOffset {
                                base: Box::new(HirExpr {
                                    kind: HirExprKind::ArrayToPointer(Box::new(lhs.clone())),
                                    ty: ptr_ty,
                                    span: lhs.span,
                                }),
                                index: Box::new(idx_expr),
                            },
                            ty: ptr_ty,
                            span: lhs.span,
                        };
                        let child_lhs = HirExpr {
                            kind: HirExprKind::Deref(Box::new(ptr)),
                            ty: elem_ty,
                            span: lhs.span,
                        };
                        self.emit_initializer_tree_assignments(child_lhs, child, out)?;
                    }
                    return Ok(());
                }
                Err(format!("initializer list target is not aggregate: {:?}", lhs.ty))
            }
        }
    }
}
