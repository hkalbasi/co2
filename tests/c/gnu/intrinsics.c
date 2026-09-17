//@ mode: c
//@ run-status: 0
//@ compile-flags: -lm

#include <assert.h>

// --- math ---

double test_ceil(double x) {
    return __builtin_ceil(x);
}

double test_floor(double x) {
    return __builtin_floor(x);
}

double test_round(double x) {
    return __builtin_round(x);
}

double test_trunc(double x) {
    return __builtin_trunc(x);
}

double test_sqrt(double x) {
    return __builtin_sqrt(x);
}

double test_fabs(double x) {
    return __builtin_fabs(x);
}

// --- fp comparisons / classification (no libc call; also fixes the plain
// macros like isgreater() that glibc headers map onto these builtins) ---

int test_fp(double x, double y) {
    if (!__builtin_isgreater(2.0, 1.0))
        return 1;
    if (__builtin_isless(2.0, 1.0))
        return 2;
    if (!__builtin_isgreaterequal(1.0, 1.0))
        return 3;
    if (!__builtin_islessequal(1.0, 1.0))
        return 4;
    if (!__builtin_islessgreater(1.0, 2.0))
        return 5;
    if (__builtin_islessgreater(1.0, 1.0))
        return 6;
    if (!__builtin_isunordered(__builtin_nan(""), 1.0))
        return 7;
    if (__builtin_isunordered(1.0, 2.0))
        return 8;
    if (!__builtin_iseqsig(1.0, 1.0))
        return 9;
    if (__builtin_isinf(x))
        return 10;
    if (!__builtin_isinf(__builtin_inf()))
        return 11;
    if (!__builtin_isnan(__builtin_nan("")))
        return 12;
    if (__builtin_isnan(x))
        return 13;
    if (!__builtin_isfinite(x))
        return 14;
    if (__builtin_isfinite(__builtin_inf()))
        return 15;
    if (!__builtin_isnormal(x))
        return 16;
    if (__builtin_isnormal(0.0))
        return 17;
    if (__builtin_isinf_sign(-__builtin_inf()) != -1)
        return 18;
    if (__builtin_isinf_sign(__builtin_inf()) != 1)
        return 19;
    if (__builtin_isinf_sign(x) != 0)
        return 20;
    if (__builtin_signbit(-1.0) == 0)
        return 21;
    if (__builtin_signbit(1.0) != 0)
        return 22;
    if (__builtin_fpclassify(0, 1, 4, 3, 2, 1.0) != 4)
        return 23;
    if (__builtin_fpclassify(0, 1, 4, 3, 2, __builtin_nan("")) != 0)
        return 24;
    if (__builtin_fpclassify(0, 1, 4, 3, 2, __builtin_inf()) != 1)
        return 25;
    if (__builtin_fpclassify(0, 1, 4, 3, 2, 0.0) != 2)
        return 26;
    if (__builtin_isgreater(x, y) != (x > y))
        return 27;
    return 0;
}

// --- libc forwarding across headers ---

int test_libc(void) {
    if (__builtin_abs(-7) != 7)
        return 1;
    if (__builtin_labs(-7L) != 7L)
        return 2;
    if (__builtin_strlen("hello") != 5)
        return 3;
    if (__builtin_strcmp("abc", "abd") >= 0)
        return 4;
    if (__builtin_memcmp("ab", "aa", 2) <= 0)
        return 5;
    char buf[16];
    __builtin_memset(buf, 0, sizeof(buf));
    __builtin_memcpy(buf, "hi", 3);
    if (__builtin_strcmp(buf, "hi") != 0)
        return 6;
    if (__builtin_toupper('a') != 'A')
        return 7;
    if (__builtin_tolower('Z') != 'z')
        return 8;
    if (!__builtin_isdigit('5'))
        return 9;
    if (__builtin_isdigit('x'))
        return 10;
    char out[32];
    if (__builtin_snprintf(out, sizeof(out), "%d", 42) != 2)
        return 11;
    if (__builtin_strcmp(out, "42") != 0)
        return 12;
    int v = 0;
    if (__builtin_sscanf("123", "%d", &v) != 1 || v != 123)
        return 13;
    void *p = __builtin_malloc(64);
    if (!p)
        return 14;
    __builtin_free(p);
    return 0;
}

int test_bits(void) {
    if (__builtin_ffs(0) != 0)
        return 1;
    if (__builtin_ffs(1) != 1)
        return 2;
    if (__builtin_ffs(0x10) != 5)
        return 3;
    if (__builtin_clz(1u) != 31)
        return 4;
    if (__builtin_clz(0x80000000u) != 0)
        return 5;
    if (__builtin_ctz(0x80000000u) != 31)
        return 6;
    if (__builtin_ctz(2u) != 1)
        return 7;
    if (__builtin_clrsb(0) != 31)
        return 8;
    if (__builtin_clrsb(-1) != 31)
        return 9;
    if (__builtin_clrsb(1) != 30)
        return 10;
    if (__builtin_popcount(0u) != 0)
        return 11;
    if (__builtin_popcount(0xffu) != 8)
        return 12;
    if (__builtin_parity(0x7u) != 1)
        return 13;
    if (__builtin_parity(0x3u) != 0)
        return 14;
    if (__builtin_ffsl(0L) != 0)
        return 15;
    if (__builtin_ffsl(0x100L) != 9)
        return 16;
    if (__builtin_clzl(1ul) != 63)
        return 17;
    if (__builtin_ctzl(0x8000000000000000ul) != 63)
        return 18;
    if (__builtin_clrsbl(0L) != 63)
        return 19;
    if (__builtin_popcountl(0xfffffffffffffffful) != 64)
        return 20;
    if (__builtin_parityl(0xfL) != 0)
        return 21;
    if (__builtin_ffsll(0LL) != 0)
        return 22;
    if (__builtin_clzll(1ull) != 63)
        return 23;
    if (__builtin_ctzll(1ull << 40) != 40)
        return 24;
    if (__builtin_clrsbll(-1LL) != 63)
        return 25;
    if (__builtin_popcountll(0x3ull) != 2)
        return 26;
    if (__builtin_parityll(0x7ull) != 1)
        return 27;
    if (__builtin_ffsg(0) != 0)
        return 28;
    if (__builtin_ffsg(0x20) != 6)
        return 29;
    if (__builtin_clzg(1u) != 31)
        return 30;
    if (__builtin_clzg(0u, 32) != 32)
        return 31;
    if (__builtin_ctzg(0u, 32) != 32)
        return 32;
    if (__builtin_ctzg(8u) != 3)
        return 33;
    if (__builtin_clrsbg((signed char)0) != 7)
        return 34;
    if (__builtin_popcountg(0xffffull) != 16)
        return 35;
    if (__builtin_parityg(0x7ull) != 1)
        return 36;
    if (__builtin_stdc_bit_ceil(0u) != 1)
        return 37;
    if (__builtin_stdc_bit_ceil(65u) != 128u)
        return 38;
    if (__builtin_stdc_bit_floor(65u) != 64u)
        return 39;
    if (__builtin_stdc_bit_floor(0u) != 0u)
        return 40;
    if (__builtin_stdc_bit_width(65u) != 7)
        return 41;
    if (__builtin_stdc_bit_width(0u) != 0)
        return 42;
    if (__builtin_stdc_count_ones(0xffu) != 8)
        return 43;
    if (__builtin_stdc_count_zeros(0xffu) != 24)
        return 44;
    if (__builtin_stdc_first_leading_one(1u) != 32)
        return 45;
    if (__builtin_stdc_first_leading_zero(0xffffffffu) != 0)
        return 46;
    if (__builtin_stdc_first_trailing_one(0x40u) != 7)
        return 47;
    if (__builtin_stdc_first_trailing_zero(0xffffffffu) != 0)
        return 48;
    if (__builtin_stdc_has_single_bit(64u) != 1)
        return 49;
    if (__builtin_stdc_has_single_bit(65u) != 0)
        return 50;
    if (__builtin_stdc_leading_ones(0xffffffffu) != 32)
        return 51;
    if (__builtin_stdc_leading_zeros(0xffffffffu) != 0)
        return 52;
    if (__builtin_stdc_trailing_ones(0x7u) != 3)
        return 53;
    if (__builtin_stdc_trailing_zeros(0x8u) != 3)
        return 54;
    if (__builtin_stdc_rotate_left(0x80000001u, 2) != 0x00000006u)
        return 55;
    if (__builtin_stdc_rotate_right(0x80000001u, 2) != 0x60000000u)
        return 56;
    if (__builtin_stdc_rotate_left(0x80u, (unsigned char)0) != 0x80u)
        return 57;
    return 0;
}

int main(void) {
    assert(test_ceil(1.5) == 2.0);
    assert(test_ceil(-1.5) == -1.0);
    assert(test_floor(1.5) == 1.0);
    assert(test_floor(-1.5) == -2.0);
    assert(test_round(2.5) == 3.0);
    assert(test_round(-2.5) == -3.0);
    assert(test_trunc(1.9) == 1.0);
    assert(test_trunc(-1.9) == -1.0);
    double s = test_sqrt(2.0);
    assert(s > 1.41 && s < 1.42);
    assert(test_fabs(-3.0) == 3.0);
    assert(test_fabs(3.0) == 3.0);

    // Suffixed variants.
    assert(__builtin_ceilf(1.5f) == 2.0f);
    assert(__builtin_floorf(-1.5f) == -2.0f);
    assert(__builtin_roundf(2.5f) == 3.0f);
    assert(__builtin_truncf(-1.9f) == -1.0f);
    assert(__builtin_sqrtf(4.0f) == 2.0f);
    assert(__builtin_fabsf(-3.0f) == 3.0f);

    // More forwarded math.
    assert(__builtin_sin(0.0) == 0.0);
    assert(__builtin_cos(0.0) == 1.0);
    assert(__builtin_pow(2.0, 10.0) == 1024.0);
    assert(__builtin_fmod(5.5, 2.0) == 1.5);
    assert(__builtin_fmax(1.0, 2.0) == 2.0);
    assert(__builtin_fmin(1.0, 2.0) == 1.0);
    assert(__builtin_exp(0.0) == 1.0);
    assert(__builtin_log(1.0) == 0.0);
    assert(__builtin_atan2(0.0, 1.0) == 0.0);
    assert(__builtin_lrint(1.5) == 2L);
    assert(__builtin_llround(2.5) == 3LL);
    assert(__builtin_ilogb(8.0) == 3);

    // Repeated use in one scope redeclares the local extern cleanly.
    assert(__builtin_ceil(0.5) == 1.0);
    assert(__builtin_ceil(0.5) == 1.0);

    assert(test_fp(1.5, 2.5) == 0);
    assert(test_libc() == 0);
    assert(test_bits() == 0);

    return 0;
}
