use co2_parser::{Span, TypeSpecifier};
use rustc_public_generative::rustc_public::ty::{
    Binder, FnSig, GenericArgKind, IntTy, RigidTy, Ty, TyKind, UintTy, VariantIdx,
};

pub(crate) fn is_integer_ty(ty: Ty) -> bool {
    matches!(
        ty.kind(),
        TyKind::RigidTy(RigidTy::Int(_)) | TyKind::RigidTy(RigidTy::Uint(_))
    )
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

pub(crate) fn resolve_field_in_adt(base: Ty, field: &str) -> Option<(usize, Ty)> {
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
    if let (
        TyKind::RigidTy(RigidTy::FnPtr(expected_sig)),
        TyKind::RigidTy(RigidTy::FnDef(_, _)),
    ) = (expected.kind(), actual.kind())
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
        (
            TyKind::RigidTy(RigidTy::Adt(_, exp_args)),
            TyKind::RigidTy(RigidTy::FnDef(_, _)),
        ) if exp_args.0.len() == 1 => {
            if let GenericArgKind::Type(exp_inner) = exp_args.0[0] {
                return ty_matches_expected(exp_inner, actual);
            }
            false
        }
        (TyKind::Param(_), _) => true,
        (TyKind::RigidTy(RigidTy::Ref(_, exp_inner, _)), _) => ty_matches_expected(exp_inner, actual),
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

pub(crate) fn no_type_specifier_err(span: Span) -> String {
    format!("no suitable type specifier at span {span:?}")
}
