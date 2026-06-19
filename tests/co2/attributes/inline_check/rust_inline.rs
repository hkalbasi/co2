#[no_mangle]
#[inline(never)]
fn never_short() -> i32 { 42 }

#[no_mangle]
#[inline(always)]
fn always_short() -> i32 { 43 }

#[no_mangle]
#[inline]
fn hint_short() -> i32 { 44 }

#[no_mangle]
fn no_attr_short() -> i32 { 45 }

fn main() {
    if never_short() != 42 { std::process::exit(1); }
    if always_short() != 43 { std::process::exit(2); }
    if hint_short() != 44 { std::process::exit(3); }
    if no_attr_short() != 45 { std::process::exit(4); }
}
