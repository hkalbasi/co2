//@ mode: c
//@ run-status: 0
//@ compile-flags: -lm
// Test for GCC extension: integer complex types (int _Complex, etc.)
// and their interaction / implicit conversions with float complex types.
// This is a GNU extension, not standard C.

#include <stddef.h>
#include <assert.h>
#include <complex.h>
#include <limits.h>
#include <math.h>

// ------------------------------------------------------------------
// Compile-time type check
// ------------------------------------------------------------------
#define TYPE_IS(expr, type) _Generic((expr), type: 1, default: 0)

// ------------------------------------------------------------------
// 1. Basic integer complex type declarations (GNU extension)
// ------------------------------------------------------------------

void test_int_complex_types(void) {
    // All standard integer types can be qualified with _Complex in GCC.
    char               _Complex zc = 1 + 2 * I;
    short              _Complex zs = 3 + 4 * I;
    int                _Complex zi = 5 + 6 * I;
    long               _Complex zl = 7 + 8 * I;
    long long          _Complex zll = 9 + 10 * I;

    unsigned char      _Complex zuc = 1 + 2 * I;
    unsigned short     _Complex zus = 3 + 4 * I;
    unsigned int       _Complex zui = 5 + 6 * I;
    unsigned long      _Complex zul = 7 + 8 * I;
    unsigned long long _Complex zull = 9 + 10 * I;

    (void)zc; (void)zs; (void)zi; (void)zl; (void)zll;
    (void)zuc; (void)zus; (void)zui; (void)zul; (void)zull;

    // Type checks
    assert(TYPE_IS(zc,   char _Complex));
    assert(TYPE_IS(zs,   short _Complex));
    assert(TYPE_IS(zi,   int _Complex));
    assert(TYPE_IS(zl,   long _Complex));
    assert(TYPE_IS(zll,  long long _Complex));
    assert(TYPE_IS(zui,  unsigned int _Complex));
    assert(TYPE_IS(zul,  unsigned long _Complex));
    assert(TYPE_IS(zull, unsigned long long _Complex));

    // Size is twice the size of the corresponding real type
    _Static_assert(sizeof(char _Complex)      == 2 * sizeof(char),      "char complex size");
    _Static_assert(sizeof(short _Complex)     == 2 * sizeof(short),     "short complex size");
    _Static_assert(sizeof(int _Complex)       == 2 * sizeof(int),       "int complex size");
    _Static_assert(sizeof(long _Complex)      == 2 * sizeof(long),      "long complex size");
    _Static_assert(sizeof(long long _Complex) == 2 * sizeof(long long), "long long complex size");

    _Static_assert(_Alignof(int _Complex)  == _Alignof(int),  "int complex align");
    _Static_assert(_Alignof(long _Complex) == _Alignof(long), "long complex align");
}

// ------------------------------------------------------------------
// 2. Constructing and inspecting integer complex values
//    creal/cimag work because int complex promotes to double complex
//    implicitly when passed to those functions.
// ------------------------------------------------------------------

void test_int_complex_construction(void) {
    int _Complex z = 3 + 4 * I;

    // Implicit conversion to double _Complex for library calls
    double _Complex zd = z;
    assert(creal(zd) == 3.0);
    assert(cimag(zd) == 4.0);

    // Direct arithmetic inspection: the real/imaginary parts are accessed
    // via the CMPLX-like splitting, but for integer complex we must
    // convert to float complex first (or use the extension's layout).
    int re = (int)creal(z);
    int im = (int)cimag(z);
    assert(re == 3);
    assert(im == 4);

    // From a value expression
    int _Complex z2 = 10;
    assert(creal((double _Complex)z2) == 10.0);
    assert(cimag((double _Complex)z2) == 0.0);

    // Pure imaginary (constructed via I)
    int _Complex z3 = 5 * I;
    assert(creal((double _Complex)z3) == 0.0);
    assert(cimag((double _Complex)z3) == 5.0);
}

// ------------------------------------------------------------------
// 3. Integer complex arithmetic (stays integer complex)
// ------------------------------------------------------------------

void test_int_complex_arithmetic(void) {
    int _Complex a = 1 + 2 * I;
    int _Complex b = 3 - 4 * I;

    // Addition
    int _Complex s = a + b;
    double _Complex sd = s;
    assert(creal(sd) == 4.0 && cimag(sd) == -2.0);

    // Subtraction
    int _Complex d = a - b;
    double _Complex dd = d;
    assert(creal(dd) == -2.0 && cimag(dd) == 6.0);

    // Multiplication: (1+2i)(3-4i) = 3 - 4i + 6i - 8i² = 11 + 2i
    int _Complex p = a * b;
    double _Complex pd = p;
    assert(creal(pd) == 11.0 && cimag(pd) == 2.0);

    // Result type is int _Complex (no conversion)
    assert(TYPE_IS(a + b, int _Complex));
    assert(TYPE_IS(a - b, int _Complex));
    assert(TYPE_IS(a * b, int _Complex));

    // Division is not supported for integer complex (GCC error).
    // Skipping.

    // Unary minus / plus
    int _Complex n = -a;
    double _Complex nd = n;
    assert(creal(nd) == -1.0 && cimag(nd) == -2.0);

    int _Complex u = +a;
    double _Complex ud = u;
    assert(creal(ud) == 1.0 && cimag(ud) == 2.0);
}

// ------------------------------------------------------------------
// 4. Integer complex + real integer / real float: conversions
// ------------------------------------------------------------------

void test_int_complex_mixed(void) {
    int _Complex z = 1 + 2 * I;

    // int _Complex + int -> int _Complex
    assert(TYPE_IS(z + 1, int _Complex));

    // int _Complex + float -> float _Complex
    assert(TYPE_IS(z + 1.0f, float _Complex));

    // int _Complex + double -> double _Complex
    assert(TYPE_IS(z + 1.0, double _Complex));

    // int _Complex + long double -> long double _Complex
    assert(TYPE_IS(z + 1.0L, long double _Complex));

    // Value checks
    double _Complex r1 = z + 10;
    assert(creal(r1) == 11.0 && cimag(r1) == 2.0);

    double _Complex r2 = z + 0.5;
    assert(creal(r2) == 1.5 && cimag(r2) == 2.0);
}

// ------------------------------------------------------------------
// 5. Interaction between integer complex and float complex
//    (usual arithmetic conversions)
// ------------------------------------------------------------------

void test_int_float_complex_mix(void) {
    int _Complex    zi = 1 + 2 * I;
    float _Complex  zf = 3.0f + 4.0f * I;
    double _Complex zd = 5.0  + 6.0  * I;
    long double _Complex zl = 7.0L + 8.0L * I;

    // int _Complex + float _Complex -> float _Complex
    assert(TYPE_IS(zi + zf, float _Complex));
    // int _Complex + double _Complex -> double _Complex
    assert(TYPE_IS(zi + zd, double _Complex));
    // int _Complex + long double _Complex -> long double _Complex
    assert(TYPE_IS(zi + zl, long double _Complex));

    // float _Complex + double _Complex -> double _Complex
    assert(TYPE_IS(zf + zd, double _Complex));
    // double _Complex + long double _Complex -> long double _Complex
    assert(TYPE_IS(zd + zl, long double _Complex));
    // float _Complex + long double _Complex -> long double _Complex
    assert(TYPE_IS(zf + zl, long double _Complex));

    // Same-rank signed/unsigned mix -> unsigned version wins
    unsigned int _Complex zui = 9 + 10 * I;
    assert(TYPE_IS(zi + zui, unsigned int _Complex));

    // Smaller int complex with int complex -> usual integer promotions
    char  _Complex zc = 1 + 2 * I;
    short _Complex zs = 3 + 4 * I;
    assert(TYPE_IS(zc + zs, short _Complex));
    assert(TYPE_IS(zc + zi, int _Complex));
    assert(TYPE_IS(zs + zi, int _Complex));

    // Mixed int types: long long wins
    long long _Complex zll = 1 + 1 * I;
    assert(TYPE_IS(zi + zll, long long _Complex));
    assert(TYPE_IS(zl + zll, long double _Complex));

    // Value checks
    double _Complex sum = zi + zd;
    assert(creal(sum) == 6.0 && cimag(sum) == 8.0);

    double _Complex sum2 = zf + zd;
    assert(creal(sum2) == 8.0 && cimag(sum2) == 10.0);
}

// ------------------------------------------------------------------
// 6. Implicit casts between complex types
// ------------------------------------------------------------------

void test_implicit_casts(void) {
    // Real -> complex
    double _Complex zd = 5.0;
    assert(creal(zd) == 5.0 && cimag(zd) == 0.0);

    // int -> int _Complex
    int _Complex zi = 7;
    double _Complex zid = zi;
    assert(creal(zid) == 7.0 && cimag(zid) == 0.0);

    // float _Complex -> double _Complex (widening, exact)
    float _Complex zf = 1.5f + 2.5f * I;
    double _Complex zd2 = zf;
    assert(creal(zd2) == 1.5 && cimag(zd2) == 2.5);

    // double _Complex -> float _Complex (narrowing, may lose precision)
    double _Complex zd3 = 1.25 + 2.75 * I;
    float _Complex zf3 = zd3;
    assert(crealf(zf3) == 1.25f && cimagf(zf3) == 2.75f);

    // int _Complex -> float _Complex
    int _Complex zii = 3 + 4 * I;
    float _Complex zfi = zii;
    assert(crealf(zfi) == 3.0f && cimagf(zfi) == 4.0f);

    // Explicit cast to complex: (double _Complex) 3
    double _Complex z = (double _Complex)3;
    assert(creal(z) == 3.0 && cimag(z) == 0.0);

    // Explicit cast int complex to int complex
    int _Complex zc = (int _Complex)(1 + 1 * I);
    double _Complex zcd = zc;
    assert(creal(zcd) == 1.0 && cimag(zcd) == 1.0);
}

// ------------------------------------------------------------------
// 7. Truncation / rounding behavior of the imaginary part
//    When narrowing from a complex with fractional parts, the
//    imaginary part is truncated toward zero (like int cast).
// ------------------------------------------------------------------

void test_narrowing_truncation(void) {
    // double _Complex with fractional real/imag -> int _Complex truncates
    double _Complex zd = 3.75 + 4.25 * I;
    int _Complex zi = (int _Complex)zd;
    double _Complex back = zi;
    assert(creal(back) == 3.0);   // truncated
    assert(cimag(back) == 4.0);

    // Negative values: truncation toward zero
    double _Complex zneg = -3.75 - 4.25 * I;
    int _Complex zineg = (int _Complex)zneg;
    double _Complex backneg = zineg;
    assert(creal(backneg) == -3.0);
    assert(cimag(backneg) == -4.0);

    // float -> int complex
    float _Complex zf = 2.5f - 3.5f * I;
    int _Complex zi2 = (int _Complex)zf;
    double _Complex back2 = zi2;
    assert(creal(back2) == 2.0);
    assert(cimag(back2) == -3.0);

    // double -> float complex
    double _Complex zd2 = 1.0 / 3.0 + 2.0 / 7.0 * I;
    float _Complex zf2 = (float _Complex)zd2;
    // Just ensure it round-trips reasonably within float precision
    assert(fabsf(crealf(zf2) - (float)(1.0 / 3.0)) < 1e-6f);
    assert(fabsf(cimagf(zf2) - (float)(2.0 / 7.0)) < 1e-6f);
}

// ------------------------------------------------------------------
// 8. Integer complex in _Generic
// ------------------------------------------------------------------

void test_generic(void) {
    int _Complex    zi = 1 + 1 * I;
    long _Complex   zl = 1 + 1 * I;
    float _Complex  zf = 1.0f + 1.0f * I;
    double _Complex zd = 1.0  + 1.0  * I;

    assert(TYPE_IS(zi, int _Complex));
    assert(TYPE_IS(zl, long _Complex));
    assert(TYPE_IS(zf, float _Complex));
    assert(TYPE_IS(zd, double _Complex));

    // Distinct types
    assert(!TYPE_IS(zi, int));
    assert(!TYPE_IS(zi, long _Complex));
    assert(!TYPE_IS(zi, float _Complex));

    // Result of mixed arithmetic
    assert(TYPE_IS(zi + zi, int _Complex));
    assert(TYPE_IS(zi + zf, float _Complex));
    assert(TYPE_IS(zi + zd, double _Complex));
    assert(TYPE_IS(zf + zd, double _Complex));
}

// ------------------------------------------------------------------
// 9. Integer complex as function arguments
//    (both explicit types and through variadic promotion)
// ------------------------------------------------------------------

double _Complex take_dcomplex(double _Complex z) {
    return z * 2.0;
}

int _Complex take_icomplex(int _Complex z) {
    return z + 1;
}

void test_function_args(void) {
    int _Complex zi = 3 + 4 * I;

    // int _Complex -> double _Complex (implicit at call)
    double _Complex r = take_dcomplex(zi);
    assert(creal(r) == 6.0 && cimag(r) == 8.0);

    // Direct int _Complex
    int _Complex r2 = take_icomplex(zi);
    double _Complex r2d = r2;
    assert(creal(r2d) == 4.0 && cimag(r2d) == 4.0);

    // int _Complex -> float _Complex at call
    float _Complex r3 = (float _Complex)zi;
    assert(crealf(r3) == 3.0f && cimagf(r3) == 4.0f);
}

// ------------------------------------------------------------------
// 10. Comparison (equality) of integer complex
// ------------------------------------------------------------------

void test_equality(void) {
    int _Complex a = 1 + 2 * I;
    int _Complex b = 1 + 2 * I;
    int _Complex c = 1 + 3 * I;

    assert(a == b);
    assert(a != c);

    // Comparing int complex with float complex
    float _Complex f = 1.0f + 2.0f * I;
    // Implicit conversion of a to float _Complex for the comparison
    assert(a == f);
    assert(c != f);

    // Comparing int complex with real
    int _Complex r = 5 + 0 * I;
    assert(r == 5);
    assert(a != 5);
}

// ------------------------------------------------------------------
// 11. Compound literals with integer complex
// ------------------------------------------------------------------

void test_compound_literal(void) {
    int _Complex *p = &(int _Complex){ 3 + 4 * I };
    double _Complex d = *p;
    assert(creal(d) == 3.0 && cimag(d) == 4.0);

    // Use in expression
    int _Complex z = (int _Complex){ 7 + 8 * I };
    double _Complex zd = z;
    assert(creal(zd) == 7.0 && cimag(zd) == 8.0);
}

// ------------------------------------------------------------------
// 12. sizeof / alignment
// ------------------------------------------------------------------

void test_sizeof_alignof(void) {
    _Static_assert(sizeof(int _Complex)       == 2 * sizeof(int),       "int complex size");
    _Static_assert(sizeof(long _Complex)      == 2 * sizeof(long),      "long complex size");
    _Static_assert(sizeof(long long _Complex) == 2 * sizeof(long long), "long long complex size");
    _Static_assert(sizeof(char _Complex)      == 2 * sizeof(char),      "char complex size");

    _Static_assert(_Alignof(int _Complex)       == _Alignof(int),       "int complex align");
    _Static_assert(_Alignof(long _Complex)      == _Alignof(long),      "long complex align");
    _Static_assert(_Alignof(long long _Complex) == _Alignof(long long), "long long complex align");
}

// ------------------------------------------------------------------
// Main
// ------------------------------------------------------------------

int main(void) {
    test_int_complex_types();
    test_int_complex_construction();
    test_int_complex_arithmetic();
    test_int_complex_mixed();
    test_int_float_complex_mix();
    test_implicit_casts();
    test_narrowing_truncation();
    test_generic();
    test_function_args();
    test_equality();
    test_compound_literal();
    test_sizeof_alignof();
    return 0;
}
