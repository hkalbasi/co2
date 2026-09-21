//@ mode: c
//@ run-status: 0
// Test for C23 floating-point types: float, double, long double,
// _Float16, _Float32, _Float64, _Float128, _Float32x, _Float64x, _Float128x,
// and the GNU extension __float128.

#include <stddef.h>
#include <assert.h>
#include <float.h>
#include <math.h>

// ------------------------------------------------------------------
// Helper: compile-time type check
// ------------------------------------------------------------------
#define TYPE_IS(expr, type) _Generic((expr), type: 1, default: 0)

// _Float16 (binary16)
#ifdef __FLT16_MAX__
#define HAS_FLOAT16 1
#else
#define HAS_FLOAT16 0
#endif

// _Float32 (binary32)
#ifdef __FLT32_MAX__
#define HAS_FLOAT32 1
#else
#define HAS_FLOAT32 0
#endif

// _Float64 (binary64)
#ifdef __FLT64_MAX__
#define HAS_FLOAT64 1
#else
#define HAS_FLOAT64 0
#endif

// _Float128 (binary128)
#ifdef __FLT128_MAX__
#define HAS_FLOAT128 1
#else
#define HAS_FLOAT128 0
#endif

// _Float32x (extended binary32)
#ifdef __FLT32X_MAX__
#define HAS_FLOAT32X 1
#else
#define HAS_FLOAT32X 0
#endif

// _Float64x (extended binary64)
#ifdef __FLT64X_MAX__
#define HAS_FLOAT64X 1
#else
#define HAS_FLOAT64X 0
#endif

// _Float128x (extended binary128)
#ifdef __FLT128X_MAX__
#define HAS_FLOAT128X 1
#else
#define HAS_FLOAT128X 0
#endif

// ------------------------------------------------------------------
// GNU __float128 (always available on x86_64 with GCC)
// ------------------------------------------------------------------
#ifdef __GNUC__
#define HAS_GNU_FLOAT128 1
#else
#define HAS_GNU_FLOAT128 0
#endif

// _Static_assert(
//     HAS_FLOAT16
//     && HAS_FLOAT32
//     && HAS_FLOAT64
//     && HAS_FLOAT128
//     && HAS_FLOAT32X
//     && HAS_FLOAT64X
//     && HAS_FLOAT128X
//     && HAS_GNU_FLOAT128,
//     "x86_64 is expected to have all types"
// );

// ------------------------------------------------------------------
// 1. Standard floating types
// ------------------------------------------------------------------

void test_standard_floats(void) {
    float  f  = 1.0f;
    double d  = 2.0;
    long double ld = 3.0L;

    assert(TYPE_IS(f, float));
    assert(TYPE_IS(d, double));
    assert(TYPE_IS(ld, long double));

    // Literal suffixes
    assert(TYPE_IS(1.0f, float));
    assert(TYPE_IS(1.0, double));
    assert(TYPE_IS(1.0L, long double));

    // Arithmetic conversions
    assert(TYPE_IS(f + d, double));
    assert(TYPE_IS(f + ld, long double));
    assert(TYPE_IS(d + ld, long double));

    // sizeof / alignment
    _Static_assert(sizeof(float) >= 4, "float too small");
    _Static_assert(sizeof(double) >= 8, "double too small");
    _Static_assert(sizeof(long double) >= sizeof(double), "long double too small");
}

// ------------------------------------------------------------------
// 2. _Float16
// ------------------------------------------------------------------

void test_float16(void) {
#if HAS_FLOAT16
    _Float16 h = 1.0f16;
    _Float16 h2 = 2.0f16;

    assert(TYPE_IS(h, _Float16));
    assert(TYPE_IS(1.0f16, _Float16));
    assert(TYPE_IS(1.0F16, _Float16));

    // Literal suffix f16 / F16
    assert(TYPE_IS(3.14f16, _Float16));

    // Size and alignment
    _Static_assert(sizeof(_Float16) == 2, "_Float16 must be 2 bytes");
    _Static_assert(_Alignof(_Float16) == 2, "_Float16 alignment");

    // Arithmetic
    _Float16 sum = h + h2;
    assert(sum == 3.0f16);

    // Conversions
    assert(TYPE_IS(h + 1.0f, float));      // _Float16 + float -> float
    assert(TYPE_IS(h + 1.0, double));      // _Float16 + double -> double
    assert(TYPE_IS(h + 1.0L, long double)); // _Float16 + long double -> long double
    assert(TYPE_IS(h + 1, _Float16));         // _Float16 + int -> _Float16

    // Limits
    assert(__FLT16_MAX__ > 0);
    assert(__FLT16_MIN__ > 0);

    // Default argument promotion: _Float16 -> double in variadic
    // (C23 6.5.2.2p6)
    // We can test with a variadic function.
    // (Implementation-defined but usually double)
#else
    // Type not available – compile-time check only.
    _Static_assert(1, "no _Float16");
#endif
}

// ------------------------------------------------------------------
// 3. _Float32
// ------------------------------------------------------------------

void test_float32(void) {
#if HAS_FLOAT32
    _Float32 f32 = 1.0f32;
    assert(TYPE_IS(f32, _Float32));
    assert(TYPE_IS(1.0f32, _Float32));
    assert(TYPE_IS(1.0F32, _Float32));

    _Static_assert(sizeof(_Float32) == 4, "_Float32 must be 4 bytes");
    _Static_assert(_Alignof(_Float32) == 4, "_Float32 alignment");

    // _Float32 is compatible with float but distinct type in _Generic
    // (They have the same representation.)
    _Float32 a = 2.0f32;
    float    b = 3.0f;
    assert(TYPE_IS(a + b, _Float32)); // usual arithmetic conversion

    // Limits
    assert(__FLT32_MAX__ > 0);
    assert(__FLT32_MIN__ > 0);
#else
    _Static_assert(1, "no _Float32");
#endif
}

// ------------------------------------------------------------------
// 4. _Float64
// ------------------------------------------------------------------

void test_float64(void) {
#if HAS_FLOAT64
    _Float64 f64 = 1.0f64;
    assert(TYPE_IS(f64, _Float64));
    assert(TYPE_IS(1.0f64, _Float64));
    assert(TYPE_IS(1.0F64, _Float64));

    _Static_assert(sizeof(_Float64) == 8, "_Float64 must be 8 bytes");
    _Static_assert(_Alignof(_Float64) == 8, "_Float64 alignment");

    _Float64 a = 2.0f64;
    double   b = 3.0;
    assert(TYPE_IS(a + b, _Float64));

    assert(__FLT64_MAX__ > 0);
    assert(__FLT64_MIN__ > 0);
#else
    _Static_assert(1, "no _Float64");
#endif
}

// ------------------------------------------------------------------
// 5. _Float128
// ------------------------------------------------------------------

void test_float128(void) {
#if HAS_FLOAT128
    _Float128 f128 = 1.0f128;
    assert(TYPE_IS(f128, _Float128));
    assert(TYPE_IS(1.0f128, _Float128));
    assert(TYPE_IS(1.0F128, _Float128));

    // Size: typically 16 bytes
    _Static_assert(sizeof(_Float128) >= 16, "_Float128 must be at least 16 bytes");
    _Static_assert(_Alignof(_Float128) >= 16, "_Float128 alignment");

    // Arithmetic
    _Float128 a = 2.0f128;
    _Float128 b = 3.0f128;
    _Float128 sum = a + b;
    assert(sum == 5.0f128);

    // Conversions
    assert(TYPE_IS(a + 1.0, _Float128));      // _Float128 + double -> _Float128
    assert(TYPE_IS(a + 1.0f, _Float128));     // _Float128 + float -> _Float128
    assert(TYPE_IS(a + 1.0L, _Float128));     // _Float128 + long double -> _Float128
    assert(TYPE_IS(a + 1, _Float128));        // _Float128 + int -> _Float128

    // Limits
    assert(__FLT128_MAX__ > 0);
    assert(__FLT128_MIN__ > 0);
#else
    _Static_assert(1, "no _Float128");
#endif
}

// ------------------------------------------------------------------
// 6. Extended types: _Float32x, _Float64x, _Float128x
// ------------------------------------------------------------------

void test_extended_floats(void) {
#if HAS_FLOAT32X
    _Float32x f32x = 1.0f32x;
    assert(TYPE_IS(f32x, _Float32x));
    assert(TYPE_IS(1.0f32x, _Float32x));
    _Static_assert(sizeof(_Float32x) >= 4, "_Float32x size");
    assert(__FLT32X_MAX__ > 0);
#else
    _Static_assert(1, "no _Float32x");
#endif

#if HAS_FLOAT64X
    _Float64x f64x = 1.0f64x;
    assert(TYPE_IS(f64x, _Float64x));
    assert(TYPE_IS(1.0f64x, _Float64x));
    _Static_assert(sizeof(_Float64x) >= 8, "_Float64x size");
    assert(__FLT64X_MAX__ > 0);
#else
    _Static_assert(1, "no _Float64x");
#endif

#if HAS_FLOAT128X
    _Float128x f128x = 1.0f128x;
    assert(TYPE_IS(f128x, _Float128x));
    assert(TYPE_IS(1.0f128x, _Float128x));
    _Static_assert(sizeof(_Float128x) >= 16, "_Float128x size");
    assert(__FLT128X_MAX__ > 0);
#else
    _Static_assert(1, "no _Float128x");
#endif
}

// ------------------------------------------------------------------
// 7. GNU __float128
// ------------------------------------------------------------------

void test_gnu_float128(void) {
#if HAS_GNU_FLOAT128
    __float128 q = 1.0;
    assert(TYPE_IS(q, __float128));

    // Size
    _Static_assert(sizeof(__float128) >= 16, "__float128 size");

    // Arithmetic
    __float128 a = 2.0;
    __float128 b = 3.0;
    assert(a + b == 5.0);

    // Conversions
    assert(TYPE_IS(a + 1.0, __float128));
    assert(TYPE_IS(a + 1.0f, __float128));
    assert(TYPE_IS(a + 1, __float128));
#else
    _Static_assert(1, "no __float128");
#endif
}

// ------------------------------------------------------------------
// 8. Variadic promotion
// ------------------------------------------------------------------

#include <stdarg.h>

int check_variadic(int dummy, ...) {
    va_list ap;
    va_start(ap, dummy);
    // Just check that _Float16 is promoted to double (C23 6.5.2.2p6)
    // We can't easily check the type via va_arg without knowing the type,
    // but we can at least pass a _Float16 and retrieve it as double.
    double d = va_arg(ap, double);
    va_end(ap);
    return (d > 0.0);
}

void test_variadic_promotion(void) {
#if HAS_FLOAT16
    _Float16 h = 1.0f16;
    assert(check_variadic(0, h) == 1);
#endif
}

// ------------------------------------------------------------------
// 9. _Generic with all types
// ------------------------------------------------------------------

void test_generic(void) {
    float  f  = 1.0f;
    double d  = 2.0;
    long double ld = 3.0L;

    assert(TYPE_IS(f, float));
    assert(TYPE_IS(d, double));
    assert(TYPE_IS(ld, long double));

#if HAS_FLOAT16
    _Float16 h = 1.0f16;
    assert(TYPE_IS(h, _Float16));
    assert(!TYPE_IS(h, float));   // distinct type
    assert(!TYPE_IS(h, double));
#endif

#if HAS_FLOAT32
    _Float32 f32 = 1.0f32;
    assert(TYPE_IS(f32, _Float32));
    assert(!TYPE_IS(f32, double));
#endif

#if HAS_FLOAT64
    _Float64 f64 = 1.0f64;
    assert(TYPE_IS(f64, _Float64));
    assert(!TYPE_IS(f64, float));
#endif

#if HAS_FLOAT128
    _Float128 f128 = 1.0f128;
    assert(TYPE_IS(f128, _Float128));
#endif

#if HAS_GNU_FLOAT128
    __float128 q = 1.0;
    assert(TYPE_IS(q, __float128));
    // __float128 may be the same as _Float128 on some platforms;
    // on x86_64 they are distinct.
    // We don't assert distinctness because it's implementation-defined.
#endif
}

// ------------------------------------------------------------------
// Main
// ------------------------------------------------------------------

int main(void) {
    test_standard_floats();
    test_float16();
    test_float32();
    test_float64();
    test_float128();
    test_extended_floats();
    test_gnu_float128();
    test_variadic_promotion();
    test_generic();
    return 0;
}
