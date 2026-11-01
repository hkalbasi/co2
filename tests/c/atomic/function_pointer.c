//@ mode: c
//@ run-status: 0

#include <stdatomic.h>

typedef int (*binop_fn)(int, int);

static int add(int a, int b) {
    return a + b;
}

static int mul(int a, int b) {
    return a * b;
}

static int sub(int a, int b) {
    return a - b;
}

int main(void) {
    _Atomic(binop_fn) op = add;

    if (atomic_load_explicit(&op, memory_order_seq_cst)(2, 3) != 5) {
        return 1;
    }

    binop_fn old = atomic_exchange_explicit(&op, mul, memory_order_acq_rel);
    if (old != add || atomic_load(&op)(2, 3) != 6) {
        return 2;
    }

    binop_fn expected = add;
    if (atomic_compare_exchange_strong_explicit(&op, &expected, sub,
                                                memory_order_acq_rel,
                                                memory_order_acquire)) {
        return 3;
    }
    if (expected != mul) {
        return 4;
    }

    expected = mul;
    if (!atomic_compare_exchange_strong(&op, &expected, sub)) {
        return 5;
    }
    if (atomic_load_explicit(&op, memory_order_acquire)(5, 2) != 3) {
        return 6;
    }

    atomic_store_explicit(&op, add, memory_order_release);
    return atomic_load(&op)(4, 4) == 8 ? 0 : 7;
}
