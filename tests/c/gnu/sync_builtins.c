//@ mode: c
//@ run-status: 0

// Legacy __sync_* atomic builtins.

typedef unsigned int uint32_t;

int main(void) {
    uint32_t value = 10;
    if (__sync_fetch_and_add(&value, 5) != 10 || value != 15) {
        return 1;
    }
    if (__sync_fetch_and_sub(&value, 3) != 15 || value != 12) {
        return 2;
    }
    if (__sync_fetch_and_or(&value, 3) != 12 || value != 15) {
        return 3;
    }
    if (__sync_fetch_and_and(&value, 6) != 15 || value != 6) {
        return 4;
    }
    if (__sync_fetch_and_xor(&value, 3) != 6 || value != 5) {
        return 5;
    }
    if (__sync_add_and_fetch(&value, 5) != 10 || value != 10) {
        return 6;
    }
    if (__sync_sub_and_fetch(&value, 4) != 6 || value != 6) {
        return 7;
    }
    if (!__sync_bool_compare_and_swap(&value, 6, 42) || value != 42) {
        return 8;
    }
    if (__sync_bool_compare_and_swap(&value, 6, 7) || value != 42) {
        return 9;
    }
    if (__sync_val_compare_and_swap(&value, 42, 1) != 42 || value != 1) {
        return 10;
    }
    uint32_t nand = 12;
    if (__sync_fetch_and_nand(&nand, 10) != 12 || nand != (uint32_t)~8u) {
        return 13;
    }
    nand = 12;
    if (__sync_nand_and_fetch(&nand, 10) != (uint32_t)~8u || nand != (uint32_t)~8u) {
        return 14;
    }
    nand = 12;
    if (__sync_or_and_fetch(&nand, 10) != 14 || nand != 14) {
        return 15;
    }
    if (__sync_and_and_fetch(&nand, 12) != 12 || nand != 12) {
        return 16;
    }
    if (__sync_xor_and_fetch(&nand, 10) != 6 || nand != 6) {
        return 17;
    }
    __sync_synchronize();
    int lock = 0;
    if (__sync_lock_test_and_set(&lock, 1) != 0 || lock != 1) {
        return 11;
    }
    __sync_lock_release(&lock);
    if (lock != 0) {
        return 12;
    }
    return 0;
}
