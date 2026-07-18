//@ mode: c
//@ compile-fail

int g[3] = {1, 2, 3, 4};
                  // ^ warning: excess elements in array initializer

double one_half_fn() {
    return 1.5;
}

static const double one_half = one_half_fn();
                            // ^^^^^^^^^^^^^ error: cannot call non-const function

int h[3] = {[10] = 4};
         // ^^^^ warning: initializer designator index 10 exceeds array bounds

int main() {
    return 0;
}
