//@ mode: c
//@ compile-fail

    extern constexpr int x = 1;
//         ^^^^^^^^^ error: `constexpr` cannot be combined with `extern`

int main(void) {
    return x;
}
