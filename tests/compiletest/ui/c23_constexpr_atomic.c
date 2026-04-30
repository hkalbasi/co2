//@ mode: c
//@ compile-fail

    constexpr _Atomic int x = 1;
//  ^^^^^^^^^ error: `constexpr` object type cannot be atomic

int main(void) {
    return x;
}
