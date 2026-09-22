//@ mode: c
//@ run-status: 0
//@ compile-flags: -march=native
//@ run-stdout: FILE: saxpy_dot.out

#include <stdio.h>
#include <stdlib.h>
#include <immintrin.h>   // pulls in SSE/AVX/FMA intrinsics

// ---------------------------------------------------------------
// SAXPY: y[i] = a * x[i] + y[i]   (AVX2 + FMA, 8 floats per iter)
// ---------------------------------------------------------------
void saxpy_avx2(size_t n, float a, const float * restrict x, float * restrict y)
{
    const __m256 va = _mm256_set1_ps(a);   // broadcast a to all 8 lanes
    size_t i = 0;

    // Main vectorized loop: 8 floats at a time
    for (; i + 8 <= n; i += 8) {
        __m256 vx = _mm256_loadu_ps(x + i);       // unaligned load
        __m256 vy = _mm256_loadu_ps(y + i);
        // FMA: vy = vx * va + vy   (single rounding, one instruction)
        vy = _mm256_fmadd_ps(vx, va, vy);
        _mm256_storeu_ps(y + i, vy);
    }

    // Tail: scalar cleanup
    for (; i < n; ++i)
        y[i] = a * x[i] + y[i];
}

// ---------------------------------------------------------------
// Dot product: sum(x[i] * y[i])   (AVX2 + FMA)
// ---------------------------------------------------------------
float dot_avx2(size_t n, const float * restrict x, const float * restrict y)
{
    __m256 acc = _mm256_setzero_ps();   // 8 partial sums
    size_t i = 0;

    for (; i + 8 <= n; i += 8) {
        __m256 vx = _mm256_loadu_ps(x + i);
        __m256 vy = _mm256_loadu_ps(y + i);
        acc = _mm256_fmadd_ps(vx, vy, acc);
    }

    // Horizontal sum of the 8 lanes
    // Step 1: add upper 128 bits to lower 128 bits
    __m128 lo = _mm256_castps256_ps128(acc);
    __m128 hi = _mm256_extractf128_ps(acc, 1);
    __m128 s  = _mm_add_ps(lo, hi);

    // Step 2: hadd twice to reduce 4 -> 1
    s = _mm_hadd_ps(s, s);
    s = _mm_hadd_ps(s, s);
    float result = _mm_cvtss_f32(s);

    // Scalar tail
    for (; i < n; ++i)
        result += x[i] * y[i];

    return result;
}

// ---------------------------------------------------------------
// Driver
// ---------------------------------------------------------------
int main(void)
{
    const size_t N = 1000;   // deliberately not a multiple of 8
    float *x = aligned_alloc(32, N * sizeof(float));
    float *y = aligned_alloc(32, N * sizeof(float));

    for (size_t i = 0; i < N; ++i) {
        x[i] = (float)i;
        y[i] = 1.0f;
    }

    saxpy_avx2(N, 2.0f, x, y);

    printf("saxpy[0..4] = %.1f %.1f %.1f %.1f %.1f\n",
           y[0], y[1], y[2], y[3], y[4]);

    // Re-init y
    for (size_t i = 0; i < N; ++i) y[i] = 1.0f;

    printf("dot          = %.1f\n", dot_avx2(N, x, y));
    printf("expected dot = %.1f\n",
           (float)((N - 1) * N / 2));   // sum_{i=0}^{N-1} i*1

    free(x); free(y);
    return 0;
}
