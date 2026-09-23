//@ mode: c
//@ compile-fail
// Failing cases for complex numbers (_Complex): rejected operators,
// invalid operands, bad casts, rejected implicit conversions, and
// non-scalar uses.

#include <complex.h>

struct S { int x; };

void take_double(double x) {
    (void)x;
}

// `/` on GNU integer complex is rejected, matching GCC.
void div_int_complex(void) {
    int _Complex a = 1 + 2 * I;
    int _Complex b = 3 + 4 * I;
    int _Complex q = a / b;
    //               ^^^^^ error: can not use `/` on integer complex types Complex<i32> and Complex<i32>
    (void)q;
}

// `%` is not defined on complex operands.
void mod_complex(void) {
    double _Complex a = 1.0 + 2.0 * I;
    double _Complex b = 3.0 + 4.0 * I;
    double _Complex r = a % b;
    //                  ^^^^^ error: can not use `%` on complex types Complex<f64> and Complex<f64>
    (void)r;
}

// A struct operand is neither complex nor real.
void add_struct_to_complex(void) {
    double _Complex z = 1.0 + 2.0 * I;
    struct S s = { 1 };
    double _Complex r = z + s;
    //                  ^^^^^ error: invalid operands to complex operator: Complex<f64> and co2(struct S)
    (void)r;
    (void)s;
}

// Explicit cast from a non-arithmetic type to complex.
void cast_struct_to_complex(void) {
    struct S s = { 1 };
    double _Complex z = (double _Complex)s;
    //                                   ^ error: cannot convert co2(struct S) to complex type
    (void)z;
    (void)s;
}

// Explicit cast from complex to a pointer type.
void cast_complex_to_ptr(void) {
    double _Complex z = 1.0 + 2.0 * I;
    int *p = (int *)z;
    //       ^^^^^^^^ error: unsupported cast from Complex<f64> to *mut i32
    (void)p;
}

// Implicit conversion from complex to real is rejected.
void init_real_from_complex(void) {
    double _Complex z = 1.0 + 2.0 * I;
    double x = z;
    //         ^ error: initializer type mismatch: expected f64, got Complex<f64>
    (void)x;
}

void assign_real_from_complex(void) {
    double _Complex z = 1.0 + 2.0 * I;
    double x = 0.0;
    x = z;
  //^^^^^ error: assignment type mismatch: expected f64, got Complex<f64>
}

double return_real_from_complex(void) {
    double _Complex z = 1.0 + 2.0 * I;
    return z;
    //     ^ error: return type mismatch: expected f64, got Complex<f64>
}

void call_real_from_complex(void) {
    double _Complex z = 1.0 + 2.0 * I;
    take_double(z);
    //          ^ error: call `take_double` type mismatch at arg 0: expected f64, got Complex<f64>
}

// Complex values are not valid conditions.
void cond_complex(void) {
    double _Complex z = 1.0 + 2.0 * I;
    if (z) {
    //  ^ error: condition must be scalar-like, got Complex<f64>
    }
}

// Complex values are not valid switch discriminants.
void switch_complex(void) {
    double _Complex z = 1.0 + 2.0 * I;
    switch (z) {
    //      ^ error: switch expression must be integer-like, got Complex<f64>
    case 1:
        break;
    }
}
