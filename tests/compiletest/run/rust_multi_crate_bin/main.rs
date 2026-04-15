use support_lib::{Point, PointPtr, add, hypot, Union1, S7Type, S7, HUGE_LONG, ComplexStruct};

const _: () = {
    use std::ptr::null;
    use core::ffi;
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

    if false {
        unsafe {
            let _: *const ffi::c_int = &raw const (*dummy).int_field;
            let _: *const *mut ffi::c_char = &raw const (*dummy).char_star;
            let _: *const *const ffi::c_char = &raw const (*dummy).const_char_star;
            let _: *const *const *const ffi::c_char = &raw const (*dummy).const_const_char_star;
        }
    }
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
}
