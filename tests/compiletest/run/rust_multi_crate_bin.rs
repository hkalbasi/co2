//@ mode: rust
//@ aux-lib: support_lib rust_multi_crate_lib.aux.co2
//@ run-status: 0

use support_lib::{Point, PointPtr, add, hypot};

fn main() {
    let mut p = Point { x: 4, y: -1 };
    assert_eq!(p.x, 4);
    assert_eq!(p.y, -1);
    let p_ptr: PointPtr = &raw mut p;
    assert_ne!(p_ptr, std::ptr::null_mut());
    assert_eq!(add(4, -1), 3);
    assert_eq!(hypot(p), 17);
}
