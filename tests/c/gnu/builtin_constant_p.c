//@ mode: c
//@ run-status: 0

int constant_p_array[__builtin_constant_p(40 + 2) ? 3 : -1] = {};

int check_constant_p(int param) {
    int local = 0;

    if (!__builtin_constant_p(1)) return 1;
    if (!__builtin_constant_p(1 + 2 * 3)) return 2;
    if (!__builtin_constant_p(sizeof(int))) return 3;
    if (__builtin_constant_p(param)) return 4;
    if (__builtin_constant_p(local)) return 5;

    if (__builtin_constant_p(local++)) return 6;
    if (local != 0) return 7;

    return 0;
}

int main(void) {
    if (sizeof(constant_p_array) != sizeof(int) * 3) return 10;
    return check_constant_p(42);
}
