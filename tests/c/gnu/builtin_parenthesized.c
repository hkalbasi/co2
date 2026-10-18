//@ mode: c
//@ run-status: 0

// Builtins are syntax, not functions: a parenthesised callee is the same call.

#include <alloca.h>
#include <assert.h>
#include <complex.h>
#include <stddef.h>

typedef unsigned int uint32_t;

int main(void) {
    unsigned char *p = (__builtin_alloca)(16);
    assert(p != NULL);
    p[0] = 'x';
    assert(p[0] == 'x');

    // More than one level of parentheses.
    unsigned char *q = ((__builtin_alloca))(8);
    q[0] = 'y';
    assert(q[0] == 'y');

    int n = 3;
    assert((__builtin_constant_p)(40 + 2));
    assert(!(__builtin_constant_p)(n));

    assert((__builtin_types_compatible_p)(int, int));
    assert(!(__builtin_types_compatible_p)(int, double));

    double _Complex z = (__builtin_complex)(1.0, 2.0);
    assert(creal(z) == 1.0 && cimag(z) == 2.0);

    uint32_t value = 1;
    uint32_t loaded = (__atomic_load_n)((_Atomic uint32_t *)&value, 5);
    assert(loaded == 1);
    uint32_t old = (__sync_fetch_and_add)((_Atomic uint32_t *)&value, 2);
    assert(old == 1 && value == 3);
    return 0;
}
