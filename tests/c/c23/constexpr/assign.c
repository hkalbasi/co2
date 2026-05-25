//@ mode: c
//@ compile-fail

int f(void) {
    constexpr int x = 1;
    x = 2;
//  ^ error: assignment of read-only constexpr variable `x`
    return x;
}
