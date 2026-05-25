//@ mode: c
//@ compile-fail

constexpr int *restrict ptr = 0;
//             ^^^^^^^^ error: `constexpr` object type cannot be restrict-qualified

int main(void) {
    return ptr != 0;
}
