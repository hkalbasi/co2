use std::collections::{BTreeMap, HashMap};

use co2_ast::{
    BinOp as ParsedBinOp, Constant, Expression, GenericAssociation, IntegerSuffix, Spanned,
    Statement, StatementOrDeclaration, UnaryOp as ParsedUnaryOp, UpdateOp as ParsedUpdateOp,
};
use co2_crate_sig::{LocalResolver, LogicalAdtFieldKind, MethodResolutionKind};
use la_arena::Arena;
use rustc_public_generative::rustc_public::{
    CrateDefType, CrateItem,
    abi::FieldsShape,
    mir::Mutability,
    ty::{
        FloatTy, GenericArgKind, GenericArgs, IntTy, RigidTy, Span as RustSpan, Ty, TyKind, UintTy,
    },
};

use crate::decl::hir_ty_to_ty;
use crate::item::{HirLocal, LocalId};
use crate::resolver::{HirCtx, ResolvedValue};
use crate::stmt::HirStmt;
use crate::ty::{
    adt_field_tys, array_elem_ty, callable_sig, common_numeric_ty, enum_payload_ty, is_array_ty,
    is_condition_ty, is_maybe_uninit_fn_ptr_ty, is_numeric_ty, needs_implicit_cast,
    resolve_field_path_in_adt, ty_matches_expected,
};
use crate::{initializer_tree::InitializerTree, ty::common_ternary_ty};

fn lower_dependency_const(
    value: rustc_public_generative::DependencyConstValue,
    span: RustSpan,
) -> HirExpr {
    match value {
        rustc_public_generative::DependencyConstValue::Bool(v) => HirExpr {
            kind: HirExprKind::ConstInt(if v { 1 } else { 0 }),
            ty: Ty::bool_ty(),
            span,
        },
        rustc_public_generative::DependencyConstValue::Char(ch) => HirExpr {
            kind: HirExprKind::ConstInt(ch as i128),
            ty: Ty::signed_ty(IntTy::I32),
            span,
        },
        rustc_public_generative::DependencyConstValue::I8(v) => HirExpr {
            kind: HirExprKind::ConstInt(v as i128),
            ty: Ty::signed_ty(IntTy::I8),
            span,
        },
        rustc_public_generative::DependencyConstValue::I16(v) => HirExpr {
            kind: HirExprKind::ConstInt(v as i128),
            ty: Ty::signed_ty(IntTy::I16),
            span,
        },
        rustc_public_generative::DependencyConstValue::I32(v) => HirExpr {
            kind: HirExprKind::ConstInt(v as i128),
            ty: Ty::signed_ty(IntTy::I32),
            span,
        },
        rustc_public_generative::DependencyConstValue::I64(v) => HirExpr {
            kind: HirExprKind::ConstInt(v as i128),
            ty: Ty::signed_ty(IntTy::I64),
            span,
        },
        rustc_public_generative::DependencyConstValue::I128(v) => HirExpr {
            kind: HirExprKind::ConstInt(v),
            ty: Ty::signed_ty(IntTy::I128),
            span,
        },
        rustc_public_generative::DependencyConstValue::Isize(v) => HirExpr {
            kind: HirExprKind::ConstInt(v as i128),
            ty: Ty::signed_ty(IntTy::Isize),
            span,
        },
        rustc_public_generative::DependencyConstValue::U8(v) => HirExpr {
            kind: HirExprKind::ConstInt(v as i128),
            ty: Ty::unsigned_ty(UintTy::U8),
            span,
        },
        rustc_public_generative::DependencyConstValue::U16(v) => HirExpr {
            kind: HirExprKind::ConstInt(v as i128),
            ty: Ty::unsigned_ty(UintTy::U16),
            span,
        },
        rustc_public_generative::DependencyConstValue::U32(v) => HirExpr {
            kind: HirExprKind::ConstInt(v as i128),
            ty: Ty::unsigned_ty(UintTy::U32),
            span,
        },
        rustc_public_generative::DependencyConstValue::U64(v) => HirExpr {
            kind: HirExprKind::ConstInt(v as i128),
            ty: Ty::unsigned_ty(UintTy::U64),
            span,
        },
        rustc_public_generative::DependencyConstValue::U128(v) => HirExpr {
            kind: HirExprKind::ConstInt(v as i128),
            ty: Ty::unsigned_ty(UintTy::U128),
            span,
        },
        rustc_public_generative::DependencyConstValue::Usize(v) => HirExpr {
            kind: HirExprKind::ConstInt(v as i128),
            ty: Ty::unsigned_ty(UintTy::Usize),
            span,
        },
        rustc_public_generative::DependencyConstValue::F32(v) => HirExpr {
            kind: HirExprKind::ConstFloat(v as f64),
            ty: Ty::from_rigid_kind(RigidTy::Float(FloatTy::F32)),
            span,
        },
        rustc_public_generative::DependencyConstValue::F64(v) => HirExpr {
            kind: HirExprKind::ConstFloat(v),
            ty: Ty::from_rigid_kind(RigidTy::Float(FloatTy::F64)),
            span,
        },
    }
}

#[derive(Clone)]
pub struct HirExpr {
    pub kind: HirExprKind,
    pub ty: Ty,
    pub span: RustSpan,
}

impl HirExpr {
    fn is_null_like(&self) -> bool {
        match &self.kind {
            HirExprKind::Zeroed => true,
            HirExprKind::ConstInt(0) => true,
            HirExprKind::Cast(inner) => inner.is_null_like(),
            _ => false,
        }
    }
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
    LocalConst(LocalId),
    ConstInt(i128),
    ConstFloat(f64),
    ConstStr(String),
    LabelAddress(crate::LabelId),
    Zeroed,
    UnionAggregate {
        active_field: usize,
        arg: Box<HirExpr>,
    },
    Field {
        base: Box<HirExpr>,
        index: usize,
    },
    Bitfield {
        base: Box<HirExpr>,
        storage_index: usize,
        storage_ty: Ty,
        bit_offset: usize,
        bit_width: usize,
        signed: bool,
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
    // TODO: this variant is just a cast duplicate
    ArrayToPointer(Box<HirExpr>),
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

#[derive(Clone, Debug)]
enum ResolvedFieldAccess {
    Direct {
        path: Vec<usize>,
        ty: Ty,
    },
    Bitfield {
        container_path: Vec<usize>,
        storage_index: usize,
        storage_ty: Ty,
        bit_offset: usize,
        bit_width: usize,
        signed: bool,
        ty: Ty,
    },
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

fn collect_param_bindings_for_receiver(expected: Ty, actual: Ty, out: &mut BTreeMap<u32, Ty>) {
    match (expected.kind(), actual.kind()) {
        (TyKind::Param(param), _) => {
            out.entry(param.index).or_insert(actual);
        }
        (TyKind::RigidTy(RigidTy::Ref(_, expected_inner, _)), _) => {
            collect_param_bindings_for_receiver(expected_inner, actual, out);
        }
        (
            TyKind::RigidTy(RigidTy::RawPtr(expected_inner, _)),
            TyKind::RigidTy(RigidTy::RawPtr(actual_inner, _)),
        ) => {
            collect_param_bindings_for_receiver(expected_inner, actual_inner, out);
        }
        (
            TyKind::RigidTy(RigidTy::Adt(expected_adt, expected_args)),
            TyKind::RigidTy(RigidTy::Adt(actual_adt, actual_args)),
        ) if expected_adt == actual_adt && actual_args.0.len() <= expected_args.0.len() => {
            for (expected_arg, actual_arg) in expected_args.0.iter().zip(actual_args.0.iter()) {
                if let (
                    rustc_public_generative::rustc_public::ty::GenericArgKind::Type(expected_ty),
                    rustc_public_generative::rustc_public::ty::GenericArgKind::Type(actual_ty),
                ) = (expected_arg, actual_arg)
                {
                    collect_param_bindings_for_receiver(*expected_ty, *actual_ty, out);
                }
            }
        }
        _ => {}
    }
}

fn substitute_ty_params(ty: Ty, bindings: &BTreeMap<u32, Ty>) -> Ty {
    match ty.kind() {
        TyKind::Param(param) => bindings.get(&param.index).copied().unwrap_or(ty),
        TyKind::RigidTy(RigidTy::Adt(def, args)) => Ty::from_rigid_kind(RigidTy::Adt(
            def,
            GenericArgs(
                args.0
                    .iter()
                    .map(|arg| match arg {
                        rustc_public_generative::rustc_public::ty::GenericArgKind::Type(inner) => {
                            rustc_public_generative::rustc_public::ty::GenericArgKind::Type(
                                substitute_ty_params(*inner, bindings),
                            )
                        }
                        _ => arg.clone(),
                    })
                    .collect(),
            ),
        )),
        TyKind::RigidTy(RigidTy::Ref(region, inner, mutability)) => Ty::from_rigid_kind(
            RigidTy::Ref(region, substitute_ty_params(inner, bindings), mutability),
        ),
        TyKind::RigidTy(RigidTy::RawPtr(inner, mutability)) => Ty::from_rigid_kind(
            RigidTy::RawPtr(substitute_ty_params(inner, bindings), mutability),
        ),
        TyKind::RigidTy(RigidTy::Tuple(items)) => Ty::from_rigid_kind(RigidTy::Tuple(
            items
                .iter()
                .map(|item| substitute_ty_params(*item, bindings))
                .collect(),
        )),
        TyKind::RigidTy(RigidTy::Array(inner, len)) => {
            Ty::from_rigid_kind(RigidTy::Array(substitute_ty_params(inner, bindings), len))
        }
        _ => ty,
    }
}

fn normalize_stable_defaulted_ty(ty: Ty) -> Ty {
    match ty.kind() {
        TyKind::RigidTy(RigidTy::Ref(region, inner, mutability)) => {
            Ty::new_ref(region, normalize_stable_defaulted_ty(inner), mutability)
        }
        TyKind::RigidTy(RigidTy::RawPtr(inner, mutability)) => {
            Ty::new_ptr(normalize_stable_defaulted_ty(inner), mutability)
        }
        TyKind::RigidTy(RigidTy::Tuple(items)) => Ty::new_tuple(
            &items
                .iter()
                .map(|item| normalize_stable_defaulted_ty(*item))
                .collect::<Vec<_>>(),
        ),
        TyKind::RigidTy(RigidTy::Array(inner, len)) => {
            Ty::from_rigid_kind(RigidTy::Array(normalize_stable_defaulted_ty(inner), len))
        }
        TyKind::RigidTy(RigidTy::Adt(adt, args)) => Ty::from_rigid_kind(RigidTy::Adt(
            adt,
            GenericArgs(
                args.0
                    .iter()
                    .map(|arg| match arg {
                        GenericArgKind::Type(inner) => {
                            GenericArgKind::Type(normalize_stable_defaulted_ty(*inner))
                        }
                        _ => arg.clone(),
                    })
                    .collect(),
            ),
        )),
        _ => ty,
    }
}

fn receiver_generic_args(ty: Ty) -> Vec<GenericArgKind> {
    match ty.kind() {
        TyKind::RigidTy(RigidTy::Adt(_, args)) => args.0.clone(),
        TyKind::RigidTy(RigidTy::Ref(_, inner, _) | RigidTy::RawPtr(inner, _)) => {
            receiver_generic_args(inner)
        }
        _ => vec![],
    }
}

fn trait_method_self_ty(ty: Ty) -> Ty {
    match ty.kind() {
        TyKind::RigidTy(RigidTy::Ref(_, inner, _) | RigidTy::RawPtr(inner, _)) => {
            trait_method_self_ty(inner)
        }
        _ => ty,
    }
}

impl HirCtx<'_> {
    fn method_receiver_arg(&self, receiver: HirExpr, expected_ty: Ty) -> HirExpr {
        match expected_ty.kind() {
            TyKind::RigidTy(RigidTy::Ref(_, _, _) | RigidTy::RawPtr(_, _))
                if !matches!(
                    receiver.ty.kind(),
                    TyKind::RigidTy(RigidTy::Ref(_, _, _) | RigidTy::RawPtr(_, _))
                ) =>
            {
                HirExpr {
                    kind: HirExprKind::AddrOf(Box::new(receiver.clone())),
                    ty: Ty::new_ptr(receiver.ty, addr_of_mutability(&receiver)),
                    span: receiver.span,
                }
            }
            _ => receiver,
        }
    }

    fn specialize_fn_sig_from_receiver(
        &self,
        fn_def: rustc_public_generative::rustc_public::ty::FnDef,
        fn_generic_args: &[GenericArgKind],
        receiver_ty: Ty,
    ) -> Option<rustc_public_generative::rustc_public::ty::FnSig> {
        let fn_ty = if fn_generic_args.is_empty() {
            fn_def.ty()
        } else {
            Ty::from_rigid_kind(RigidTy::FnDef(
                fn_def,
                GenericArgs(fn_generic_args.to_vec()),
            ))
        };
        let sig = callable_sig(fn_ty)?;
        let mut sig = rustc_public_generative::erase_late_bound_regions_in_fn_sig(sig);
        let mut by_index = BTreeMap::new();
        if let Some(first_input) = sig.inputs().first() {
            collect_param_bindings_for_receiver(*first_input, receiver_ty, &mut by_index);
        }
        collect_param_bindings_for_receiver(sig.output(), receiver_ty, &mut by_index);
        sig.inputs_and_output = sig
            .inputs_and_output
            .into_iter()
            .map(|ty| normalize_stable_defaulted_ty(substitute_ty_params(ty, &by_index)))
            .collect();
        Some(sig)
    }

    fn lower_call_args(
        &self,
        parser_span: co2_ast::Span,
        sig: &rustc_public_generative::rustc_public::ty::FnSig,
        args: &mut Vec<HirExpr>,
    ) {
        if sig.inputs().len() != args.len() && !sig.c_variadic {
            self.terminate_with_error(
                parser_span,
                &format!(
                    "call argument count mismatch: expected {}, got {}",
                    sig.inputs().len(),
                    args.len()
                ),
            );
        }

        for (idx, actual) in args.iter_mut().enumerate() {
            let expected = match sig.inputs().get(idx) {
                Some(ty) => *ty,
                None => {
                    if actual.ty.kind().is_adt() && enum_payload_ty(actual.ty).is_none() {
                        *actual = HirExpr {
                            kind: HirExprKind::AddrOf(Box::new(actual.clone())),
                            ty: Ty::new_ptr(actual.ty, Mutability::Mut),
                            span: actual.span,
                        };
                        continue;
                    }
                    ty_passed_to_variadic(actual.ty)
                }
            };
            if needs_implicit_cast(expected, actual.ty) {
                *actual = HirExpr {
                    kind: HirExprKind::Cast(Box::new(actual.clone())),
                    ty: expected,
                    span: actual.span,
                };
            }
            if !ty_matches_expected(expected, actual.ty) {
                self.terminate_with_error(
                    parser_span,
                    &format!(
                        "call argument type mismatch at index {idx}: expected {expected:?}, got {:?}",
                        actual.ty
                    ),
                );
            }
        }
    }

    fn try_lower_method_call(
        &self,
        func: &Spanned<Expression<LocalResolver>>,
        params: &[Spanned<Expression<LocalResolver>>],
        locals: &mut Arena<HirLocal>,
        local_map: &mut HashMap<usize, LocalId>,
    ) -> Result<
        Option<(
            HirExpr,
            rustc_public_generative::rustc_public::ty::FnSig,
            Vec<HirExpr>,
        )>,
        String,
    > {
        let resolver = &self.decl_resolver;

        let (receiver, method_name, parser_span) = match &func.0 {
            Expression::Field(base, field) => {
                let receiver = self.lower_expr((base.0.clone(), base.1), locals, local_map)?;
                (receiver, field.0.as_str(), func.1)
            }
            Expression::Arrow(base, field) => {
                let mut base = self.lower_expr((base.0.clone(), base.1), locals, local_map)?;
                self.array_to_pointer_decay_if_array(&mut base);
                let TyKind::RigidTy(RigidTy::RawPtr(pointee, _)) = base.ty.kind() else {
                    return Ok(None);
                };
                let receiver = HirExpr {
                    kind: HirExprKind::Deref(Box::new(base)),
                    ty: pointee,
                    span: self.to_rust_span(func.1),
                };
                (receiver, field.0.as_str(), func.1)
            }
            _ => return Ok(None),
        };

        let (method_def, class, resolution_kind) =
            match resolver.resolve_method(receiver.ty, method_name) {
                Ok(Some(found)) => found,
                Ok(None) => return Ok(None),
                Err(err) => return Err(err),
            };
        if class != co2_ast::TypeQueryResult::Expr {
            return Ok(None);
        }
        let fn_def = rustc_public_generative::rustc_public::ty::FnDef(method_def);
        let resolved_generic_args = match resolution_kind {
            MethodResolutionKind::Inherent => receiver_generic_args(receiver.ty),
            MethodResolutionKind::Trait => {
                vec![GenericArgKind::Type(trait_method_self_ty(receiver.ty))]
            }
        };
        let Some(sig) =
            self.specialize_fn_sig_from_receiver(fn_def, &resolved_generic_args, receiver.ty)
        else {
            return Ok(None);
        };
        let resolved = match resolution_kind {
            MethodResolutionKind::Inherent => ResolvedValue::Fn(fn_def, resolved_generic_args),
            MethodResolutionKind::Trait => ResolvedValue::Fn(fn_def, resolved_generic_args),
        };
        let func_ty = resolved.ty();

        let mut lowered_args = Vec::with_capacity(params.len() + 1);
        let receiver = sig
            .inputs()
            .first()
            .copied()
            .map(|expected| self.method_receiver_arg(receiver.clone(), expected))
            .unwrap_or(receiver);
        lowered_args.push(receiver);
        for param in params {
            let mut arg = self.lower_expr((param.0.clone(), param.1), locals, local_map)?;
            self.array_to_pointer_decay_if_array(&mut arg);
            lowered_args.push(arg);
        }

        Ok(Some((
            HirExpr {
                kind: HirExprKind::Path(resolved),
                ty: func_ty,
                span: self.to_rust_span(parser_span),
            },
            sig,
            lowered_args,
        )))
    }

    fn try_lower_assoc_method_call(
        &self,
        func: &Spanned<Expression<LocalResolver>>,
        params: &[Spanned<Expression<LocalResolver>>],
        locals: &mut Arena<HirLocal>,
        local_map: &mut HashMap<usize, LocalId>,
    ) -> Result<
        Option<(
            HirExpr,
            rustc_public_generative::rustc_public::ty::FnSig,
            Vec<HirExpr>,
        )>,
        String,
    > {
        let resolver = &self.decl_resolver;

        let (receiver, parsed_receiver_generic_args, method_name, parser_span) = match &func.0 {
            Expression::Identifier((
                co2_crate_sig::DefOrLocal::AssocMethod {
                    receiver,
                    method,
                    receiver_generic_args,
                },
                _,
            )) => (*receiver, receiver_generic_args, method.as_str(), func.1),
            _ => return Ok(None),
        };

        let receiver_ty = if parsed_receiver_generic_args.is_empty() {
            CrateItem(receiver).ty()
        } else {
            self.ty_of_resolved_path(
                &co2_crate_sig::DefOrLocal::Def {
                    def_id: receiver,
                    generic_args: parsed_receiver_generic_args.clone(),
                },
                self.to_rust_span(parser_span),
            )
        };
        let (method_def, class, resolution_kind) =
            match resolver.resolve_method(receiver_ty, method_name) {
                Ok(Some(found)) => found,
                Ok(None) => return Ok(None),
                Err(err) => return Err(err),
            };
        if class != co2_ast::TypeQueryResult::Expr {
            return Ok(None);
        }

        let fn_def = rustc_public_generative::rustc_public::ty::FnDef(method_def);
        let resolved_generic_args = match resolution_kind {
            MethodResolutionKind::Inherent => {
                let receiver_args = receiver_generic_args(receiver_ty);
                let Some(sig) =
                    self.specialize_fn_sig_from_receiver(fn_def, &receiver_args, receiver_ty)
                else {
                    return Ok(None);
                };
                let output_args = receiver_generic_args(sig.output());
                let resolved_generic_args = if output_args.len() > receiver_args.len()
                    && output_args
                        .iter()
                        .zip(receiver_args.iter())
                        .all(|(o, r)| o == r)
                {
                    output_args
                } else {
                    receiver_args
                };
                let Some(sig) = self.specialize_fn_sig_from_receiver(
                    fn_def,
                    &resolved_generic_args,
                    receiver_ty,
                ) else {
                    return Ok(None);
                };
                return Ok(Some((
                    HirExpr {
                        kind: HirExprKind::Path(ResolvedValue::Fn(
                            fn_def,
                            resolved_generic_args.clone(),
                        )),
                        ty: ResolvedValue::Fn(fn_def, resolved_generic_args).ty(),
                        span: self.to_rust_span(parser_span),
                    },
                    sig,
                    {
                        let mut lowered_args = Vec::with_capacity(params.len());
                        for param in params {
                            let mut arg =
                                self.lower_expr((param.0.clone(), param.1), locals, local_map)?;
                            self.array_to_pointer_decay_if_array(&mut arg);
                            lowered_args.push(arg);
                        }
                        lowered_args
                    },
                )));
            }
            MethodResolutionKind::Trait => {
                vec![GenericArgKind::Type(trait_method_self_ty(receiver_ty))]
            }
        };
        let Some(sig) =
            self.specialize_fn_sig_from_receiver(fn_def, &resolved_generic_args, receiver_ty)
        else {
            return Ok(None);
        };
        let resolved = match resolution_kind {
            MethodResolutionKind::Inherent => ResolvedValue::Fn(fn_def, resolved_generic_args),
            MethodResolutionKind::Trait => ResolvedValue::Fn(fn_def, resolved_generic_args),
        };
        let func_ty = resolved.ty();

        let mut lowered_args = Vec::with_capacity(params.len());
        for param in params {
            let mut arg = self.lower_expr((param.0.clone(), param.1), locals, local_map)?;
            self.array_to_pointer_decay_if_array(&mut arg);
            lowered_args.push(arg);
        }

        Ok(Some((
            HirExpr {
                kind: HirExprKind::Path(resolved),
                ty: func_ty,
                span: self.to_rust_span(parser_span),
            },
            sig,
            lowered_args,
        )))
    }

    pub(crate) fn array_to_pointer_decay_if_array(&self, expr: &mut HirExpr) {
        if !is_array_ty(expr.ty) {
            return;
        }
        *expr = self.array_to_pointer_decay(expr.clone());
    }

    fn array_to_pointer_decay(&self, expr: HirExpr) -> HirExpr {
        let elem = array_elem_ty(expr.ty).expect("Expr is not array");
        let mutability = addr_of_mutability(&expr);
        HirExpr {
            kind: HirExprKind::ArrayToPointer(Box::new(expr.clone())),
            ty: Ty::new_ptr(elem, mutability),
            span: expr.span,
        }
    }

    pub(crate) fn fn_def_to_c_fn_ptr_decay_if_fn_def(&self, expr: &mut HirExpr) {
        if !matches!(expr.ty.kind(), TyKind::RigidTy(RigidTy::FnDef(..))) {
            return;
        }
        *expr = self.fn_def_to_c_fn_ptr_decay(expr.clone());
    }

    fn fn_def_to_fn_ptr_decay(&self, expr: HirExpr) -> HirExpr {
        let sig = expr
            .ty
            .kind()
            .fn_sig()
            .expect("FnDef should have fn signature");
        return HirExpr {
            span: expr.span,
            kind: HirExprKind::Cast(Box::new(expr)),
            ty: Ty::from_rigid_kind(RigidTy::FnPtr(sig)),
        };
    }

    fn fn_def_to_c_fn_ptr_decay(&self, expr: HirExpr) -> HirExpr {
        let sig = expr
            .ty
            .kind()
            .fn_sig()
            .expect("FnDef should have fn signature");
        let ty = Ty::from_rigid_kind(RigidTy::FnPtr(sig));
        let ty = self.maybe_uninit_of(ty);
        return HirExpr {
            span: expr.span,
            kind: HirExprKind::Cast(Box::new(expr)),
            ty,
        };
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
                co2_crate_sig::DefOrLocal::Def {
                    def_id,
                    generic_args,
                } => {
                    if self.function_name.is_none() && self.decl_resolver.is_constexpr_def(def_id) {
                        let value =
                            self.decl_resolver
                                .local_const_int_value(def_id)
                                .map_err(|_| {
                                    format!("missing scalar constant value for def {def_id:?}")
                                })?;
                        return Ok(HirExpr {
                            kind: HirExprKind::ConstInt(value),
                            ty: Ty::signed_ty(IntTy::I32),
                            span,
                        });
                    }
                    let resolved = self.resolve_value_with_generic_args(def_id, &generic_args);
                    Ok(HirExpr {
                        kind: HirExprKind::Path(resolved.clone()),
                        ty: resolved.ty(),
                        span,
                    })
                }
                co2_crate_sig::DefOrLocal::Const(def_id) => {
                    let resolver = &self.decl_resolver;
                    if resolver.has_local_const_value(def_id) {
                        let value = resolver.local_const_int_value(def_id).map_err(|_| {
                            format!("missing scalar constant value for def {def_id:?}")
                        })?;
                        Ok(HirExpr {
                            kind: HirExprKind::ConstInt(value),
                            ty: Ty::signed_ty(IntTy::I32),
                            span,
                        })
                    } else if let Some(value) = resolver.dependency_const_value(def_id) {
                        Ok(lower_dependency_const(value, span))
                    } else {
                        Err(format!("missing scalar constant value for def {def_id:?}"))
                    }
                }
                co2_crate_sig::DefOrLocal::AssocMethod { .. } => self.terminate_with_error(
                    parser_span,
                    "associated method path is only valid in call position",
                ),
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
                co2_crate_sig::DefOrLocal::LocalConst(l) => {
                    let Some(&local) = local_map.get(&(l as usize)) else {
                        let value = self
                            .decl_resolver
                            .local_constexpr_int_value(l)
                            .map_err(|_| format!("missing scalar constant value for local {l}"))?;
                        return Ok(HirExpr {
                            kind: HirExprKind::ConstInt(value),
                            ty: Ty::signed_ty(IntTy::I32),
                            span,
                        });
                    };
                    let local_decl = &locals[local];
                    return Ok(HirExpr {
                        kind: HirExprKind::LocalConst(local),
                        ty: local_decl.ty,
                        span,
                    });
                }
                co2_crate_sig::DefOrLocal::FuncName => Ok(HirExpr {
                    kind: HirExprKind::ConstStr(self.function_name.clone().unwrap_or_else(|| {
                        self.terminate_with_error(
                            parser_span,
                            "__func__ is only available inside function bodies",
                        )
                    })),
                    ty: Ty::new_ptr(Ty::signed_ty(IntTy::I8), Mutability::Mut),
                    span,
                }),
                co2_crate_sig::DefOrLocal::Prim(_)
                | co2_crate_sig::DefOrLocal::UnrepresentableType(_) => {
                    panic!("Invalid type in expression")
                }
            },
            Expression::LabelAddress(label) => {
                if self.function_name.is_none() {
                    return Err("GNU label address is only valid inside a function body".to_owned());
                }
                Ok(HirExpr {
                    kind: HirExprKind::LabelAddress(self.resolve_or_insert_label(label.0)),
                    ty: Ty::usize_ty(),
                    span,
                })
            }
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
                let (func_expr, sig, mut lowered_args) = if let Some(lowered) =
                    self.try_lower_method_call(&func, &params, locals, local_map)?
                {
                    lowered
                } else if let Some(lowered) =
                    self.try_lower_assoc_method_call(&func, &params, locals, local_map)?
                {
                    lowered
                } else {
                    let func_expr = self.lower_expr((func.0, func.1), locals, local_map)?;
                    let Some(sig) = callable_sig(func_expr.ty) else {
                        self.terminate_with_error(parser_span, "Type is not callable");
                    };

                    let sig = rustc_public_generative::erase_late_bound_regions_in_fn_sig(sig);

                    let mut lowered_args = Vec::with_capacity(params.len());
                    for param in params {
                        let mut arg = self.lower_expr((param.0, param.1), locals, local_map)?;
                        self.array_to_pointer_decay_if_array(&mut arg);
                        lowered_args.push(arg);
                    }
                    (func_expr, sig, lowered_args)
                };

                self.lower_call_args(parser_span, &sig, &mut lowered_args);

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
                match self
                    .resolve_struct_field_access(base.ty, &field.0)
                    .ok_or_else(|| format!("unknown field `{}` on type {:?}", field.0, base.ty))?
                {
                    ResolvedFieldAccess::Direct { path, ty } => {
                        self.project_field_path(base, &path, ty, span)
                    }
                    ResolvedFieldAccess::Bitfield {
                        container_path,
                        storage_index,
                        storage_ty,
                        bit_offset,
                        bit_width,
                        signed,
                        ty,
                    } => {
                        let container_ty = self.field_path_ty(base.ty, &container_path)?;
                        let base =
                            self.project_field_path(base, &container_path, container_ty, span)?;
                        Ok(HirExpr {
                            kind: HirExprKind::Bitfield {
                                base: Box::new(base),
                                storage_index,
                                storage_ty,
                                bit_offset,
                                bit_width,
                                signed,
                            },
                            ty,
                            span,
                        })
                    }
                }
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
                match self
                    .resolve_struct_field_access(deref_base.ty, &field.0)
                    .ok_or_else(|| {
                        format!("unknown field `{}` on type {:?}", field.0, deref_base.ty)
                    })? {
                    ResolvedFieldAccess::Direct { path, ty } => {
                        self.project_field_path(deref_base, &path, ty, span)
                    }
                    ResolvedFieldAccess::Bitfield {
                        container_path,
                        storage_index,
                        storage_ty,
                        bit_offset,
                        bit_width,
                        signed,
                        ty,
                    } => {
                        let container_ty = self.field_path_ty(deref_base.ty, &container_path)?;
                        let deref_base = self.project_field_path(
                            deref_base,
                            &container_path,
                            container_ty,
                            span,
                        )?;
                        Ok(HirExpr {
                            kind: HirExprKind::Bitfield {
                                base: Box::new(deref_base),
                                storage_index,
                                storage_ty,
                                bit_offset,
                                bit_width,
                                signed,
                            },
                            ty,
                            span,
                        })
                    }
                }
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
                    let lhs_span = lhs.1;
                    let lhs = self.lower_expr(*lhs, locals, local_map)?;
                    if !is_assignable_expr(&lhs) {
                        if let Some(name) = readonly_constexpr_name(&lhs, locals) {
                            self.terminate_with_error(
                                lhs_span,
                                &format!("assignment of read-only constexpr variable `{name}`"),
                            );
                        }
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
                let lhs_span = lhs.1;
                let lhs = self.lower_expr(*lhs, locals, local_map)?;
                if !is_assignable_expr(&lhs) {
                    if let Some(name) = readonly_constexpr_name(&lhs, locals) {
                        self.terminate_with_error(
                            lhs_span,
                            &format!("assignment of read-only constexpr variable `{name}`"),
                        );
                    }
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
                if !is_assignable_expr(&lhs) {
                    if let Some(name) = readonly_constexpr_name(&lhs, locals) {
                        self.terminate_with_error(
                            parser_span,
                            &format!("update of read-only constexpr variable `{name}`"),
                        );
                    }
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
                            RigidTy::RawPtr(..) => {
                                (Ty::signed_ty(IntTy::Isize), HirExprKind::ConstInt(1))
                            }
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
                let dst_is_void =
                    matches!(target_ty.kind(), TyKind::RigidTy(RigidTy::Tuple(l)) if l.is_empty());
                if !((src_is_int && dst_is_int)
                    || (src_is_ptr_like && dst_is_ptr_like)
                    || (src_is_int && dst_is_ptr_like)
                    || (src_is_ptr_like && dst_is_int)
                    || dst_is_void
                    || (src_is_fn_item && dst_is_int)
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
            Expression::Offsetof {
                ty: type_name,
                field,
            } => {
                let ty = self.lower_type_name(*type_name, parser_span)?;
                let indices =
                    match self
                        .resolve_struct_field_access(ty, &field)
                        .unwrap_or_else(|| {
                            self.terminate_with_error(
                                parser_span,
                                &format!("offsetof: field '{field}' not found in type"),
                            )
                        }) {
                        ResolvedFieldAccess::Direct { path, .. } => path,
                        ResolvedFieldAccess::Bitfield { .. } => {
                            self.terminate_with_error(
                                parser_span,
                                &format!("offsetof: field '{field}' is a bitfield"),
                            );
                        }
                    };
                // Walk the type hierarchy following `indices` to accumulate the byte offset.
                let mut offset_bytes: u64 = 0;
                let mut cur_ty = ty;
                for &field_idx in &indices {
                    let layout = cur_ty.layout().unwrap_or_else(|e| {
                        self.terminate_with_error(
                            parser_span,
                            &format!("offsetof: failed to compute layout: {e}"),
                        )
                    });
                    let field_offset = match &layout.shape().fields {
                        FieldsShape::Arbitrary { offsets, .. } => {
                            let off = offsets
                                .get(field_idx)
                                .unwrap_or_else(|| {
                                    self.terminate_with_error(
                                        parser_span,
                                        &format!("offsetof: field index {field_idx} out of bounds"),
                                    )
                                })
                                .bytes();
                            off
                        }
                        other => {
                            self.terminate_with_error(
                                parser_span,
                                &format!("offsetof: unsupported layout kind for type: {other:?}"),
                            );
                        }
                    };
                    offset_bytes += field_offset as u64;
                    // Descend into the field type for next iteration.
                    cur_ty = adt_field_tys(cur_ty)
                        .and_then(|tys| tys.into_iter().nth(field_idx))
                        .unwrap_or_else(|| {
                            self.terminate_with_error(
                                parser_span,
                                &format!("offsetof: failed to get field type at index {field_idx}"),
                            )
                        });
                }
                Ok(HirExpr {
                    kind: HirExprKind::ConstInt(offset_bytes as i128),
                    ty: Ty::usize_ty(),
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
                            return Ok(self.fn_def_to_fn_ptr_decay(inner));
                        }
                        if !is_place_expr(&inner) {
                            if matches!(
                                inner.kind,
                                HirExprKind::Aggregate { .. } | HirExprKind::Zeroed
                            ) {
                                return Ok(HirExpr {
                                    kind: HirExprKind::AddrOf(Box::new(inner.clone())),
                                    ty: Ty::new_ptr(inner.ty, addr_of_mutability(&inner)),
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
                            ty: Ty::new_ptr(inner.ty, addr_of_mutability(&inner)),
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
                let parser_span = body.1;
                let mut parsed = body.0;
                let tail = if let Some(last_item) = parsed.statements.pop() {
                    if let StatementOrDeclaration::Statement((Statement::Expression(tail), _)) =
                        last_item.0
                    {
                        tail
                    } else {
                        parsed.statements.push(last_item);
                        (Expression::Empty, parser_span)
                    }
                } else {
                    (Expression::Empty, parser_span)
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
            Expression::VaStart {
                args,
                last_param: _,
            } => {
                let args = Box::new(self.lower_expr(*args, locals, local_map)?);
                Ok(HirExpr {
                    kind: HirExprKind::VaStart(args),
                    ty: Ty::new_tuple(&[]),
                    span,
                })
            }
            Expression::VaArg { args, type_name } => {
                let args = Box::new(self.lower_expr(*args, locals, local_map)?);
                let ty = self.lower_type_name(type_name, parser_span)?;
                Ok(HirExpr {
                    kind: HirExprKind::VaArg(args),
                    ty,
                    span,
                })
            }
            Expression::VaEnd { args } => {
                let args = Box::new(self.lower_expr(*args, locals, local_map)?);
                Ok(HirExpr {
                    kind: HirExprKind::VaEnd(args),
                    ty: Ty::new_tuple(&[]),
                    span,
                })
            }
            Expression::GenericSelection {
                controlling,
                associations,
            } => {
                let mut controlling_expr =
                    self.lower_expr((controlling.0.clone(), controlling.1), locals, local_map)?;
                self.array_to_pointer_decay_if_array(&mut controlling_expr);
                self.fn_def_to_c_fn_ptr_decay_if_fn_def(&mut controlling_expr);
                let controlling_ty = controlling_expr.ty;

                let mut default_expr = None;
                for (assoc, assoc_span) in associations {
                    match assoc {
                        GenericAssociation::Default { expr } => {
                            if default_expr.is_some() {
                                self.terminate_with_error(
                                    assoc_span,
                                    "duplicate default association in _Generic",
                                );
                            }
                            default_expr = Some(expr);
                        }
                        GenericAssociation::Type { type_name, expr } => {
                            let assoc_ty = self.lower_type_name(type_name, assoc_span)?;
                            if ty_matches_expected(assoc_ty, controlling_ty) {
                                return self.lower_expr(expr, locals, local_map);
                            }
                        }
                    }
                }
                if let Some(expr) = default_expr {
                    self.lower_expr(expr, locals, local_map)
                } else {
                    self.terminate_with_error(
                        parser_span,
                        "no matching association in _Generic and no default provided",
                    );
                }
            }
            Expression::Empty => Ok(HirExpr {
                kind: HirExprKind::Zeroed,
                ty: Ty::new_tuple(&[]),
                span,
            }),
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

                self.fn_def_to_c_fn_ptr_decay_if_fn_def(&mut then_expr);
                self.fn_def_to_c_fn_ptr_decay_if_fn_def(&mut else_expr);

                let common_ty = if then_expr.is_null_like() {
                    else_expr.ty
                } else if else_expr.is_null_like() {
                    then_expr.ty
                } else if let Some(common_ty) = common_ternary_ty(then_expr.ty, else_expr.ty) {
                    common_ty
                } else {
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

    pub(crate) fn adt_logical_field_tys(&self, ty: Ty) -> Option<Vec<Ty>> {
        let TyKind::RigidTy(RigidTy::Adt(def, _)) = ty.kind() else {
            return None;
        };
        self.decl_resolver.adt_logical_fields(def.0).map(|fields| {
            fields
                .into_iter()
                .map(|field| hir_ty_to_ty(&field.ty))
                .collect()
        })
    }

    pub(crate) fn resolve_logical_field_path(
        &self,
        ty: Ty,
        field: &str,
    ) -> Option<(Vec<usize>, Ty)> {
        let TyKind::RigidTy(RigidTy::Adt(def, _)) = ty.kind() else {
            return resolve_field_path_in_adt(ty, field);
        };
        self.resolve_logical_field_path_from_metadata(def.0, field)
            .map(|(path, ty)| (path, hir_ty_to_ty(&ty)))
            .or_else(|| resolve_field_path_in_adt(ty, field))
    }

    fn resolve_logical_field_path_from_metadata(
        &self,
        def: rustc_public_generative::rustc_public::DefId,
        field: &str,
    ) -> Option<(Vec<usize>, rustc_public_generative::HirTy)> {
        let fields = self.decl_resolver.adt_logical_fields(def)?;
        for (idx, logical_field) in fields.iter().enumerate() {
            if logical_field.name == field && !logical_field.name.starts_with("__anon_field_") {
                return Some((vec![idx], logical_field.ty.clone()));
            }
        }
        for (idx, logical_field) in fields.into_iter().enumerate() {
            if !logical_field.name.starts_with("__anon_field_") {
                continue;
            }
            let rustc_public_generative::HirTyKind::Adt(nested_def, _) = logical_field.ty.kind
            else {
                continue;
            };
            if let Some((mut path, ty)) =
                self.resolve_logical_field_path_from_metadata(nested_def, field)
            {
                path.insert(0, idx);
                return Some((path, ty));
            }
        }
        None
    }

    fn resolve_struct_field_access(&self, ty: Ty, field: &str) -> Option<ResolvedFieldAccess> {
        let TyKind::RigidTy(RigidTy::Adt(def, _)) = ty.kind() else {
            return resolve_field_path_in_adt(ty, field)
                .map(|(path, ty)| ResolvedFieldAccess::Direct { path, ty });
        };
        self.resolve_struct_field_access_from_metadata(def.0, field)
            .or_else(|| {
                resolve_field_path_in_adt(ty, field)
                    .map(|(path, ty)| ResolvedFieldAccess::Direct { path, ty })
            })
    }

    fn resolve_struct_field_access_from_metadata(
        &self,
        def: rustc_public_generative::rustc_public::DefId,
        field: &str,
    ) -> Option<ResolvedFieldAccess> {
        let fields = self.decl_resolver.adt_logical_fields(def)?;
        for logical_field in &fields {
            if logical_field.name != field || logical_field.name.starts_with("__anon_field_") {
                continue;
            }
            return Some(match &logical_field.kind {
                LogicalAdtFieldKind::Direct { physical_index } => ResolvedFieldAccess::Direct {
                    path: vec![*physical_index],
                    ty: hir_ty_to_ty(&logical_field.ty),
                },
                LogicalAdtFieldKind::Bitfield {
                    storage_index,
                    storage_ty,
                    bit_offset,
                    bit_width,
                    is_signed,
                } => ResolvedFieldAccess::Bitfield {
                    container_path: vec![],
                    storage_index: *storage_index,
                    storage_ty: hir_ty_to_ty(storage_ty),
                    bit_offset: *bit_offset,
                    bit_width: *bit_width,
                    signed: *is_signed,
                    ty: hir_ty_to_ty(&logical_field.ty),
                },
            });
        }
        for logical_field in fields {
            let LogicalAdtFieldKind::Direct { physical_index } = logical_field.kind else {
                continue;
            };
            if !logical_field.name.starts_with("__anon_field_") {
                continue;
            }
            let rustc_public_generative::HirTyKind::Adt(nested_def, _) = logical_field.ty.kind
            else {
                continue;
            };
            if let Some(resolved) =
                self.resolve_struct_field_access_from_metadata(nested_def, field)
            {
                return Some(match resolved {
                    ResolvedFieldAccess::Direct { mut path, ty } => {
                        path.insert(0, physical_index);
                        ResolvedFieldAccess::Direct { path, ty }
                    }
                    ResolvedFieldAccess::Bitfield {
                        mut container_path,
                        storage_index,
                        storage_ty,
                        bit_offset,
                        bit_width,
                        signed,
                        ty,
                    } => {
                        container_path.insert(0, physical_index);
                        ResolvedFieldAccess::Bitfield {
                            container_path,
                            storage_index,
                            storage_ty,
                            bit_offset,
                            bit_width,
                            signed,
                            ty,
                        }
                    }
                });
            }
        }
        None
    }

    fn field_path_ty(&self, mut base_ty: Ty, path: &[usize]) -> Result<Ty, String> {
        for index in path {
            let Some(field_tys) = adt_field_tys(base_ty) else {
                return Err(format!("field projection on non-adt type: {:?}", base_ty));
            };
            base_ty = *field_tys
                .get(*index)
                .ok_or_else(|| format!("field index out of bounds: {} for {:?}", index, base_ty))?;
        }
        Ok(base_ty)
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
    if let Some(inner) = enum_payload_ty(ty) {
        return ty_passed_to_variadic(inner);
    }
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
    pub(crate) fn lower_binop_from_lowered(
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

        if matches!(op, HirBinOp::Shl | HirBinOp::Shr) {
            let common_ty = if let Some(inner) = enum_payload_ty(lhs.ty) {
                inner
            } else {
                match lhs.ty.kind() {
                    TyKind::RigidTy(rigid_ty) => match rigid_ty {
                        RigidTy::Int(int_ty) => Ty::signed_ty(match int_ty {
                            IntTy::I8 | IntTy::I16 => IntTy::I32,
                            _ => int_ty,
                        }),
                        RigidTy::Uint(uint_ty) => Ty::unsigned_ty(match uint_ty {
                            UintTy::U8 | UintTy::U16 => UintTy::U32,
                            _ => uint_ty,
                        }),
                        _ => return Err("Invalid type for shift".to_owned()),
                    },
                    _ => unreachable!(),
                }
            };
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
                        return Err("type error: adding two pointers is invalid".to_owned());
                    }
                    _ => {}
                }
            } else {
                match (lhs_is_ptr, rhs_is_ptr) {
                    (true, false) if is_numeric_ty(rhs.ty) => {
                        let offset_ty = Ty::signed_ty(IntTy::Isize);
                        let neg_rhs = HirExpr {
                            kind: HirExprKind::Binary {
                                op: HirBinOp::Sub,
                                lhs: Box::new(HirExpr {
                                    kind: HirExprKind::ConstInt(0),
                                    ty: offset_ty,
                                    span,
                                }),
                                rhs: Box::new(HirExpr {
                                    kind: HirExprKind::Cast(Box::new(rhs)),
                                    ty: offset_ty,
                                    span,
                                }),
                            },
                            ty: offset_ty,
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

        if is_numeric_ty(lhs.ty)
            && is_numeric_ty(rhs.ty)
            && !matches!(op, HirBinOp::Shl | HirBinOp::Shr)
            && (lhs.ty != rhs.ty
                || lhs.ty.kind().is_bool()
                || enum_payload_ty(lhs.ty).is_some()
                || enum_payload_ty(rhs.ty).is_some())
        {
            let Some(mut common_ty) = common_numeric_ty(lhs.ty, rhs.ty) else {
                return Err("failed to find common ty in binop".to_owned());
            };
            if common_ty.kind().is_bool() {
                common_ty = Ty::signed_ty(IntTy::I32);
            }
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

    fn zeroed_expr(&self, ty: Ty, span: RustSpan) -> HirExpr {
        HirExpr {
            kind: HirExprKind::Zeroed,
            ty,
            span,
        }
    }

    fn const_int_expr(&self, value: i128, ty: Ty, span: RustSpan) -> HirExpr {
        HirExpr {
            kind: HirExprKind::ConstInt(value),
            ty,
            span,
        }
    }

    fn pack_bitfield_initializer(
        &self,
        current_storage: HirExpr,
        value: HirExpr,
        storage_ty: Ty,
        bit_offset: usize,
        bit_width: usize,
        span: RustSpan,
    ) -> HirExpr {
        let value_mask = if bit_width >= 128 {
            self.emit_cast(
                self.const_int_expr(-1, Ty::signed_ty(IntTy::I32), span),
                storage_ty,
            )
        } else {
            self.emit_cast(
                self.const_int_expr(
                    ((1u128 << bit_width) - 1) as i128,
                    Ty::signed_ty(IntTy::I32),
                    span,
                ),
                storage_ty,
            )
        };
        let field_mask = if bit_offset == 0 {
            value_mask.clone()
        } else {
            HirExpr {
                kind: HirExprKind::Binary {
                    op: HirBinOp::Shl,
                    lhs: Box::new(value_mask.clone()),
                    rhs: Box::new(self.emit_cast(
                        self.const_int_expr(bit_offset as i128, Ty::signed_ty(IntTy::I32), span),
                        storage_ty,
                    )),
                },
                ty: storage_ty,
                span,
            }
        };
        let cleared = HirExpr {
            kind: HirExprKind::Binary {
                op: HirBinOp::BitAnd,
                lhs: Box::new(current_storage),
                rhs: Box::new(HirExpr {
                    kind: HirExprKind::BitNot(Box::new(field_mask.clone())),
                    ty: storage_ty,
                    span,
                }),
            },
            ty: storage_ty,
            span,
        };
        let masked_value = HirExpr {
            kind: HirExprKind::Binary {
                op: HirBinOp::BitAnd,
                lhs: Box::new(self.emit_cast(value, storage_ty)),
                rhs: Box::new(value_mask),
            },
            ty: storage_ty,
            span,
        };
        let shifted_value = if bit_offset == 0 {
            masked_value
        } else {
            HirExpr {
                kind: HirExprKind::Binary {
                    op: HirBinOp::Shl,
                    lhs: Box::new(masked_value),
                    rhs: Box::new(self.emit_cast(
                        self.const_int_expr(bit_offset as i128, Ty::signed_ty(IntTy::I32), span),
                        storage_ty,
                    )),
                },
                ty: storage_ty,
                span,
            }
        };
        HirExpr {
            kind: HirExprKind::Binary {
                op: HirBinOp::BitOr,
                lhs: Box::new(cleared),
                rhs: Box::new(shifted_value),
            },
            ty: storage_ty,
            span,
        }
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
                    if let TyKind::RigidTy(RigidTy::Adt(def, _)) = ty.kind() {
                        if let Some(logical_fields) = self.decl_resolver.adt_logical_fields(def.0)
                            && logical_fields.iter().any(|field| {
                                matches!(field.kind, LogicalAdtFieldKind::Bitfield { .. })
                            })
                        {
                            let Some(physical_field_tys) = adt_field_tys(ty) else {
                                self.terminate_with_error(parser_span, "Can't compute adt fields");
                            };
                            let mut physical_args = vec![None::<HirExpr>; physical_field_tys.len()];
                            for (child, logical_field) in children.iter().zip(logical_fields.iter())
                            {
                                match &logical_field.kind {
                                    LogicalAdtFieldKind::Direct { physical_index } => {
                                        physical_args[*physical_index] =
                                            Some(self.initializer_tree_to_expr(
                                                child,
                                                physical_field_tys[*physical_index],
                                                parser_span,
                                            ));
                                    }
                                    LogicalAdtFieldKind::Bitfield {
                                        storage_index,
                                        storage_ty,
                                        bit_offset,
                                        bit_width,
                                        ..
                                    } => {
                                        let storage_ty = hir_ty_to_ty(storage_ty);
                                        let current_storage = physical_args[*storage_index]
                                            .take()
                                            .unwrap_or_else(|| self.zeroed_expr(storage_ty, span));
                                        let value = self.initializer_tree_to_expr(
                                            child,
                                            hir_ty_to_ty(&logical_field.ty),
                                            parser_span,
                                        );
                                        physical_args[*storage_index] =
                                            Some(self.pack_bitfield_initializer(
                                                current_storage,
                                                value,
                                                storage_ty,
                                                *bit_offset,
                                                *bit_width,
                                                span,
                                            ));
                                    }
                                }
                            }
                            let args = physical_field_tys
                                .into_iter()
                                .enumerate()
                                .map(|(idx, field_ty)| {
                                    physical_args[idx]
                                        .take()
                                        .unwrap_or_else(|| self.zeroed_expr(field_ty, span))
                                })
                                .collect();
                            return HirExpr {
                                kind: HirExprKind::Aggregate { args },
                                ty,
                                span,
                            };
                        }
                    }
                    let Some(field_tys) = adt_field_tys(ty) else {
                        self.terminate_with_error(parser_span, "Can't compute adt fields");
                    };
                    if matches!(ty.kind(), TyKind::RigidTy(RigidTy::Adt(adt, _)) if adt.kind().is_union())
                    {
                        let Some((active_field, child)) = children
                            .iter()
                            .enumerate()
                            .find(|(_, child)| !matches!(child, InitializerTree::Zeroed))
                        else {
                            return HirExpr {
                                kind: HirExprKind::Zeroed,
                                ty,
                                span,
                            };
                        };
                        let field_ty = field_tys[active_field];
                        let arg = self.initializer_tree_to_expr(child, field_ty, parser_span);
                        return HirExpr {
                            kind: HirExprKind::UnionAggregate {
                                active_field,
                                arg: Box::new(arg),
                            },
                            ty,
                            span,
                        };
                    }
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
        HirExprKind::Local(_) | HirExprKind::LocalConst(_) => true,
        HirExprKind::Field { base, .. } => is_place_expr(base),
        HirExprKind::Path(ResolvedValue::Static { .. } | ResolvedValue::StaticConst(_)) => true,
        HirExprKind::PtrOffset { .. } => false,
        HirExprKind::PtrDiff { .. } => false,
        HirExprKind::Logical { .. } => false,
        HirExprKind::LogicalNot(_) => false,
        HirExprKind::BitNot(_) => false,
        HirExprKind::Deref(_) => true,
        _ => false,
    }
}

fn is_assignable_expr(expr: &HirExpr) -> bool {
    match &expr.kind {
        HirExprKind::LocalConst(_) | HirExprKind::Path(ResolvedValue::StaticConst(_)) => false,
        _ => is_place_expr(expr) || matches!(expr.kind, HirExprKind::Bitfield { .. }),
    }
}

fn readonly_constexpr_name(expr: &HirExpr, locals: &Arena<HirLocal>) -> Option<String> {
    match &expr.kind {
        HirExprKind::LocalConst(local) => Some(locals[*local].name.clone()),
        _ => None,
    }
}

fn addr_of_mutability(expr: &HirExpr) -> Mutability {
    match &expr.kind {
        HirExprKind::Deref(inner) => match inner.ty.kind() {
            TyKind::RigidTy(RigidTy::RawPtr(_, mutability)) => mutability,
            _ => Mutability::Mut,
        },
        HirExprKind::Field { base, .. } => addr_of_mutability(base),
        HirExprKind::LocalConst(_) => Mutability::Not,
        HirExprKind::Path(ResolvedValue::StaticConst(_)) => Mutability::Not,
        _ => Mutability::Mut,
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
        }
        IntegerSuffix::Long | IntegerSuffix::LongLong => Ty::signed_ty(IntTy::I64),
        IntegerSuffix::Unsigned => Ty::unsigned_ty(UintTy::U32),
        IntegerSuffix::UnsignedLong | IntegerSuffix::UnsignedLongLong => {
            Ty::unsigned_ty(UintTy::U64)
        }
    }
}

pub(crate) fn coerce_expr_to_type(expr: HirExpr, expected_ty: Ty) -> Result<HirExpr, String> {
    if ty_matches_expected(expected_ty, expr.ty) {
        return Ok(expr);
    }
    if matches!(expr.kind, HirExprKind::ConstInt(0))
        && matches!(
            expected_ty.kind(),
            TyKind::RigidTy(RigidTy::RawPtr(..) | RigidTy::FnPtr(..))
        )
    {
        return Ok(HirExpr {
            kind: HirExprKind::Zeroed,
            ty: expected_ty,
            span: expr.span,
        });
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
