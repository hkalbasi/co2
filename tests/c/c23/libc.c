//@ mode: c
//@ requires-header: stdckdint.h stdbit.h
//@ run-status: 0
//@ run-stdout: FILE: libc.out
//@ compile-warning: this arithmetic operation will overflow

#include <assert.h>
#include <limits.h>
#include <locale.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <uchar.h>
#include <stdckdint.h>
#include <stdbit.h>

#if __STDC_VERSION__ < 202311L
# error "This test requires C23"
#endif

/*
 * --------------------------------------------------------------------------
 * Compile-time libc feature checks
 * --------------------------------------------------------------------------
 */

#ifndef __STDC_VERSION_UCHAR_H__
# error "<uchar.h> does not advertise a C23 version"
#endif

#ifndef __STDC_VERSION_STDCKDINT_H__
# error "<stdckdint.h> does not advertise a C23 version"
#endif

#ifndef __STDC_VERSION_STDBIT_H__
# error "<stdbit.h> does not advertise a C23 version"
#endif

/*
 * char8_t is a typedef, not a distinct fundamental type in C23.
 *
 * Check both its size and that it is suitable for UTF-8 code units.
 */
_Static_assert(sizeof(char8_t) == 1);
_Static_assert(sizeof(u8"") == 1);

/*
 * The u8 literal is now an array of char8_t in C23.
 */
_Static_assert(
    sizeof(u8"hello") == 6,
    "C23 u8 string literal should contain 5 code units + NUL"
);

static_assert(
    sizeof(u8"€") == 4,
    "UTF-8 Euro sign should occupy three code units + NUL"
);


/*
 * --------------------------------------------------------------------------
 * stdbit.h
 * --------------------------------------------------------------------------
 */

static void
test_stdbit(void)
{
    puts("  stdbit.h");

    /*
     * The generic macros must work with different unsigned integer types.
     */
    unsigned char uc = 0x0f;
    unsigned short us = 0x8001;
    unsigned int ui = 0x80000001u;
    unsigned long long ull = 0x8000000000000001ull;

    assert(stdc_count_ones(uc) == 4);
    assert(stdc_count_ones(us) == 2);
    assert(stdc_count_ones(ui) == 2);
    assert(stdc_count_ones(ull) == 2);

    assert(stdc_count_zeros(uc) == CHAR_BIT - 4);
    assert(stdc_count_ones(0u) == 0);
    assert(stdc_count_zeros(0u) == sizeof(unsigned int) * CHAR_BIT);

    assert(stdc_has_single_bit(1u));
    assert(stdc_has_single_bit(2u));
    assert(stdc_has_single_bit(0x80000000u));
    assert(!stdc_has_single_bit(0u));
    assert(!stdc_has_single_bit(3u));

    assert(stdc_bit_width(0u) == 0);
    assert(stdc_bit_width(1u) == 1);
    assert(stdc_bit_width(2u) == 2);
    assert(stdc_bit_width(3u) == 2);
    assert(stdc_bit_width(255u) == 8);

    assert(stdc_bit_floor(0u) == 0);
    assert(stdc_bit_floor(1u) == 1);
    assert(stdc_bit_floor(2u) == 2);
    assert(stdc_bit_floor(3u) == 2);
    assert(stdc_bit_floor(7u) == 4);
    assert(stdc_bit_floor(8u) == 8);
    assert(stdc_bit_floor(9u) == 8);

    assert(stdc_bit_ceil(0u) == 1);
    assert(stdc_bit_ceil(1u) == 1);
    assert(stdc_bit_ceil(2u) == 2);
    assert(stdc_bit_ceil(3u) == 4);
    assert(stdc_bit_ceil(7u) == 8);
    assert(stdc_bit_ceil(8u) == 8);
    assert(stdc_bit_ceil(9u) == 16);

    /*
     * Leading/trailing zero/one operations.
     *
     * Do not use signed types: these APIs operate on unsigned integer
     * types, and zero has deliberately special semantics.
     */
    assert(stdc_trailing_zeros(0x10u) == 4);
    assert(stdc_trailing_zeros(0x11u) == 0);
    assert(stdc_trailing_ones(0x0fu) == 4);
    assert(stdc_trailing_ones(0x10u) == 0);

    assert(stdc_leading_zeros(1u) ==
           sizeof(unsigned int) * CHAR_BIT - 1);

    assert(stdc_leading_ones(~0u) ==
           sizeof(unsigned int) * CHAR_BIT);

    /*
     * "first" operations are one-based.
     */
    assert(stdc_first_trailing_zero(0u) == 1);
    assert(stdc_first_trailing_one(0u) == 0);

    assert(stdc_first_trailing_one(1u) == 1);
    assert(stdc_first_trailing_zero(1u) == 2);

    /*
     * Endianness macros must form a consistent set.
     */
    assert(__STDC_ENDIAN_NATIVE__ == __STDC_ENDIAN_LITTLE__ ||
           __STDC_ENDIAN_NATIVE__ == __STDC_ENDIAN_BIG__);
}


/*
 * --------------------------------------------------------------------------
 * stdckdint.h
 * --------------------------------------------------------------------------
 */

static void
test_stdckdint(void)
{
    puts("  stdckdint.h");

    int result;

    /*
     * Normal operations.
     */
    assert(!ckd_add(&result, 10, 20));
    assert(result == 30);

    assert(!ckd_sub(&result, 30, 20));
    assert(result == 10);

    assert(!ckd_mul(&result, 7, 6));
    assert(result == 42);

    /*
     * Signed overflow.
     */
    result = 0;
    assert(ckd_add(&result, INT_MAX, 1));

    /*
     * The result is the wrapped result, but the important property here
     * is that no signed-overflow UB occurred.
     */
    assert(result == INT_MIN);

    result = 0;
    assert(ckd_sub(&result, INT_MIN, 1));
    assert(result == INT_MAX);

    result = 0;
    assert(ckd_mul(&result, INT_MAX, 2));
    assert(result == -2);

    /*
     * Different operand/result types exercise the type-generic interface.
     */
    int64_t wide;

    assert(!ckd_add(&wide, INT32_MAX, INT32_MAX));
    assert(wide == (int64_t)INT32_MAX * 2);

    /*
     * Result type can itself be narrower than the operands.
     */
    int8_t narrow;

    assert(ckd_add(&narrow, INT16_MAX, 1));

    /*
     * Unsigned arithmetic is checked against the destination type.
     */
    unsigned int u;

    assert(!ckd_add(&u, 1u, 2u));
    assert(u == 3u);

    assert(ckd_add(&u, UINT_MAX, 1u));
    assert(u == 0u);
}


/*
 * --------------------------------------------------------------------------
 * strdup / strndup
 * --------------------------------------------------------------------------
 */

static void
test_strdup_family(void)
{
    puts("  strdup/strndup");

    const char *src = "C23 libc torture test";

    char *copy = strdup(src);
    assert(copy != NULL);
    assert(copy != src);
    assert(strcmp(copy, src) == 0);

    free(copy);

    /*
     * Exact length.
     */
    copy = strndup(src, strlen(src));
    assert(copy != NULL);
    assert(strcmp(copy, src) == 0);
    free(copy);

    /*
     * Truncation must still produce a terminated string.
     */
    copy = strndup(src, 3);
    assert(copy != NULL);
    assert(strcmp(copy, "C23") == 0);
    assert(strlen(copy) == 3);
    free(copy);

    /*
     * size == 0 still produces a valid empty string.
     */
    copy = strndup(src, 0);
    assert(copy != NULL);
    assert(copy[0] == '\0');
    free(copy);

    /*
     * Embedded NUL: strdup follows the first NUL, while strndup copies
     * bytes only until the first NUL.
     */
    const char embedded[] = {
        'a', 'b', '\0', 'c', 'd', '\0'
    };

    copy = strndup(embedded, sizeof embedded);
    assert(copy != NULL);
    assert(strcmp(copy, "ab") == 0);
    free(copy);
}


/*
 * --------------------------------------------------------------------------
 * memset_explicit
 * --------------------------------------------------------------------------
 */

static void
test_memset_explicit(void)
{
    puts("  memset_explicit");

    unsigned char secret[64];

    memset(secret, 0xa5, sizeof secret);

    for (size_t i = 0; i < sizeof secret; ++i)
        assert(secret[i] == 0xa5);

    void *returned =
        memset_explicit(secret, 0, sizeof secret);

    assert(returned == secret);

    for (size_t i = 0; i < sizeof secret; ++i)
        assert(secret[i] == 0);
}


/*
 * --------------------------------------------------------------------------
 * UTF-8 / char8_t
 * --------------------------------------------------------------------------
 */

static void
test_char8(void)
{
    puts("  char8_t / mbrtoc8 / c8rtomb");

    /*
     * C23 UTF-8 literal.
     *
     * "€" = U+20AC = E2 82 AC
     */
    static const char8_t euro[] = u8"€";

    assert(euro[0] == (char8_t)0xe2);
    assert(euro[1] == (char8_t)0x82);
    assert(euro[2] == (char8_t)0xac);
    assert(euro[3] == u8'\0');

    /*
     * Type compatibility: a UTF-8 literal is an array of char8_t.
     */
    const char8_t *p = u8"hello";
    assert(p[0] == u8'h');
    assert(p[4] == u8'o');
    assert(p[5] == u8'\0');

    /*
     * Use the UTF-8 conversion API.
     *
     * The locale determines the source multibyte encoding.  The test
     * deliberately requests UTF-8 because that is the encoding whose
     * interaction with char8_t we are testing.
     */
    const char *old_locale = setlocale(LC_CTYPE, NULL);
    char old_locale_copy[256];

    if (old_locale != NULL) {
        snprintf(old_locale_copy, sizeof old_locale_copy,
                 "%s", old_locale);
    } else {
        old_locale_copy[0] = '\0';
    }

    const char *locale = setlocale(LC_CTYPE, "C.UTF-8");

    /*
     * C23 does not require a "C.UTF-8" locale to exist.
     * If it doesn't, retain the compile/API/type tests and skip the
     * locale-dependent conversion part.
     */
    if (locale == NULL) {
        puts("    C.UTF-8 locale unavailable; conversion part skipped");
        return;
    }

    {
        mbstate_t state = {0};
        char8_t out[8] = {0};

        /*
         * Euro sign is three UTF-8 bytes.
         */
        size_t r = mbrtoc8(out, "\xe2\x82\xac", 3, &state);

        assert(r == 3);
        assert(out[0] == (char8_t)0xe2);

        /*
         * mbrtoc8 is stateful: the remaining UTF-8 code units are
         * obtained by subsequent calls with no new input.
         */
        r = mbrtoc8(&out[1], NULL, 0, &state);
        assert(r == (size_t)-3);
        assert(out[1] == (char8_t)0x82);

        r = mbrtoc8(&out[2], NULL, 0, &state);
        assert(r == (size_t)-3);
        assert(out[2] == (char8_t)0xac);

        /*
         * Complete the sequence with NUL.
         */
        r = mbrtoc8(&out[3], "", 1, &state);
        assert(r == 0);
        assert(out[3] == u8'\0');
    }

    /*
     * Exercise c8rtomb in the other direction.
     */
    {
        mbstate_t state = {0};
        char out[MB_LEN_MAX + 1] = {0};

        size_t r = c8rtomb(out, u8'\xe2', &state);
        assert(r == 0);

        r = c8rtomb(out, u8'\x82', &state);
        assert(r == 0);

        r = c8rtomb(out, u8'\xac', &state);
        assert(r == 3);

        assert((unsigned char)out[0] == 0xe2);
        assert((unsigned char)out[1] == 0x82);
        assert((unsigned char)out[2] == 0xac);

        /*
         * Reset the conversion state.
         */
        r = c8rtomb(out, u8'\0', &state);
        assert(r >= 1);
        assert(out[r - 1] == '\0');
    }

    /*
     * Invalid UTF-8 must be rejected.
     */
    {
        mbstate_t state = {0};
        char8_t out;

        size_t r = mbrtoc8(&out, "\xff", 1, &state);

        assert(r == (size_t)-1);
    }

    /*
     * Restore the original locale.
     */
    if (old_locale_copy[0] != '\0')
        (void)setlocale(LC_CTYPE, old_locale_copy);
}


/*
 * --------------------------------------------------------------------------
 * timespec_getres
 * --------------------------------------------------------------------------
 */

static void
test_timespec_getres(void)
{
    puts("  timespec_getres");

    struct timespec resolution = {0};

    int r = timespec_getres(&resolution, TIME_UTC);

    assert(r == TIME_UTC);

    /*
     * A successful resolution must be a non-negative duration.
     */
    assert(resolution.tv_sec >= 0);
    assert(resolution.tv_nsec >= 0);
    assert(resolution.tv_nsec < 1000000000L);

    /*
     * Calling repeatedly must remain well-defined.
     */
    struct timespec resolution2 = {0};

    r = timespec_getres(&resolution2, TIME_UTC);

    assert(r == TIME_UTC);
    assert(resolution2.tv_sec >= 0);
    assert(resolution2.tv_nsec >= 0);
    assert(resolution2.tv_nsec < 1000000000L);
}


/*
 * --------------------------------------------------------------------------
 * C23 stdio additions
 *
 * The 'b' conversion specifier prints unsigned integers in binary.
 * --------------------------------------------------------------------------
 */

static void
test_printf_binary(void)
{
    puts("  printf %b");

    char buf[64];

    int r = snprintf(buf, sizeof buf, "%b", 0x2au);

    assert(r >= 0);
    assert(strcmp(buf, "101010") == 0);

    r = snprintf(buf, sizeof buf, "%08b", 5u);

    assert(r == 8);
    assert(strcmp(buf, "00000101") == 0);

    /*
     * Width/precision/alternate form interactions are useful for finding
     * formatter bugs.
     */
    r = snprintf(buf, sizeof buf, "%#b", 10u);

    assert(r > 0);
}

/*
 * --------------------------------------------------------------------------
 * Integration torture
 *
 * Exercise several new facilities together rather than in isolation.
 * --------------------------------------------------------------------------
 */

static void
test_integration(void)
{
    puts("  integration");

    /*
     * A UTF-8 C23 string -> char8_t buffer -> checked size arithmetic ->
     * allocation -> copy -> explicit wipe.
     */
    static const char8_t text[] = u8"héllo, 世界 🌍";

    size_t length = 0;

    while (text[length] != u8'\0')
        ++length;

    size_t allocation_size;

    /*
     * Even though this particular addition cannot overflow on normal
     * machines, using ckd_add makes the allocation-size calculation
     * explicit and testable.
     */
    assert(!ckd_add(&allocation_size, length, (size_t)1));

    char8_t *copy = malloc(allocation_size);

    assert(copy != NULL);

    memcpy(copy, text, allocation_size);

    assert(copy[allocation_size - 1] == u8'\0');
    assert(memcmp(copy, text, allocation_size) == 0);

    /*
     * Verify that every code unit survived.
     */
    for (size_t i = 0; i < allocation_size; ++i)
        assert(copy[i] == text[i]);

    memset_explicit(copy, 0, allocation_size);

    for (size_t i = 0; i < allocation_size; ++i)
        assert(copy[i] == 0);

    free(copy);
}


/*
 * --------------------------------------------------------------------------
 * Main
 * --------------------------------------------------------------------------
 */

int
main(void)
{
    puts("C23 libc torture test");

    printf("  __STDC_VERSION__ = %ld\n", (long)__STDC_VERSION__);
    printf("  __STDC_VERSION_UCHAR_H__ = %ld\n",
           (long)__STDC_VERSION_UCHAR_H__);
    printf("  __STDC_VERSION_STDCKDINT_H__ = %ld\n",
           (long)__STDC_VERSION_STDCKDINT_H__);
    printf("  __STDC_VERSION_STDBIT_H__ = %ld\n",
           (long)__STDC_VERSION_STDBIT_H__);

    test_stdbit();
    test_stdckdint();
    test_strdup_family();
    test_memset_explicit();
    test_char8();
    test_timespec_getres();
    test_printf_binary();
    test_integration();

    puts("PASS");
    return 0;
}
