//@ mode: c
//@ run-status: 0

#include <assert.h>

void foo(int a, int* b) {
    *b += a;
}

void foo_checker(void (*p)(int, int*)) {
    int s = 2;
    p(4, &s);
    assert(s == 6);
}

int bar(int a, int* b) {
    return *b += a;
}

void bar_checker(int (*p)(int, int*)) {
    int s = 2;
    assert(p(4, &s) == 6);
    assert(s == 6);
}

int main() {
    int s = 5;
    foo(50, &s);

    void (*p1)(int, int*) = foo;
    p1(500, &s);

    void (*p2)(const int, int*) = foo;
    p2(5000, &s);

    foo_checker(foo);   
    foo_checker((void*)foo);   
    foo_checker(p1);   
    foo_checker(p2);
    foo_checker((void (*)())foo); // Removed in C23
    bar_checker(bar);
    bar_checker((int (*)())bar); // Removed in C23

    assert(s == 5555);

    return 0;
}
