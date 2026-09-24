//@ mode: c
//@ run-status: 0
//@ compile-flags: -mavx2

#include <assert.h>
#include <stdint.h>
#include <x86intrin.h>

int main(void) {
    int32_t out[8];

    _mm256_storeu_si256((__m256i *)out, _mm256_set1_epi32(3));
    assert(out[0] == 3 && out[1] == 3 && out[2] == 3 && out[3] == 3 && out[4] == 3 &&
           out[5] == 3 && out[6] == 3 && out[7] == 3);

    int32_t a[8] = {1, 2, 3, 4, 5, 6, 7, 8};
    int32_t b[8] = {10, 20, 30, 40, 50, 60, 70, 80};
    __m256i va = _mm256_loadu_si256((const __m256i *)a);
    __m256i vb = _mm256_loadu_si256((const __m256i *)b);

    _mm256_storeu_si256((__m256i *)out, _mm256_add_epi32(va, vb));
    assert(out[0] == 11 && out[1] == 22 && out[2] == 33 && out[3] == 44 && out[4] == 55 &&
           out[5] == 66 && out[6] == 77 && out[7] == 88);

    _mm256_storeu_si256((__m256i *)out, _mm256_sub_epi32(vb, va));
    assert(out[0] == 9 && out[1] == 18 && out[2] == 27 && out[3] == 36 && out[4] == 45 &&
           out[5] == 54 && out[6] == 63 && out[7] == 72);

    return 0;
}
