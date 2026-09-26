//@ mode: c
//@ compile-fail

#include <complex.h>
// Integer-only operators (`%`, `&`, `|`, `^`, `<<`, `>>`)
// rejected on floating-point and complex operands, matching GCC
// (`error: invalid operands to binary ...`).

void mod_float(void) {
    double x = 5.5, y = 2.0;
    double z = x % y;
    //         ^^^^^ error: can not use `%` on type f64 and f64
    (void)z;
}

void mod_mixed(void) {
    int x = 5; double y = 2.0;
    double z = x % y;
    //         ^^^^^ error: can not use `%` on type i32 and f64
    (void)z;
}

void bitand_float(void) {
    double x = 5.5, y = 2.0;
    double z = x & y;
    //         ^^^^^ error: can not use `&` on type f64 and f64
    (void)z;
}

void bitor_mixed(void) {
    float x = 1.0f; int y = 2;
    float z = x | y;
    //        ^^^^^ error: can not use `|` on type f32 and i32
    (void)z;
}

void bitxor_float(void) {
    double x = 1.0, y = 2.0;
    double z = x ^ y;
    //         ^^^^^ error: can not use `^` on type f64 and f64
    (void)z;
}

void shl_float(void) {
    double x = 1.0, y = 2.0;
    double z = x << y;
    //         ^^^^^^ error: can not use `<<` on type f64 and f64
    (void)z;
}

void shl_mixed(void) {
    double x = 1.0; int y = 2;
    double z = x << y;
    //         ^^^^^^ error: can not use `<<` on type f64 and i32
    (void)z;
}

void shr_float(void) {
    float x = 1.0f, y = 2.0f;
    float z = x >> y;
    //        ^^^^^^ error: can not use `>>` on type f32 and f32
    (void)z;
}

void shr_mixed(void) {
    int x = 8; float y = 2.0f;
    float z = x >> y;
    //        ^^^^^^ error: can not use `>>` on type i32 and f32
    (void)z;
}

void mod_assign_float(void) {
    double x = 5.5;
    x %= 2.0;
  //^^^^^^^^ error: can not use `%` on type f64 and f64
    (void)x;
}

// Complex operands use the same integer-only rules.

void mod_int_complex(void) {
    int _Complex a = 1 + 2*I, b = 3 + 4*I;
    int _Complex r = a % b;
    //               ^^^^^ error: can not use `%` on complex types Complex<i32> and Complex<i32>
    (void)r;
}

void mod_mixed_complex(void) {
    double _Complex a = 1.0 + 2.0*I; double b = 3.0;
    double _Complex r = a % b;
    //                  ^^^^^ error: can not use `%` on complex types Complex<f64> and f64
    (void)r;
}

void bitand_complex(void) {
    double _Complex a = 1.0 + 2.0*I, b = 3.0 + 4.0*I;
    double _Complex r = a & b;
    //                  ^^^^^ error: can not use `&` on complex types Complex<f64> and Complex<f64>
    (void)r;
}

void bitor_mixed_complex(void) {
    float _Complex a = 1.0f + 2.0f*I; float b = 3.0f;
    float _Complex r = a | b;
    //                 ^^^^^ error: can not use `|` on complex types Complex<f32> and f32
    (void)r;
}

void bitxor_complex(void) {
    double _Complex a = 1.0 + 2.0*I, b = 3.0 + 4.0*I;
    double _Complex r = a ^ b;
    //                  ^^^^^ error: can not use `^` on complex types Complex<f64> and Complex<f64>
    (void)r;
}

void shl_complex(void) {
    double _Complex a = 1.0 + 2.0*I, b = 3.0 + 4.0*I;
    double _Complex r = a << b;
    //                  ^^^^^^ error: can not use `<<` on complex types Complex<f64> and Complex<f64>
    (void)r;
}

void shr_int_complex(void) {
    int _Complex a = 1 + 2*I, b = 3 + 4*I;
    int _Complex r = a >> b;
    //               ^^^^^^ error: can not use `>>` on complex types Complex<i32> and Complex<i32>
    (void)r;
}

void shr_mixed_complex(void) {
    double _Complex a = 1.0 + 2.0*I; int b = 2;
    double _Complex r = a >> b;
    //                  ^^^^^^ error: can not use `>>` on complex types Complex<f64> and i32
    (void)r;
}

void mod_assign_complex(void) {
    double _Complex x = 1.0 + 2.0*I;
    x %= 2.0;
  //^^^^^^^^ error: can not use `%` on complex types Complex<f64> and f64
    (void)x;
}

int main() {
    return 0;
}