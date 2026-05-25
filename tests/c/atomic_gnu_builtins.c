//@ mode: c
//@ run-status: 0

typedef unsigned int uint32_t;

int main(void) {
    uint32_t value = 1;
    uint32_t expected = 6;
    uint32_t old_add = __atomic_fetch_add((_Atomic uint32_t *)&value, 2, 5);
    uint32_t old_or = __atomic_fetch_or((_Atomic uint32_t *)&value, 4, 5);
    uint32_t old_and = __atomic_fetch_and((_Atomic uint32_t *)&value, 6, 5);
    uint32_t old_xor = __atomic_fetch_xor((_Atomic uint32_t *)&value, 3, 5);
    uint32_t old_sub = __atomic_fetch_sub((_Atomic uint32_t *)&value, 1, 5);
    uint32_t loaded = __atomic_load_n((_Atomic uint32_t *)&value, 5);
    int exchanged =
        __atomic_compare_exchange_n((_Atomic uint32_t *)&value, &expected, 9, 0, 5, 5);
    __atomic_store_n((_Atomic uint32_t *)&value, 11, 5);

    if (old_add != 1 || old_or != 3 || old_and != 7 || old_xor != 6 || old_sub != 5) {
        return 1;
    }
    if (loaded != 4 || exchanged || value != 11 || expected != 4) {
        return 2;
    }
    return 0;
}
