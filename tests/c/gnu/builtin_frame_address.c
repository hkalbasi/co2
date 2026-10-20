//@ mode: c
//@ run-status: 0

// Regression test: __builtin_frame_address is used by quickjs.c. It is not
// supported, so it warns and evaluates to NULL, and the cast target must still
// parse when the builtin follows it.

#include <stdint.h>

static uintptr_t stack_pointer(void) {
    return (uintptr_t)__builtin_frame_address(0);
                    //^^^^^^^^^^^^^^^^^^^^^^^^^^ warning: `__builtin_frame_address` is not supported, returning NULL
}

int main(void) {
    return stack_pointer() != 0;
}
