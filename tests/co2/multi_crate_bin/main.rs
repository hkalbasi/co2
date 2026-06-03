#![feature(const_cmp)]
#![feature(const_trait_impl)]

use support_lib::{
    add, hypot,
    inner_mod::{inner_mod_fn, RustStructInModule, TypedefedStructInModule},
    BitFieldReprCStruct, ComplexStruct, CopyReprRustStruct, ExternType1, ExternType1Again,
    ExternType2, NormalReprCStruct, Point, PointPtr, ReprRustStruct, RustStructHoldingVector,
    S7Type, Union1, HUGE_LONG, S7,
};

use std::mem::offset_of;

macro_rules! assert_impl {
    ($ty:ty : $($trait:path)+) => {{
        const fn check<T: $($trait +)+>() {}
        check::<$ty>();
    }};
}

macro_rules! assert_not_impl {
    ($x:ty: $($t:path),+ $(,)?) => {
        const _: fn() = || {
            // Generic trait with a blanket impl over `()` for all types.
            trait AmbiguousIfImpl<A> {
                // Required for actually being able to reference the trait.
                fn some_item() {}
            }

            impl<T: ?Sized> AmbiguousIfImpl<()> for T {}

            // Creates multiple scoped `Invalid` types for each trait `$t`, over
            // which a specialized `AmbiguousIfImpl<Invalid>` is implemented for
            // every type that implements `$t`.
            $({
                #[allow(dead_code)]
                struct Invalid;

                impl<T: ?Sized + $t> AmbiguousIfImpl<Invalid> for T {}
            })+

            // If there is only one specialized trait impl, type inference with
            // `_` can be resolved and this can compile. Fails to compile if
            // `$x` implements any `AmbiguousIfImpl<Invalid>`.
            let _ = <$x as AmbiguousIfImpl<_>>::some_item;
        };
    };
}

const _: () = {
    use core::ffi;
    use std::ptr::null;
    let _: *const support_lib::Co2Int = null::<ffi::c_int>();
    let _: *const support_lib::Co2Long = null::<ffi::c_long>();
    let _: *const support_lib::Co2LongLong = null::<ffi::c_longlong>();
    let _: *const support_lib::Co2UInt = null::<ffi::c_uint>();
    let _: *const support_lib::Co2ULong = null::<ffi::c_ulong>();
    let _: *const support_lib::Co2ULongLong = null::<ffi::c_ulonglong>();
    let _: *const support_lib::Co2Float = null::<ffi::c_float>();
    let _: *const support_lib::Co2Double = null::<ffi::c_double>();
    let _: *const support_lib::Co2Char = null::<ffi::c_char>();
    let _: *const support_lib::Co2UChar = null::<ffi::c_uchar>();
    let _: *const support_lib::Co2Bool = null::<bool>();

    let _: *const support_lib::Union1MutPtr = null::<*mut Union1>();
    let _: *const support_lib::Union1ConstPtr = null::<*const Union1>();
    let _: *const support_lib::ConstPtrToMutPtrToUnion1 = null::<*const *mut Union1>();
    let _: *const support_lib::MutPtrToConstPtrToUnion1 = null::<*mut *const Union1>();

    let dummy: *const ComplexStruct = null();
    let dummy2: *const ReprRustStruct = null();
    let dummy3: *const RustStructHoldingVector = null();
    let dummy4: *const CopyReprRustStruct = null();

    if false {
        unsafe {
            let _: *const ffi::c_int = &raw const (*dummy).int_field;
            let _: *const *mut ffi::c_char = &raw const (*dummy).char_star;
            let _: *const *const ffi::c_char = &raw const (*dummy).const_char_star;
            let _: *const *const *const ffi::c_char = &raw const (*dummy).const_const_char_star;

            let _: *const i32 = &raw const (*dummy2).a;
            let _: *const *mut i32 = &raw const (*dummy2).b;
            let _: *const *const i32 = &raw const (*dummy2).c;
            let _: *const NormalReprCStruct = &raw const (*dummy2).d;
            let _: *const BitFieldReprCStruct = &raw const (*dummy2).e;
            let _: *const *mut BitFieldReprCStruct = &raw const (*dummy2).f;
        
            let _: *const Vec<i32> = &raw const (*dummy3).b;
            let _: *const Vec<RustStructHoldingVector> = &raw const (*dummy3).c;
        
            let _: *const Point = &raw const (*dummy4).b;
            let _: *const *mut CopyReprRustStruct = &raw const (*dummy4).c;
            let _: *const *const RustStructHoldingVector = &raw const (*dummy4).foo;
        }
    }

    let _: *const [i32; support_lib::C23Const] = null::<[i32; 2]>();

    assert!(size_of::<*mut ExternType1>() == size_of::<*mut ()>());
    assert!(size_of::<*mut ExternType1Again>() == size_of::<*mut ()>());
    assert!(size_of::<*mut ExternType2>() == size_of::<*mut ()>());

    let _: *const ExternType1 = null::<ExternType1Again>();

    use std::any::TypeId;

    assert!(TypeId::of::<*mut ExternType1>() == TypeId::of::<*mut ExternType1Again>());
    assert!(TypeId::of::<*mut ExternType1>() != TypeId::of::<*mut ExternType2>());

    assert_impl!(Union1: Copy);
    assert_impl!(BitFieldReprCStruct: Copy);
    assert_impl!(Point: Copy);
    assert_impl!(ComplexStruct: Copy);
    assert_impl!(CopyReprRustStruct: Copy);
    assert_not_impl!(ReprRustStruct: Copy);
    assert_not_impl!(RustStructHoldingVector: Copy);
};

fn main() {
    let mut p = Point { x: 4, y: -1 };
    assert_eq!(p.x, 4);
    assert_eq!(p.y, -1);

    let p_ptr: PointPtr = &raw mut p;
    assert_ne!(p_ptr, std::ptr::null_mut());
    assert_eq!(add(4, -1), 3);
    assert_eq!(hypot(p), 17);

    let u = Union1 { b: 12 };
    unsafe {
        assert_eq!(u.a, 12);
    }

    let s7_copy: S7Type = unsafe { S7 };
    assert_eq!(s7_copy.x[2], b'n' as i8);
    assert_eq!(unsafe { HUGE_LONG }, 0xabcd00000000);

    assert_eq!(inner_mod_fn(), 5);

    assert_eq!(offset_of!(NormalReprCStruct, a), 0);
    assert_eq!(offset_of!(NormalReprCStruct, b), 4);
    assert_eq!(offset_of!(NormalReprCStruct, c), 5);
    assert_eq!(offset_of!(NormalReprCStruct, d), 8);

    assert_eq!(offset_of!(BitFieldReprCStruct, a), 0);
    assert_eq!(offset_of!(BitFieldReprCStruct, e), 8);

    assert_eq!(std::any::type_name::<Point>(), "support_lib::Point");
    assert_eq!(std::any::type_name::<Union1>(), "support_lib::Union1");
    assert!(
        std::any::type_name::<S7Type>().contains("__co2")
            && std::any::type_name::<S7Type>().contains("S7")
    );
    assert_eq!(
        std::any::type_name::<NormalReprCStruct>(),
        "support_lib::NormalReprCStruct"
    );
    assert_eq!(
        std::any::type_name::<BitFieldReprCStruct>(),
        "support_lib::BitFieldReprCStruct"
    );
    assert_eq!(
        std::any::type_name::<support_lib::TaggedAlias1>(),
        std::any::type_name::<support_lib::TaggedAlias2>()
    );
    assert_eq!(
        std::any::type_name::<support_lib::TaggedAlias1>(),
        std::any::type_name::<support_lib::TaggedAlias3>()
    );
    assert_eq!(
        std::any::type_name::<RustStructInModule>(),
        "support_lib::inner_mod::RustStructInModule"
    );
    assert_eq!(
        std::any::type_name::<RustStructInModule>(),
        "support_lib::inner_mod::RustStructInModule"
    );
}
