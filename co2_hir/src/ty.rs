use co2_parser::{Span, TypeSpecifier};
use rustc_public_generative::rustc_public::ty::{
    Binder, FnSig, GenericArgKind, IntTy, RigidTy, Ty, TyKind, UintTy, VariantIdx,
};

const ANON_FIELD_PREFIX: &str = "__anon_field_";

pub(crate) fn is_integer_ty(ty: Ty) -> bool {
    matches!(
        ty.kind(),
        TyKind::RigidTy(RigidTy::Int(_)) | TyKind::RigidTy(RigidTy::Uint(_))
    )
}

fn integer_rank(ty: Ty) -> Option<(u8, bool)> {
    match ty.kind() {
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
        _ => None,
    }
}

pub(crate) fn common_integer_ty(lhs: Ty, rhs: Ty) -> Option<Ty> {
    let (lhs_rank, lhs_unsigned) = integer_rank(lhs)?;
    let (rhs_rank, rhs_unsigned) = integer_rank(rhs)?;

    let (rank, unsigned) = if lhs_rank == rhs_rank {
        (lhs_rank, lhs_unsigned || rhs_unsigned)
    } else if lhs_rank > rhs_rank {
        (lhs_rank, lhs_unsigned)
    } else {
        (rhs_rank, rhs_unsigned)
    };

    let ty = match (rank, unsigned) {
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

pub(crate) fn is_sized_array_ty(ty: Ty) -> bool {
    matches!(ty.kind(), TyKind::RigidTy(RigidTy::Array(_, _)))
}

pub(crate) fn is_condition_ty(ty: Ty) -> bool {
    matches!(
        ty.kind(),
        TyKind::RigidTy(RigidTy::Int(_))
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
    let dst_is_mu_fn_ptr = is_maybe_uninit_fn_ptr_ty(dst).is_some();
    matches!(
        (dst.kind(), src.kind()),
        (
            TyKind::RigidTy(RigidTy::RawPtr(_, _)),
            TyKind::RigidTy(RigidTy::Int(_) | RigidTy::Uint(_))
        ) | (
            TyKind::RigidTy(RigidTy::RawPtr(_, _)),
            TyKind::RigidTy(RigidTy::RawPtr(_, _))
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
                RigidTy::Int(_) | RigidTy::Uint(_) | RigidTy::FnDef(_, _) | RigidTy::FnPtr(_)
            )
        ))
        || (is_integer_ty(dst) && is_integer_ty(src))
}

pub(crate) fn resolve_field_path_in_adt(base: Ty, field: &str) -> Option<(Vec<usize>, Ty)> {
    let TyKind::RigidTy(RigidTy::Adt(adt, args)) = base.kind() else {
        return None;
    };
    let adt_is_union = is_union_ty(base);
    let variant = adt.variant(variant_idx(0))?;
    let fields = variant.fields();
    for (idx, field_def) in fields.iter().enumerate() {
        let name = field_def.name.to_string();
        if name == field {
            let storage_idx = if adt_is_union { 0 } else { idx };
            return Some((vec![storage_idx], field_def.ty_with_args(&args)));
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

pub(crate) fn type_specifier_to_ty(type_specifier: &TypeSpecifier) -> Result<Option<Ty>, String> {
    let ty = match type_specifier {
        TypeSpecifier::Int => Some(Ty::signed_ty(IntTy::I32)),
        TypeSpecifier::Bool => Some(Ty::bool_ty()),
        TypeSpecifier::Void => Some(Ty::new_tuple(&[])),
        TypeSpecifier::Char => Some(Ty::signed_ty(IntTy::I8)),
        TypeSpecifier::Short => Some(Ty::signed_ty(IntTy::I16)),
        TypeSpecifier::Long => Some(Ty::signed_ty(IntTy::I64)),
        TypeSpecifier::Float => return Err("float is not supported".to_owned()),
        TypeSpecifier::Double => return Err("double is not supported".to_owned()),
        TypeSpecifier::Signed | TypeSpecifier::Unsigned => None,
        TypeSpecifier::Enum(_) => Some(Ty::signed_ty(IntTy::I32)),
        TypeSpecifier::StructOrUnion { .. } => {
            return Err("struct/union types are not supported yet".to_owned());
        }
        TypeSpecifier::TypedefName(_) => None,
    };
    Ok(ty)
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
    if let (TyKind::RigidTy(RigidTy::FnPtr(expected_sig)), TyKind::RigidTy(RigidTy::FnDef(_, _))) =
        (expected.kind(), actual.kind())
        && let Some(actual_sig) = actual.kind().fn_sig()
    {
        let expected_sig = expected_sig.skip_binder();
        let actual_sig = actual_sig.skip_binder();
        if expected_sig.inputs().len() != actual_sig.inputs().len() {
            return false;
        }
        return expected_sig
            .inputs()
            .iter()
            .zip(actual_sig.inputs().iter())
            .all(|(e, a)| ty_matches_expected(*e, *a))
            && ty_matches_expected(expected_sig.output(), actual_sig.output());
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
                    (
                        rustc_public_generative::rustc_public::ty::GenericArgKind::Type(et),
                        rustc_public_generative::rustc_public::ty::GenericArgKind::Type(at),
                    ) => ty_matches_expected(*et, *at),
                    _ => e == a,
                })
        }
        _ => false,
    }
}

pub(crate) fn no_type_specifier_err(span: Span) -> String {
    format!("no suitable type specifier at span {span:?}")
}
