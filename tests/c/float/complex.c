//@ mode: c
//@ run-status: 0
//@ compile-flags: -lm
// Test for complex numbers (_Complex, complex, I, _Complex_I, imaginary).

#include <stddef.h>
#include <assert.h>
#include <complex.h>
#include <math.h>

// ------------------------------------------------------------------
// Compile-time type check
// ------------------------------------------------------------------
#define TYPE_IS(expr, type) _Generic((expr), type: 1, default: 0)

// ------------------------------------------------------------------
// 1. Basic type declarations
// ------------------------------------------------------------------

void test_types(void) {
    float _Complex       fc = 1.0f + 2.0f * I;
    double _Complex      dc = 3.0  + 4.0  * I;
    long double _Complex lc = 5.0L + 6.0L * I;

    // The `complex` macro from <complex.h> is equivalent to `_Complex`
    float complex        fc2 = 1.0f + 2.0f * I;
    double __complex__       dc2 = 3.0  + 4.0  * I;
    long double complex  lc2 = 5.0L + 6.0L * I;

    (void)fc; (void)dc; (void)lc;
    (void)fc2; (void)dc2; (void)lc2;

    assert(TYPE_IS(fc,  float _Complex));
    assert(TYPE_IS(dc,  double __complex__));
    assert(TYPE_IS(lc,  long double _Complex));

    assert(TYPE_IS(fc2, float _Complex));
    assert(TYPE_IS(dc2, double _Complex));
    assert(TYPE_IS(lc2, long double _Complex));

    // Sizes: complex has twice the size of its real counterpart
    _Static_assert(sizeof(float _Complex)       == 2 * sizeof(float),       "float complex size");
    _Static_assert(sizeof(double _Complex)      == 2 * sizeof(double),      "double complex size");
    _Static_assert(sizeof(long double _Complex) == 2 * sizeof(long double), "long double complex size");
}

// ------------------------------------------------------------------
// 2. Real and imaginary parts via creal / cimag
// ------------------------------------------------------------------

void test_parts(void) {
    double _Complex z = 3.5 - 2.25 * I;

    assert(creal(z) == 3.5);
    assert(cimag(z) == -2.25);

    float _Complex zf = 1.0f + 1.5f * I;
    assert(crealf(zf) == 1.0f);
    assert(cimagf(zf) == 1.5f);

    long double _Complex zl = 2.0L - 0.5L * I;
    assert(creall(zl) == 2.0L);
    assert(cimagl(zl) == -0.5L);

    // A pure real number used as complex has imaginary part 0
    double _Complex r = 7.0;
    assert(creal(r) == 7.0);
    assert(cimag(r) == 0.0);

    // A pure imaginary number has real part 0
    double _Complex im = 4.0 * I;
    assert(creal(im) == 0.0);
    assert(cimag(im) == 4.0);
}

// ------------------------------------------------------------------
// 3. The I macro and _Complex_I
// ------------------------------------------------------------------

void test_i_macro(void) {
    // I is the imaginary unit: I * I == -1
    double _Complex i = I;
    assert(creal(i) == 0.0);
    assert(cimag(i) == 1.0);

    double _Complex i2 = I * I;
    assert(creal(i2) == -1.0);
    assert(cimag(i2) == 0.0);

    // _Complex_I is the same as I
    double _Complex j = _Complex_I;
    assert(creal(j) == 0.0);
    assert(cimag(j) == 1.0);

    // Type of I is `float _Complex` (glibc: 1.0iF, no const)
    assert(TYPE_IS(I,          float _Complex));
    assert(TYPE_IS(_Complex_I, float _Complex));
}

// ------------------------------------------------------------------
// 4. Arithmetic
// ------------------------------------------------------------------

void test_arithmetic(void) {
    double _Complex a = 1.0 + 2.0 * I;
    double _Complex b = 3.0 - 4.0 * I;

    // Addition
    double _Complex s = a + b;
    assert(creal(s) == 4.0 && cimag(s) == -2.0);

    // Subtraction
    double _Complex d = a - b;
    assert(creal(d) == -2.0 && cimag(d) == 6.0);

    // Multiplication: (1+2i)(3-4i) = 3 - 4i + 6i - 8i^2 = 3 + 2i + 8 = 11 + 2i
    double _Complex p = a * b;
    assert(creal(p) == 11.0 && cimag(p) == 2.0);

    // Division: (1+2i)/(3-4i) = (1+2i)(3+4i)/25 = (3+4i+6i+8i^2)/25
    //         = (3 + 10i - 8)/25 = (-5 + 10i)/25 = -0.2 + 0.4i
    double _Complex q = a / b;
    assert(fabs(creal(q) - (-0.2)) < 1e-12);
    assert(fabs(cimag(q) - 0.4) < 1e-12);

    // Unary minus
    double _Complex n = -a;
    assert(creal(n) == -1.0 && cimag(n) == -2.0);

    // Unary plus
    double _Complex u = +a;
    assert(creal(u) == 1.0 && cimag(u) == 2.0);
}

// ------------------------------------------------------------------
// 4b. Compound assignment (+=, -=, *=, /=)
// ------------------------------------------------------------------

void test_compound_assign(void) {
    double _Complex a = 1.0 + 2.0 * I;
    double _Complex b = 3.0 - 4.0 * I;

    a += b;
    assert(creal(a) == 4.0 && cimag(a) == -2.0);

    a -= b;
    assert(creal(a) == 1.0 && cimag(a) == 2.0);

    a *= b;
    assert(creal(a) == 11.0 && cimag(a) == 2.0);

    a /= b;
    assert(fabs(creal(a) - 1.0) < 1e-12);
    assert(fabs(cimag(a) - 2.0) < 1e-12);

    // Compound assignment with a real operand acts on both parts via
    // conversion of the real operand to complex.
    a += 1.0;
    assert(creal(a) == 2.0 && cimag(a) == 2.0);
    a -= 1.0;
    assert(creal(a) == 1.0 && cimag(a) == 2.0);
    a *= 2.0;
    assert(creal(a) == 2.0 && cimag(a) == 4.0);
    a /= 2.0;
    assert(creal(a) == 1.0 && cimag(a) == 2.0);
}

// ------------------------------------------------------------------
// 5. Conjugate, absolute value, argument, projection
// ------------------------------------------------------------------

void test_functions(void) {
    double _Complex z = 3.0 + 4.0 * I;

    // Absolute value (modulus): |3 + 4i| = 5
    double m = cabs(z);
    assert(m == 5.0);

    // Argument: arg(3 + 4i) = atan2(4, 3)
    double arg = carg(z);
    assert(fabs(arg - atan2(4.0, 3.0)) < 1e-12);

    // Conjugate: conj(3 + 4i) = 3 - 4i
    double _Complex c = conj(z);
    assert(creal(c) == 3.0 && cimag(c) == -4.0);

    // Projection: cproj(z) is z unless z is infinite; returns a finite complex
    double _Complex p = cproj(z);
    assert(creal(p) == 3.0 && cimag(p) == 4.0);

    // Proj on infinity yields +inf + 0*i
    double _Complex inf_z = INFINITY + 2.0 * I;
    double _Complex pinf = cproj(inf_z);
    assert(isinf(creal(pinf)) && cimag(pinf) == 0.0);

    // Float and long double variants
    float _Complex zf = 3.0f + 4.0f * I;
    assert(cabsf(zf) == 5.0f);
    assert(cargf(zf) > 0.0f);
    assert(crealf(conjf(zf)) == 3.0f);

    long double _Complex zl = 3.0L + 4.0L * I;
    assert(cabsl(zl) == 5.0L);
    assert(cargl(zl) > 0.0L);
    assert(creall(conjl(zl)) == 3.0L);
}

// ------------------------------------------------------------------
// 6. Comparisons (only == and != are meaningful)
// ------------------------------------------------------------------

void test_comparison(void) {
    double _Complex a = 1.0 + 2.0 * I;
    double _Complex b = 1.0 + 2.0 * I;
    double _Complex c = 1.0 + 3.0 * I;

    assert(a == b);
    assert(a != c);
}

// ------------------------------------------------------------------
// 7. Conversions and usual arithmetic conversions
// ------------------------------------------------------------------

void test_conversions(void) {
    float _Complex       f = 1.0f + 1.0f * I;
    double _Complex      d = 2.0  + 2.0  * I;
    long double _Complex l = 3.0L + 3.0L * I;

    // float complex + double complex -> double complex
    assert(TYPE_IS(f + d, double _Complex));
    // double complex + long double complex -> long double complex
    assert(TYPE_IS(d + l, long double _Complex));
    // float complex + long double complex -> long double complex
    assert(TYPE_IS(f + l, long double _Complex));

    // complex + real -> complex
    assert(TYPE_IS(d + 1.0,       double _Complex));
    assert(TYPE_IS(d + 1.0f,      double _Complex));
    assert(TYPE_IS(d + 1.0L,      long double _Complex));
    assert(TYPE_IS(d + 1,         double _Complex));

    // complex + int -> complex (real is promoted, then converted)
    assert(TYPE_IS(f + 1,         float _Complex));

    // Conversions between complex types
    double _Complex dconv = (double _Complex)f;
    assert(creal(dconv) == 1.0 && cimag(dconv) == 1.0);

    // Conversion from real to complex
    double _Complex r = 5.0;
    assert(creal(r) == 5.0 && cimag(r) == 0.0);

    // Conversion from complex to real discards the imaginary part (warning)
    double re = creal(d);
    assert(re == 2.0);
}

// ------------------------------------------------------------------
// 8. _Generic on complex types
// ------------------------------------------------------------------

void test_generic(void) {
    float _Complex       f = 1.0f + 0.0f * I;
    double _Complex      d = 1.0  + 0.0  * I;
    long double _Complex l = 1.0L + 0.0L * I;

    assert(TYPE_IS(f, float _Complex));
    assert(TYPE_IS(d, double _Complex));
    assert(TYPE_IS(l, long double _Complex));

    assert(!TYPE_IS(d, double));
    assert(!TYPE_IS(d, float _Complex));

    // _Generic on real-valued expression
    assert(TYPE_IS(1.0, double));
    assert(!TYPE_IS(1.0, double _Complex));
}

// ------------------------------------------------------------------
// 9. Pure-imaginary values (C23 removed the imaginary types;
//    use _Complex with zero real part via _Complex_I).
// ------------------------------------------------------------------

void test_imaginary_types(void) {
    float _Complex       fi = 2.0f * _Complex_I;
    double _Complex      di = 3.0  * _Complex_I;
    long double _Complex li = 4.0L * _Complex_I;

    assert(crealf(fi) == 0.0f);
    assert(cimagf(fi) == 2.0f);

    assert(creal(di) == 0.0);
    assert(cimag(di) == 3.0);

    assert(creall(li) == 0.0L);
    assert(cimagl(li) == 4.0L);

    // _Complex_I is the pure imaginary unit
    assert(crealf(_Complex_I) == 0.0f);
    assert(cimagf(_Complex_I) == 1.0f);
}

// ------------------------------------------------------------------
// 10. Complex compound literals and designators
// ------------------------------------------------------------------

void test_compound_literal(void) {
    double _Complex *p = &(double _Complex){ 3.0 + 4.0 * I };
    assert(creal(*p) == 3.0);
    assert(cimag(*p) == 4.0);

    // Initialize from expression
    double _Complex z = (double _Complex){ 5.0 + 6.0 * I };
    assert(creal(z) == 5.0 && cimag(z) == 6.0);
}

// ------------------------------------------------------------------
// 11. sizeof and alignment
// ------------------------------------------------------------------

void test_sizeof_alignof(void) {
    _Static_assert(sizeof(float _Complex)       == 2 * sizeof(float),       "sizeof float complex");
    _Static_assert(sizeof(double _Complex)      == 2 * sizeof(double),      "sizeof double complex");
    _Static_assert(sizeof(long double _Complex) == 2 * sizeof(long double), "sizeof long double complex");

    _Static_assert(_Alignof(float _Complex)       == _Alignof(float),       "align float complex");
    _Static_assert(_Alignof(double _Complex)      == _Alignof(double),      "align double complex");
    _Static_assert(_Alignof(long double _Complex) == _Alignof(long double), "align long double complex");
}

// ------------------------------------------------------------------
// 12. Imaginary parts via construction
// ------------------------------------------------------------------

void test_construction(void) {
    // Building complex from parts
    double re = 2.5, im = -1.5;
    double _Complex z = re + im * I;
    assert(creal(z) == re);
    assert(cimag(z) == im);

    // Pure real
    double _Complex r = 3.0 + 0.0 * I;
    assert(cimag(r) == 0.0);

    // Pure imaginary
    double _Complex i = 0.0 + 5.0 * I;
    assert(creal(i) == 0.0);
    assert(cimag(i) == 5.0);

    // CMPLX macro (C11)
    double _Complex cm = CMPLX(1.0, 2.0);
    assert(creal(cm) == 1.0 && cimag(cm) == 2.0);

    // Also float and long double variants
    assert(crealf(CMPLXF(1.5f, 2.5f)) == 1.5f);
    assert(cimagl(CMPLXL(1.5L, 2.5L)) == 2.5L);
}

// ------------------------------------------------------------------
// Main
// ------------------------------------------------------------------

int main(void) {
    test_types();
    test_parts();
    test_i_macro();
    test_arithmetic();
    test_compound_assign();
    test_functions();
    test_comparison();
    test_conversions();
    test_generic();
    test_imaginary_types();
    test_compound_literal();
    test_sizeof_alignof();
    test_construction();
    return 0;
}
