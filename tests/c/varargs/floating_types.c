//@ mode: c
//@ run-status: 0

#include <stdarg.h>

int sum_doubles(int n, ...) {
    va_list ap;
    va_start(ap, n);
    double total = 0.0;
    for (int i = 0; i < n; i++) {
        total += va_arg(ap, double);
    }
    va_end(ap);
    return (int)total;
}

int check_long_doubles(int n, ...) {
    va_list ap;
    va_start(ap, n);
    int ok = 1;
    ok &= va_arg(ap, long double) == 1.25L;
    ok &= va_arg(ap, int) == 7;
    ok &= va_arg(ap, long double) == -3.5L;
    ok &= va_arg(ap, double) == 2.0;
    va_end(ap);
    return ok;
}

int check_float128(int n, ...) {
    va_list ap;
    va_start(ap, n);
    int ok = 1;
    ok &= va_arg(ap, _Float128) == 4.5f128;
    ok &= va_arg(ap, int) == 8;
    ok &= va_arg(ap, _Float128) == -0.125f128;
    va_end(ap);
    return ok;
}

int sum_via_helper(va_list ap, int n) {
    double total = 0.0;
    for (int i = 0; i < n; i++) {
        total += va_arg(ap, double);
    }
    return (int)total;
}

int forward_doubles(int n, ...) {
    va_list ap;
    va_start(ap, n);
    int result = sum_via_helper(ap, n);
    va_end(ap);
    return result;
}

int many_mixed(int n, ...) {
    va_list ap;
    va_start(ap, n);
    int ok = 1;
    ok &= va_arg(ap, double) == 1.0;
    ok &= va_arg(ap, long double) == 2.0L;
    ok &= va_arg(ap, _Float128) == 3.0f128;
    ok &= va_arg(ap, double) == 4.0;
    ok &= va_arg(ap, long double) == 5.0L;
    ok &= va_arg(ap, _Float128) == 6.0f128;
    ok &= va_arg(ap, double) == 7.0;
    ok &= va_arg(ap, long double) == 8.0L;
    ok &= va_arg(ap, _Float128) == 9.0f128;
    ok &= va_arg(ap, double) == 10.0;
    ok &= va_arg(ap, long double) == 11.0L;
    ok &= va_arg(ap, _Float128) == 12.0f128;
    ok &= va_arg(ap, double) == 13.0;
    ok &= va_arg(ap, long double) == 14.0L;
    ok &= va_arg(ap, _Float128) == 15.0f128;
    va_end(ap);
    return ok;
}

int main(void) {
    if (sum_doubles(3, 1.5, 2.25, -0.75) != 3) {
        return 1;
    }
    if (sum_doubles(10, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0) != 10) {
        return 2;
    }
    if (!check_long_doubles(4, 1.25L, 7, -3.5L, 2.0)) {
        return 3;
    }
    if (!check_float128(3, 4.5f128, 8, -0.125f128)) {
        return 4;
    }
    if (forward_doubles(4, 1.0, 2.0, 3.0, 4.0) != 10) {
        return 5;
    }
    if (!many_mixed(15,
                    1.0, 2.0L, 3.0f128, 4.0,
                    5.0L, 6.0f128, 7.0, 8.0L,
                    9.0f128, 10.0, 11.0L, 12.0f128,
                    13.0, 14.0L, 15.0f128)) {
        return 6;
    }
    return 0;
}
