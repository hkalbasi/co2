//@ mode: c
//@ compile-fail

#include <alloca.h>
#include <stddef.h>

void takes_fn(void) {}

int bare(void) {
    void *(*p)(size_t) = __builtin_alloca;
                       //^^^^^^^^^^^^^^^^ error: builtin function `__builtin_alloca` must be called
    (void)p;
    return 0;
}

int address_of(void) {
    int (*p)(int) = &__builtin_constant_p;
                   //^^^^^^^^^^^^^^^^^^^^ error: builtin function `__builtin_constant_p` must be called
    (void)p;
    return 0;
}

int as_argument(void) {
    takes_fn(__atomic_load_n);
           //^^^^^^^^^^^^^^^ error: builtin function `__atomic_load_n` must be called
    return 0;
}
