#include <immintrin.h>
#include <stdio.h>

int main(void)
{
    float x[8] = {1.0f, 2.0f, 3.0f, 4.0f, 5.0f, 6.0f, 7.0f, 8.0f};
    float y[8] = {1.0f, 1.0f, 1.0f, 1.0f, 1.0f, 1.0f, 1.0f, 1.0f};

    __m256 vx = _mm256_loadu_ps(x);
    __m256 vy = _mm256_loadu_ps(y);
    __m256 va = _mm256_set1_ps(2.0f);
    // FMA: y = x * 2 + y
    vy = _mm256_fmadd_ps(vx, va, vy);
    _mm256_storeu_ps(y, vy);

    for (int i = 0; i < 8; ++i) {
        float expect = 2.0f * x[i] + 1.0f;
        if (y[i] != expect) {
            printf("mismatch at %d: got %f expected %f\n", i, y[i], expect);
            return 1;
        }
    }

    printf("simd ok\n");
    return 0;
}
