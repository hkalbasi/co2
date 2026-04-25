//@ mode: c
//@ run-status: 0

#include <stddef.h>

typedef int Callback(int);

static int invoke(Callback cb, int value) {
    return cb(value);
}

static int plus_one(int value) {
    return value + 1;
}

int main(void) {
    return invoke(plus_one, 41) != 42;
}
