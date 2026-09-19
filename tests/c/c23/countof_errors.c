//@ mode: c
//@ compile-fail
//@ requires-header: stdcountof.h

#include <stdcountof.h>

int non_array = 0;

int bad_expr(void) {
    return _Countof(non_array);
    //     ^^^^^^^^^^^^^^^^^^^ error: '_Countof' requires an argument of array type
}

int bad_type(void) {
    return _Countof(int);
    //     ^^^^^^^^^^^^^ error: '_Countof' requires an argument of array type
}

int bad_macro(void) {
    return countof(1);
    //     ^^^^^^^^^^ error: '_Countof' requires an argument of array type
}

int *ptr = &non_array;

int bad_ptr(void) {
    return _Countof(ptr);
    //     ^^^^^^^^^^^^^ error: '_Countof' requires an argument of array type
}
