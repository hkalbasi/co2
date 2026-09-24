//@ mode: c
//@ run-status: 0
//@ compile-flags: -madx

// ADX carry/borrow intrinsics from <x86intrin.h>.
// Needed by vpp (src/vppinfra/clib.h: u64_add_with_carry, u64_sub_with_borrow),
// which currently fails to compile on every translation unit including clib.h.

#include <assert.h>
#include <stdint.h>
#include <x86intrin.h>

static uint64_t add_carry(uint64_t *carry, uint64_t a, uint64_t b) {
    unsigned long long v;
    *carry = _addcarry_u64((unsigned char)*carry, a, b, &v);
    return (uint64_t)v;
}

static uint64_t sub_borrow(uint64_t *borrow, uint64_t x, uint64_t y) {
    unsigned long long v;
    *borrow = _subborrow_u64((unsigned char)*borrow, x, y, &v);
    return (uint64_t)v;
}

int main(void) {
    // 64-bit wrap produces carry-out.
    {
        uint64_t c = 0;
        uint64_t r = add_carry(&c, 0xffffffffffffffffull, 1);
        assert(r == 0 && c == 1);
    }
    // Carry-in propagates.
    {
        uint64_t c = 1;
        uint64_t r = add_carry(&c, 0xffffffffffffffffull, 0);
        assert(r == 0 && c == 1);
    }
    // No carry.
    {
        uint64_t c = 0;
        uint64_t r = add_carry(&c, 5, 7);
        assert(r == 12 && c == 0);
    }
    // 128-bit addition chained through carry: (2^64 - 1) + 1 == 2^64.
    {
        uint64_t c = 0;
        uint64_t lo = add_carry(&c, 0xffffffffffffffffull, 1);
        uint64_t hi = add_carry(&c, 0, 0);
        assert(lo == 0 && hi == 1 && c == 0);
    }
    // 32-bit variants.
    {
        unsigned int v = 0;
        unsigned char c = _addcarry_u32(0, 0xffffffffu, 1, &v);
        assert(v == 0 && c == 1);
    }
    {
        unsigned int v = 0;
        unsigned char c = _addcarry_u32(1, 0xffffffffu, 0, &v);
        assert(v == 0 && c == 1);
    }
    // Borrow: 0 - 1 borrows.
    {
        uint64_t b = 0;
        uint64_t r = sub_borrow(&b, 0, 1);
        assert(r == 0xffffffffffffffffull && b == 1);
    }
    // 128-bit subtraction chained through borrow: (1, 0) - (0, 1) == 2^64 - 1.
    {
        uint64_t b = 0;
        uint64_t lo = sub_borrow(&b, 0, 1);
        uint64_t hi = sub_borrow(&b, 1, 0);
        assert(lo == 0xffffffffffffffffull && hi == 0 && b == 0);
    }
    {
        unsigned int v = 0;
        unsigned char b = _subborrow_u32(0, 0, 1, &v);
        assert(v == 0xffffffffu && b == 1);
    }
    return 0;
}
