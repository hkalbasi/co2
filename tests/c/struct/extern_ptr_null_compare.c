//@ mode: c
//@ run-status: 0

// Regression test: comparing a pointer to an incomplete struct type against
// NULL/(void *)0 is valid C and must not be diagnosed as operator overloading
// (quickjs run-test262.c / quickjs-libc.c use `rt == NULL`).

#include <stddef.h>

struct runtime;

static struct runtime *new_runtime(void);

int main(void) {
    struct runtime *rt = new_runtime();
    if (rt == NULL) {
        return 0;
    }
    return 1;
}

static struct runtime *new_runtime(void) {
    return NULL;
}
