//@ mode: c
//@ run-status: 0
//@ compile-flags: -msse2

#include <assert.h>
#include <stdint.h>
#include <x86intrin.h>

int main(void) {
    int32_t out[4];

    _mm_storeu_si128((__m128i *)out, _mm_setzero_si128());
    assert(out[0] == 0 && out[1] == 0 && out[2] == 0 && out[3] == 0);

    _mm_storeu_si128((__m128i *)out, _mm_set1_epi32(7));
    assert(out[0] == 7 && out[1] == 7 && out[2] == 7 && out[3] == 7);

    int32_t a[4] = {1, 2, 3, 4};
    int32_t b[4] = {10, 20, 30, 40};
    __m128i va = _mm_loadu_si128((const __m128i *)a);
    __m128i vb = _mm_loadu_si128((const __m128i *)b);

    _mm_storeu_si128((__m128i *)out, _mm_add_epi32(va, vb));
    assert(out[0] == 11 && out[1] == 22 && out[2] == 33 && out[3] == 44);

    _mm_storeu_si128((__m128i *)out, _mm_sub_epi32(vb, va));
    assert(out[0] == 9 && out[1] == 18 && out[2] == 27 && out[3] == 36);

    _mm_storeu_si128((__m128i *)out, _mm_set_epi32(4, 3, 2, 1));
    assert(out[0] == 1 && out[1] == 2 && out[2] == 3 && out[3] == 4);

    assert(_mm_movemask_epi8(_mm_cmpeq_epi32(va, vb)) == 0);
    assert(_mm_movemask_epi8(_mm_cmpeq_epi32(va, va)) == 0xffff);

    __m64 m = (__m64)0LL;
    (void)m;

    return 0;
}
