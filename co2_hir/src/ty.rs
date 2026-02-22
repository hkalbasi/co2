use co2_parser::{Span, TypeSpecifier};
use rustc_public_generative::rustc_public::ty::{IntTy, RigidTy, Ty, TyKind, UintTy, VariantIdx};

pub(crate) fn is_integer_ty(ty: Ty) -> bool {
    matches!(
        ty.kind(),
        TyKind::RigidTy(RigidTy::Int(_)) | TyKind::RigidTy(RigidTy::Uint(_))
    )
}

pub(crate) fn is_condition_ty(ty: Ty) -> bool {
    matches!(
        ty.kind(),
        TyKind::RigidTy(RigidTy::Int(_))
            | TyKind::RigidTy(RigidTy::Uint(_))
            | TyKind::RigidTy(RigidTy::RawPtr(_, _))
    )
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
    match (expected.kind(), actual.kind()) {
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
