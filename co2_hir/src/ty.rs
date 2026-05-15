use rustc_public_generative::rustc_public::ty::{
    Binder, FloatTy, FnSig, GenericArgKind, IntTy, RigidTy, Ty, TyKind, UintTy, VariantIdx,
};

const ANON_FIELD_PREFIX: &str = "__anon_field_";
const ENUM_FIELD_NAME: &str = "__co2_enum_value";

pub(crate) fn enum_payload_ty(ty: Ty) -> Option<Ty> {
    let TyKind::RigidTy(RigidTy::Adt(adt, args)) = ty.kind() else {
        return None;
    };
    let variant = adt.variant(variant_idx(0))?;
    let fields = variant.fields();
    if fields.len() != 1 || fields[0].name.to_string() != ENUM_FIELD_NAME {
        return None;
    }
    Some(fields[0].ty_with_args(&args))
}

pub(crate) fn is_numeric_ty(ty: Ty) -> bool {
    if enum_payload_ty(ty).is_some() {
        return true;
    }
    matches!(
        ty.kind(),
        TyKind::RigidTy(RigidTy::Bool)
            | TyKind::RigidTy(RigidTy::Int(_))
            | TyKind::RigidTy(RigidTy::Uint(_))
            | TyKind::RigidTy(RigidTy::Float(_))
    )
}

fn numeric_rank(ty: Ty) -> Option<(u8, bool)> {
    if let Some(inner) = enum_payload_ty(ty) {
        return numeric_rank(inner);
    }
    match ty.kind() {
        TyKind::RigidTy(RigidTy::Bool) => Some((0, false)),
        TyKind::RigidTy(RigidTy::Int(int_ty)) => {
            let rank = match int_ty {
                IntTy::I8 => 1,
                IntTy::I16 => 2,
                IntTy::I32 => 3,
                IntTy::I64 => 4,
                IntTy::Isize => 5,
                IntTy::I128 => 6,
            };
            Some((rank, false))
        }
        TyKind::RigidTy(RigidTy::Uint(uint_ty)) => {
            let rank = match uint_ty {
                UintTy::U8 => 1,
                UintTy::U16 => 2,
                UintTy::U32 => 3,
                UintTy::U64 => 4,
                UintTy::Usize => 5,
                UintTy::U128 => 6,
            };
            Some((rank, true))
        }
        TyKind::RigidTy(RigidTy::Float(float_ty)) => {
            let rank = match float_ty {
                FloatTy::F16 => 101,
                FloatTy::F32 => 102,
                FloatTy::F64 => 103,
                FloatTy::F128 => 104,
            };
            Some((rank, false))
        }
        _ => None,
    }
}

pub(crate) fn common_ternary_ty(lhs_ty: Ty, rhs_ty: Ty) -> Option<Ty> {
    if lhs_ty == rhs_ty {
        return Some(lhs_ty);
    }
    if let Some(r) = common_numeric_ty(lhs_ty, rhs_ty) {
        return Some(r);
    }
    let TyKind::RigidTy(lhs) = lhs_ty.kind() else {
        return None;
    };
    let TyKind::RigidTy(rhs) = rhs_ty.kind() else {
        return None;
    };
    let lhs_is_void = matches!(lhs, RigidTy::Tuple(ref l) if l.is_empty());
    let rhs_is_void = matches!(rhs, RigidTy::Tuple(ref l) if l.is_empty());
    let one_is_void = lhs_is_void || rhs_is_void;

    if one_is_void {
        return Some(Ty::new_tuple(&[]));
    }

    if let (
        RigidTy::RawPtr(lhs_pointee, lhs_mutability),
        RigidTy::RawPtr(rhs_pointee, rhs_mutability),
    ) = (lhs, rhs)
    {
        let common_mutability = if matches!(
            (lhs_mutability, rhs_mutability),
            (
                rustc_public_generative::rustc_public::mir::Mutability::Not,
                _
            ) | (
                _,
                rustc_public_generative::rustc_public::mir::Mutability::Not
            )
        ) {
            rustc_public_generative::rustc_public::mir::Mutability::Not
        } else {
            rustc_public_generative::rustc_public::mir::Mutability::Mut
        };

        if lhs_pointee == rhs_pointee {
            return Some(Ty::new_ptr(lhs_pointee, common_mutability));
        }

        let lhs_is_void_pointee =
            matches!(lhs_pointee.kind(), TyKind::RigidTy(RigidTy::Tuple(l)) if l.is_empty());
        let rhs_is_void_pointee =
            matches!(rhs_pointee.kind(), TyKind::RigidTy(RigidTy::Tuple(l)) if l.is_empty());
        if lhs_is_void_pointee || rhs_is_void_pointee {
            return Some(Ty::new_ptr(Ty::new_tuple(&[]), common_mutability));
        }
    }
    None
}

pub(crate) fn common_numeric_ty(lhs: Ty, rhs: Ty) -> Option<Ty> {
    let (lhs_rank, lhs_unsigned) = numeric_rank(lhs)?;
    let (rhs_rank, rhs_unsigned) = numeric_rank(rhs)?;

    let (rank, unsigned) = if lhs_rank == rhs_rank {
        (lhs_rank, lhs_unsigned || rhs_unsigned)
    } else if lhs_rank > rhs_rank {
        (lhs_rank, lhs_unsigned)
    } else {
        (rhs_rank, rhs_unsigned)
    };

    let ty = match (rank, unsigned) {
        (0, _) => Ty::bool_ty(),
        (1, false) => Ty::signed_ty(IntTy::I8),
        (2, false) => Ty::signed_ty(IntTy::I16),
        (3, false) => Ty::signed_ty(IntTy::I32),
        (4, false) => Ty::signed_ty(IntTy::I64),
        (5, false) => Ty::signed_ty(IntTy::Isize),
        (6, false) => Ty::signed_ty(IntTy::I128),
        (1, true) => Ty::unsigned_ty(UintTy::U8),
        (2, true) => Ty::unsigned_ty(UintTy::U16),
        (3, true) => Ty::unsigned_ty(UintTy::U32),
        (4, true) => Ty::unsigned_ty(UintTy::U64),
        (5, true) => Ty::unsigned_ty(UintTy::Usize),
        (6, true) => Ty::unsigned_ty(UintTy::U128),
        (101, _) => Ty::from_rigid_kind(RigidTy::Float(FloatTy::F16)),
        (102, _) => Ty::from_rigid_kind(RigidTy::Float(FloatTy::F32)),
        (103, _) => Ty::from_rigid_kind(RigidTy::Float(FloatTy::F64)),
        (104, _) => Ty::from_rigid_kind(RigidTy::Float(FloatTy::F128)),
        _ => return None,
    };
    Some(ty)
}

pub(crate) fn array_elem_ty(ty: Ty) -> Option<Ty> {
    let TyKind::RigidTy(RigidTy::Array(elem, _)) = ty.kind() else {
        return None;
    };
    Some(elem)
}

pub(crate) fn is_array_ty(ty: Ty) -> bool {
    matches!(ty.kind(), TyKind::RigidTy(RigidTy::Array(_, _)))
}

pub(crate) fn is_condition_ty(ty: Ty) -> bool {
    if enum_payload_ty(ty).is_some() {
        return true;
    }
    matches!(
        ty.kind(),
        TyKind::RigidTy(RigidTy::Bool)
            | TyKind::RigidTy(RigidTy::Int(_))
            | TyKind::RigidTy(RigidTy::Uint(_))
            | TyKind::RigidTy(RigidTy::RawPtr(_, _))
            | TyKind::RigidTy(RigidTy::FnPtr(_))
            | TyKind::RigidTy(RigidTy::FnDef(_, _))
    ) || is_maybe_uninit_fn_ptr_ty(ty).is_some()
}

pub(crate) fn is_maybe_uninit_fn_ptr_ty(ty: Ty) -> Option<Binder<FnSig>> {
    let TyKind::RigidTy(RigidTy::Adt(_, args)) = ty.kind() else {
        return None;
    };
    if args.0.len() != 1 {
        return None;
    }
    let GenericArgKind::Type(inner) = args.0[0] else {
        return None;
    };
    let TyKind::RigidTy(RigidTy::FnPtr(sig)) = inner.kind() else {
        return None;
    };
    Some(sig)
}

pub(crate) fn callable_sig(ty: Ty) -> Option<Binder<FnSig>> {
    ty.kind().fn_sig().or_else(|| is_maybe_uninit_fn_ptr_ty(ty))
}

pub(crate) fn needs_implicit_cast(dst: Ty, src: Ty) -> bool {
    if dst == src {
        return false;
    }
    let src_is_mu_fn_ptr = is_maybe_uninit_fn_ptr_ty(src).is_some();
    let dst_is_mu_fn_ptr = is_maybe_uninit_fn_ptr_ty(dst).is_some();
    matches!(
        (dst.kind(), src.kind()),
        (TyKind::RigidTy(RigidTy::Bool), _) if is_condition_ty(src)
    ) || matches!(
        (dst.kind(), src.kind()),
        (
            TyKind::RigidTy(RigidTy::RawPtr(_, _)),
            TyKind::RigidTy(RigidTy::Int(_) | RigidTy::Uint(_))
        ) | (
            TyKind::RigidTy(RigidTy::RawPtr(_, _) | RigidTy::Ref(_, _, _)),
            TyKind::RigidTy(RigidTy::RawPtr(_, _) | RigidTy::Ref(_, _, _) | RigidTy::FnPtr(_))
        ) | (
            TyKind::RigidTy(RigidTy::FnPtr(_)),
            TyKind::RigidTy(RigidTy::FnDef(_, _))
        ) | (
            TyKind::RigidTy(RigidTy::FnPtr(_)),
            TyKind::RigidTy(RigidTy::Int(_) | RigidTy::Uint(_))
        )
    ) || (dst_is_mu_fn_ptr
        && matches!(
            src.kind(),
            TyKind::RigidTy(
                RigidTy::Int(_)
                    | RigidTy::Uint(_)
                    | RigidTy::FnDef(_, _)
                    | RigidTy::FnPtr(_)
                    | RigidTy::RawPtr(..)
            )
        ))
        || (src_is_mu_fn_ptr
            && matches!(
                dst.kind(),
                TyKind::RigidTy(RigidTy::Int(_) | RigidTy::Uint(_) | RigidTy::RawPtr(..))
            ))
        || (is_numeric_ty(dst) && is_numeric_ty(src))
}

pub(crate) fn resolve_field_path_in_adt(base: Ty, field: &str) -> Option<(Vec<usize>, Ty)> {
    let TyKind::RigidTy(RigidTy::Adt(adt, args)) = base.kind() else {
        return None;
    };
    let variant = adt.variant(variant_idx(0))?;
    let fields = variant.fields();
    for (idx, field_def) in fields.iter().enumerate() {
        let name = field_def.name.to_string();
        if name == field {
            return Some((vec![idx], field_def.ty_with_args(&args)));
        }
    }
    for (idx, field_def) in fields.iter().enumerate() {
        let name = field_def.name.to_string();
        if !name.starts_with(ANON_FIELD_PREFIX) {
            continue;
        }
        let nested_ty = field_def.ty_with_args(&args);
        if let Some((mut sub_path, nested_field_ty)) = resolve_field_path_in_adt(nested_ty, field) {
            sub_path.insert(0, idx);
            return Some((sub_path, nested_field_ty));
        }
    }
    None
}

pub(crate) fn is_union_ty(ty: Ty) -> bool {
    let TyKind::RigidTy(RigidTy::Adt(adt, _)) = ty.kind() else {
        return false;
    };
    adt.kind().is_union()
}

pub(crate) fn adt_field_tys(base: Ty) -> Option<Vec<Ty>> {
    if is_maybe_uninit_fn_ptr_ty(base).is_some() {
        return None;
    }
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

pub(crate) fn variant_idx(id: usize) -> VariantIdx {
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

pub(crate) fn ty_matches_expected(expected: Ty, actual: Ty) -> bool {
    if expected == actual {
        return true;
    }
    match (expected.kind(), actual.kind()) {
        (TyKind::RigidTy(RigidTy::Adt(_, exp_args)), TyKind::RigidTy(RigidTy::FnDef(_, _)))
            if exp_args.0.len() == 1 =>
        {
            if let GenericArgKind::Type(exp_inner) = exp_args.0[0] {
                return ty_matches_expected(exp_inner, actual);
            }
            false
        }
        (
            TyKind::RigidTy(RigidTy::Adt(exp_adt, exp_args)),
            TyKind::RigidTy(RigidTy::Adt(act_adt, act_args)),
        ) => {
            if exp_adt != act_adt {
                return false;
            }
            let shared_len = exp_args.0.len().min(act_args.0.len());
            exp_args
                .0
                .iter()
                .take(shared_len)
                .zip(act_args.0.iter().take(shared_len))
                .all(|(e, a)| match (e, a) {
                    (
                        rustc_public_generative::rustc_public::ty::GenericArgKind::Type(et),
                        rustc_public_generative::rustc_public::ty::GenericArgKind::Type(at),
                    ) => ty_matches_expected(*et, *at),
                    (
                        rustc_public_generative::rustc_public::ty::GenericArgKind::Lifetime(_),
                        rustc_public_generative::rustc_public::ty::GenericArgKind::Lifetime(_),
                    ) => true,
                    _ => e == a,
                })
                && extra_adt_args_are_concrete(&exp_args.0[shared_len..])
                && extra_adt_args_are_concrete(&act_args.0[shared_len..])
        }
        // Function pointer types may differ only in lifetime parameters (e.g. VaList<'erased> vs
        // VaList<'static>) because SMIR erases lifetimes inside fn ptrs. Compare structurally
        // while recursing into parameter/return types with ty_matches_expected.
        (
            TyKind::RigidTy(RigidTy::FnPtr(exp_binder)),
            TyKind::RigidTy(RigidTy::FnPtr(act_binder)),
        ) => {
            let exp_sig = exp_binder.value;
            let act_sig = act_binder.value;
            exp_sig.c_variadic == act_sig.c_variadic
                && exp_sig.safety == act_sig.safety
                && exp_sig.abi == act_sig.abi
                && exp_sig.inputs_and_output.len() == act_sig.inputs_and_output.len()
                && exp_sig
                    .inputs_and_output
                    .iter()
                    .zip(act_sig.inputs_and_output.iter())
                    .all(|(et, at)| ty_matches_expected(*et, *at))
        }
        _ => false,
    }
}

fn extra_adt_args_are_concrete(
    args: &[rustc_public_generative::rustc_public::ty::GenericArgKind],
) -> bool {
    args.iter().all(|arg| {
        !matches!(
            arg,
            rustc_public_generative::rustc_public::ty::GenericArgKind::Type(ty)
                if matches!(ty.kind(), TyKind::Param(_))
        )
    })
}
