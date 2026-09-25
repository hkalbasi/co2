//@ mode: c
//@ run-status: 0
// Test for computed goto through a function-pointer-typed variable.
// Label addresses (`&&label`) are small integer discriminants, so the
// indirect-goto lowering must cast pointer-like operands to `usize`
// before switching on them. C function pointers allow null and use a
// different MIR encoding than raw pointers, which used to be missed by
// the cast (causing a rustc ICE) while plain `void *` worked.

#include <assert.h>

static int jump_once(void) {
    int x = 0;
    void (*fp)(void) = (void (*)(void))&&target;
    goto *fp;
    x = 1;
    goto done;
target:
    x = 42;
done:
    return x;
}

static int dispatch(int opcode) {
    void (*to_zero)(void) = (void (*)(void))&&case_zero;
    void (*to_one)(void) = (void (*)(void))&&case_one;
    assert(to_zero != to_one);
    assert(to_zero == to_zero);
    void (*fp)(void) = opcode == 0 ? to_zero : to_one;
    goto *fp;
case_zero:
    return 10;
case_one:
    return 11;
}

int main(void) {
    assert(jump_once() == 42);
    assert(dispatch(0) == 10);
    assert(dispatch(1) == 11);
    return 0;
}
