//@ mode: c
//@ compile-fail

int f(void) {
    constexpr int x = 1;
    x++;
//  ^ error: update of read-only constexpr variable `x`
    return x;
}
