//@ mode: c
//@ compile-fail

#include <stdatomic.h>

void bad_load_order() {
    _Atomic int x = 0;
    __atomic_load_n(&x, 6);
  //^^^^^^^^^^^^^^^^^^^^^^ error: `__atomic_load_n` cannot load with memory order 6 (use 0, 1, 2 or 5)
}

void bad_store_order() {
    _Atomic int x = 0;
    __atomic_store_n(&x, 1, 99);
  //^^^^^^^^^^^^^^^^^^^^^^^^^^^ error: `__atomic_store_n` cannot store with memory order 99 (use 0, 3 or 5)
}

void bad_store_acquire() {
    _Atomic int x = 0;
    __atomic_store_n(&x, 1, 2);
  //^^^^^^^^^^^^^^^^^^^^^^^^^^ error: `__atomic_store_n` cannot store with memory order 2 (use 0, 3 or 5)
}

void bad_rmw_order() {
    _Atomic int x = 0;
    __atomic_fetch_add(&x, 1, 7);
  //^^^^^^^^^^^^^^^^^^^^^^^^^^^^ error: `__atomic_fetch_add` expects a memory order 0-5, got 7
}

void bad_cas_failure_order() {
    _Atomic int x = 0;
    int e = 0;
    __atomic_compare_exchange_n(&x, &e, 1, 0, 5, 3);
  //^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ error: `__atomic_compare_exchange_n` cannot fail with memory order 3 (use 0, 1, 2 or 5)
}

void bad_fence_order() {
    __atomic_thread_fence(6);
  //^^^^^^^^^^^^^^^^^^^^^^^^ error: `__atomic_thread_fence` expects a memory order 0-5, got 6
}
