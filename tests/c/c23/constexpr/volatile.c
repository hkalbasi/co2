//@ mode: c
//@ compile-fail

constexpr volatile int x = 1;
      //  ^^^^^^^^ error: `constexpr` object type cannot be volatile-qualified

int main(void) {
    return x;
}
