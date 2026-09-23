//@ mode: c
//@ run-status: 0

#include <stdatomic.h>

typedef unsigned int uint32_t;

int main(void) {
    uint32_t value = 1;
    uint32_t expected = 6;
    uint32_t old_add = atomic_fetch_add_explicit((_Atomic uint32_t *)&value, 2, 5);
    uint32_t old_or = atomic_fetch_or((_Atomic uint32_t *)&value, 4);
    uint32_t old_and = atomic_fetch_and_explicit((_Atomic uint32_t *)&value, 6, 5);
    uint32_t old_xor = atomic_fetch_xor_explicit((_Atomic uint32_t *)&value, 3, 5);
    uint32_t old_sub = atomic_fetch_sub_explicit((_Atomic uint32_t *)&value, 1, 5);
    uint32_t loaded = atomic_load_explicit((_Atomic uint32_t *)&value, 5);
    int exchanged =
        atomic_compare_exchange_strong_explicit((_Atomic uint32_t *)&value, &expected, 9, 5, 5);
    atomic_store_explicit((_Atomic uint32_t *)&value, 11, 5);

    if (old_add != 1 || old_or != 3 || old_and != 7 || old_xor != 6 || old_sub != 5) {
        return 1;
    }
    if (loaded != 4 || exchanged || value != 11 || expected != 4) {
        return 2;
    }
    return 0;
}
