//@ mode: c
//@ run-status: 0

#include <emmintrin.h>

// zstd's lazy match finder broadcasts a tag byte with _mm_set1_epi8.
int main(void) {
    __m128i v = _mm_set1_epi8((char)7);
    int mask = _mm_movemask_epi8(_mm_cmpeq_epi8(v, _mm_set1_epi8((char)7)));
    return mask == 0xFFFF ? 0 : 1;
}
