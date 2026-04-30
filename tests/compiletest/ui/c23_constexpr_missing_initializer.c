//@ mode: c
//@ compile-fail

int f(void) {
    constexpr int x;
//  ^^^^^^^^^ error: `constexpr` requires an initializer
    return 0;
}
