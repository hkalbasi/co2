//@ mode: c
//@ run-status: 0

/* C11 §6.4.4.1: for a hex literal with 'u'/'U' suffix the type is the
 * first of { unsigned int, unsigned long, unsigned long long } that fits.
 * 0xffffffffffffffffu exceeds UINT_MAX so its type must be at least
 * unsigned long (64-bit on LP64), NOT unsigned int.
 *
 * Repro for the co2cc bug where IntegerSuffix::Unsigned always produced
 * UintTy::U32 regardless of the value. */

#include <stdint.h>

#define trim64(x) ((x) & 0xffffffffffffffffu)

int main(void) {
    /* 0xffffffffffffffffu must equal the 64-bit max, not the 32-bit max. */
    uint64_t lit = 0xffffffffffffffffu;
    if (lit != UINT64_C(0xffffffffffffffff))
        return 1;

    /* trim64 must leave bits above bit 31 intact. */
    uint64_t high = UINT64_C(0xffffffff00000000);
    if (trim64(high) != UINT64_C(0xffffffff00000000))
        return 2;

    return 0;
}
