// SIMD predefined-macro check for -m flags (see main.nu).
//
// Usage: must FAIL to compile without arch flags, must PASS with
// `co2cc -mavx2 -mfma` (and with `-march=native` on an AVX2 host),
// mirroring simd.c. Each #error names the missing predefined macro so a
// wrong -m -> macro mapping shows up as a compile error, not silent
// miscompilation.
#ifndef __SSE4_2__
#error "__SSE4_2__ not defined (expected with -mavx2 -mfma / -march=native)"
#endif
#ifndef __AVX__
#error "__AVX__ not defined (expected with -mavx2 -mfma / -march=native)"
#endif
#ifndef __AVX2__
#error "__AVX2__ not defined (expected with -mavx2 / -march=native)"
#endif
#ifndef __FMA__
#error "__FMA__ not defined (expected with -mfma / -march=native)"
#endif

int main(void)
{
    return 0;
}
