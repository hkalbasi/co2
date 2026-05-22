use co2_ast::StructOrUnionKind;
use co2_crate_sig::LocalResolver;
use rustc_public_generative::rustc_public::{
    CrateDef,
    mir::Mutability,
    ty::{Binder, FloatTy, FnSig, GenericArgKind, IntTy, RigidTy, Ty, TyKind, UintTy, VariantIdx},
};

use crate::resolver::HirCtx;

const ANON_FIELD_PREFIX: &str = "__anon_field_";
const ENUM_FIELD_NAME: &str = "__co2_enum_value";

impl HirCtx<'_> {
    pub(crate) fn format_ty(&self, ty: Ty) -> String {
        format_ty(Some(&self.decl_resolver), ty)
    }
}

pub fn format_ty(resolver: Option<&LocalResolver>, ty: Ty) -> String {
    match ty.kind() {
        TyKind::RigidTy(rigid) => format_rigid_ty(resolver, rigid),
        TyKind::Alias(_, _) | TyKind::Param(_) | TyKind::Bound(_, _) => format!("{ty:?}"),
    }
}

fn format_rigid_ty(resolver: Option<&LocalResolver>, ty: RigidTy) -> String {
    match ty {
        RigidTy::Bool => "bool".to_owned(),
        RigidTy::Char => "char".to_owned(),
        RigidTy::Int(int_ty) => format_int_ty(int_ty).to_owned(),
        RigidTy::Uint(uint_ty) => format_uint_ty(uint_ty).to_owned(),
        RigidTy::Float(float_ty) => format_float_ty(float_ty).to_owned(),
        RigidTy::Adt(adt, args) => {
            let base = if let Some(resolver) = resolver
                && let Some(info) = resolver.c_adt_display_info(adt.0)
            {
                let kind = if info.is_enum {
                    "enum"
                } else {
                    match info.kind {
                        StructOrUnionKind::Struct => "struct",
                        StructOrUnionKind::Union => "union",
                    }
                };
                let name = info
                    .tag_name
                    .unwrap_or_else(|| format!("#{}", info.anonymous_id));
                format!("co2({kind} {name})")
            } else {
                adt.trimmed_name().clone()
            };
            format_with_generic_args(resolver, base, &args.0)
        }
        RigidTy::Foreign(def) => def.trimmed_name().clone(),
        RigidTy::Str => "str".to_owned(),
        RigidTy::Array(inner, len) => {
            let len = len
                .eval_target_usize()
                .map_or_else(|_| format!("{len:?}"), |len| len.to_string());
            format!("[{}; {len}]", format_ty(resolver, inner))
        }
        RigidTy::Pat(inner, _) => format_ty(resolver, inner),
        RigidTy::Slice(inner) => format!("[{}]", format_ty(resolver, inner)),
        RigidTy::RawPtr(inner, mutability) => {
            format!(
                "*{} {}",
                format_mutability(mutability),
                format_ty(resolver, inner)
            )
        }
        RigidTy::Ref(_, inner, Mutability::Not) => format!("&{}", format_ty(resolver, inner)),
        RigidTy::Ref(_, inner, Mutability::Mut) => format!("&mut {}", format_ty(resolver, inner)),
        RigidTy::FnDef(def, args) => {
            format_with_generic_args(resolver, def.trimmed_name().clone(), &args.0)
        }
        RigidTy::FnPtr(sig) => {
            let sig = sig.value;
            let params = sig
                .inputs()
                .iter()
                .map(|ty| format_ty(resolver, *ty))
                .chain(sig.c_variadic.then(|| "...".to_owned()))
                .collect::<Vec<_>>()
                .join(", ");
            format!("fn({params}) -> {}", format_ty(resolver, sig.output()))
        }
        RigidTy::Never => "!".to_owned(),
        RigidTy::Tuple(items) => {
            if items.is_empty() {
                return "()".to_owned();
            }
            let mut items = items
                .into_iter()
                .map(|ty| format_ty(resolver, ty))
                .collect::<Vec<_>>();
            if items.len() == 1 {
                items[0].push(',');
            }
            format!("({})", items.join(", "))
        }
        RigidTy::Closure(def, args) => {
            format_with_generic_args(resolver, def.trimmed_name().clone(), &args.0)
        }
        RigidTy::Coroutine(def, args) => {
            format_with_generic_args(resolver, def.trimmed_name().clone(), &args.0)
        }
        RigidTy::CoroutineClosure(def, args) => {
            format_with_generic_args(resolver, def.trimmed_name().clone(), &args.0)
        }
        RigidTy::CoroutineWitness(def, args) => {
            format_with_generic_args(resolver, def.trimmed_name().clone(), &args.0)
        }
        RigidTy::Dynamic(_, _) => "dyn Trait".to_owned(),
    }
}

fn format_with_generic_args(
    resolver: Option<&LocalResolver>,
    base: String,
    args: &[GenericArgKind],
) -> String {
    let args = args
        .iter()
        .filter_map(|arg| match arg {
            GenericArgKind::Type(ty) => Some(format_ty(resolver, *ty)),
            GenericArgKind::Const(c) => Some(
                c.eval_target_usize()
                    .map_or_else(|_| format!("{c:?}"), |value| value.to_string()),
            ),
            GenericArgKind::Lifetime(_) => None,
        })
        .collect::<Vec<_>>();
    if args.is_empty() {
        base
    } else {
        format!("{base}<{}>", args.join(", "))
    }
}

fn format_mutability(mutability: Mutability) -> &'static str {
    match mutability {
        Mutability::Not => "const",
        Mutability::Mut => "mut",
    }
}

fn format_int_ty(ty: IntTy) -> &'static str {
    match ty {
        IntTy::Isize => "isize",
        IntTy::I8 => "i8",
        IntTy::I16 => "i16",
        IntTy::I32 => "i32",
        IntTy::I64 => "i64",
        IntTy::I128 => "i128",
    }
}

fn format_uint_ty(ty: UintTy) -> &'static str {
    match ty {
        UintTy::Usize => "usize",
        UintTy::U8 => "u8",
        UintTy::U16 => "u16",
        UintTy::U32 => "u32",
        UintTy::U64 => "u64",
        UintTy::U128 => "u128",
    }
}

fn format_float_ty(ty: FloatTy) -> &'static str {
    match ty {
        FloatTy::F16 => "f16",
        FloatTy::F32 => "f32",
        FloatTy::F64 => "f64",
        FloatTy::F128 => "f128",
    }
}

pub(crate) fn enum_payload_ty(ty: Ty) -> Option<Ty> {
    if let TyKind::RigidTy(RigidTy::Pat(inner, _)) = ty.kind() {
        return enum_payload_ty(inner);
    }
    let TyKind::RigidTy(RigidTy::Adt(adt, args)) = ty.kind() else {
        return None;
    };
    let variant = adt.variant(variant_idx(0))?;
    let fields = variant.fields();
    if fields.len() != 1 || fields[0].name.clone() != ENUM_FIELD_NAME {
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
        TyKind::RigidTy(RigidTy::Bool | RigidTy::Int(_) | RigidTy::Uint(_) | RigidTy::Float(_))
    )
}

pub(crate) fn is_integer_ty(ty: Ty) -> bool {
    if enum_payload_ty(ty).is_some() {
        return true;
    }
    matches!(
        ty.kind(),
        TyKind::RigidTy(RigidTy::Bool | RigidTy::Int(_) | RigidTy::Uint(_))
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

    let (rank, unsigned) = match lhs_rank.cmp(&rhs_rank) {
        std::cmp::Ordering::Equal => (lhs_rank, lhs_unsigned || rhs_unsigned),
        std::cmp::Ordering::Greater => (lhs_rank, lhs_unsigned),
        std::cmp::Ordering::Less => (rhs_rank, rhs_unsigned),
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
        TyKind::RigidTy(
            RigidTy::Bool
                | RigidTy::Int(_)
                | RigidTy::Uint(_)
                | RigidTy::RawPtr(_, _)
                | RigidTy::FnPtr(_)
                | RigidTy::FnDef(_, _)
        )
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
            TyKind::RigidTy(RigidTy::RawPtr(_, _) | RigidTy::FnPtr(_)),
            TyKind::RigidTy(RigidTy::Int(_) | RigidTy::Uint(_))
        ) | (
            TyKind::RigidTy(RigidTy::FnPtr(_)),
            TyKind::RigidTy(RigidTy::FnDef(_, _))
        ) | (
            TyKind::RigidTy(RigidTy::Int(_) | RigidTy::Uint(_)),
            TyKind::RigidTy(RigidTy::RawPtr(_, _) | RigidTy::FnPtr(_) | RigidTy::FnDef(_, _))
        )
    ) || fn_pointer_void_pointer_cast_allowed(dst, src)
        || pointer_implicit_cast_allowed(dst, src)
        || (dst_is_mu_fn_ptr
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

fn fn_pointer_void_pointer_cast_allowed(dst: Ty, src: Ty) -> bool {
    match (dst.kind(), src.kind()) {
        (
            TyKind::RigidTy(RigidTy::RawPtr(dst_pointee, _)),
            TyKind::RigidTy(RigidTy::FnPtr(_) | RigidTy::FnDef(_, _)),
        ) => is_void_ty(dst_pointee),
        (TyKind::RigidTy(RigidTy::FnPtr(_)), TyKind::RigidTy(RigidTy::RawPtr(src_pointee, _))) => {
            is_void_ty(src_pointee)
        }
        _ => false,
    }
}

fn pointer_implicit_cast_allowed(dst: Ty, src: Ty) -> bool {
    let Some(dst_pointee) = pointer_pointee_ty(dst) else {
        return false;
    };
    let Some(src_pointee) = pointer_pointee_ty(src) else {
        return false;
    };
    is_void_ty(dst_pointee)
        || is_void_ty(src_pointee)
        || pointer_pointees_compatible(dst_pointee, src_pointee)
}

fn pointer_pointee_ty(ty: Ty) -> Option<Ty> {
    match ty.kind() {
        TyKind::RigidTy(RigidTy::RawPtr(inner, _) | RigidTy::Ref(_, inner, _)) => Some(inner),
        _ => None,
    }
}

fn is_void_ty(ty: Ty) -> bool {
    matches!(ty.kind(), TyKind::RigidTy(RigidTy::Tuple(items)) if items.is_empty())
}

fn pointer_pointees_compatible(expected: Ty, actual: Ty) -> bool {
    let expected = strip_pat_ty(expected);
    let actual = strip_pat_ty(actual);

    if ty_matches_expected(expected, actual) {
        return true;
    }

    let expected_inner = enum_payload_ty(expected).unwrap_or(expected);
    let actual_inner = enum_payload_ty(actual).unwrap_or(actual);
    let expected_is_enum = expected_inner != expected;
    let actual_is_enum = actual_inner != actual;

    ((expected_inner != expected || actual_inner != actual)
        && ty_matches_expected(expected_inner, actual_inner))
        || ((expected_is_enum || actual_is_enum)
            && integer_bit_width(expected_inner)
                .zip(integer_bit_width(actual_inner))
                .is_some_and(|(expected_bits, actual_bits)| expected_bits == actual_bits))
        || (is_byte_pointer_pointee(expected_inner) && is_byte_pointer_pointee(actual_inner))
}

fn is_byte_pointer_pointee(ty: Ty) -> bool {
    matches!(
        strip_pat_ty(ty).kind(),
        TyKind::RigidTy(RigidTy::Int(IntTy::I8) | RigidTy::Uint(UintTy::U8))
    )
}

fn strip_pat_ty(ty: Ty) -> Ty {
    match ty.kind() {
        TyKind::RigidTy(RigidTy::Pat(inner, _)) => strip_pat_ty(inner),
        _ => ty,
    }
}

fn integer_bit_width(ty: Ty) -> Option<u8> {
    match strip_pat_ty(ty).kind() {
        TyKind::RigidTy(RigidTy::Int(int_ty)) => Some(match int_ty {
            IntTy::I8 => 8,
            IntTy::I16 => 16,
            IntTy::I32 => 32,
            IntTy::I64 => 64,
            IntTy::I128 => 128,
            IntTy::Isize => usize::BITS as u8,
        }),
        TyKind::RigidTy(RigidTy::Uint(uint_ty)) => Some(match uint_ty {
            UintTy::U8 => 8,
            UintTy::U16 => 16,
            UintTy::U32 => 32,
            UintTy::U64 => 64,
            UintTy::U128 => 128,
            UintTy::Usize => usize::BITS as u8,
        }),
        _ => None,
    }
}

pub(crate) fn resolve_field_path_in_adt(base: Ty, field: &str) -> Option<(Vec<usize>, Ty)> {
    let TyKind::RigidTy(RigidTy::Adt(adt, args)) = base.kind() else {
        return None;
    };
    let variant = adt.variant(variant_idx(0))?;
    let fields = variant.fields();
    for (idx, field_def) in fields.iter().enumerate() {
        let name = field_def.name.clone();
        if name == field {
            return Some((vec![idx], field_def.ty_with_args(&args)));
        }
    }
    for (idx, field_def) in fields.iter().enumerate() {
        let name = field_def.name.clone();
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
