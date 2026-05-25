//@ mode: c
//@ run-status: 0

// Test __builtin_clz
int test_clz() {
    // 0x80000000 has 0 leading zeros (highest bit set)
    if (__builtin_clz(0x80000000) != 0) return 1;
    // 0x40000000 has 1 leading zero
    if (__builtin_clz(0x40000000) != 1) return 2;
    // 0x00000001 has 31 leading zeros
    if (__builtin_clz(1) != 31) return 3;
    // 0x00000000 - behavior is undefined in GCC, but let's test what Rust does (returns 32)
    // Skip 0 for now as it's undefined behavior
    return 0;
}

// Test __builtin_clzll
int test_clzll() {
    // 0x8000000000000000LL has 0 leading zeros
    if (__builtin_clzll(0x8000000000000000LL) != 0) return 1;
    // 0x4000000000000000LL has 1 leading zero
    if (__builtin_clzll(0x4000000000000000LL) != 1) return 2;
    // 1 has 63 leading zeros
    if (__builtin_clzll(1LL) != 63) return 3;
    return 0;
}

// Test __builtin_ctz
int test_ctz() {
    // 1 has 0 trailing zeros
    if (__builtin_ctz(1) != 0) return 1;
    // 2 has 1 trailing zero
    if (__builtin_ctz(2) != 1) return 2;
    // 0x80000000 has 31 trailing zeros
    if (__builtin_ctz(0x80000000) != 31) return 3;
    return 0;
}

// Test __builtin_ctzll
int test_ctzll() {
    // 1 has 0 trailing zeros
    if (__builtin_ctzll(1LL) != 0) return 1;
    // 2 has 1 trailing zero
    if (__builtin_ctzll(2LL) != 1) return 2;
    // 0x8000000000000000LL has 63 trailing zeros
    if (__builtin_ctzll(0x8000000000000000LL) != 63) return 3;
    return 0;
}

int main() {
    if (test_clz()) return 1;
    if (test_clzll()) return 2;
    if (test_ctz()) return 3;
    if (test_ctzll()) return 4;
    return 0;
}
