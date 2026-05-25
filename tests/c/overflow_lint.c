//@ run-status: 0
//@ mode: c

#include <stdint.h>

unsigned int mask(unsigned int value) {
    return value - 1;
}

int foo(int argc) {
    unsigned int shift = argc & 31U;

    if (((1U << shift) - 1U) != mask(1U << shift)) {
        return 2;
    }

    return 0;
}

int main() {
    return (foo(2) || foo(3) || foo(5));
}
