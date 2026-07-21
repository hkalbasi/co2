//@ mode: c
//@ compile-fail

#include <assert.h>

void func1(int a, int* b) {
    *b += a;
}

void receiver1(void (*ptr)(int, int*)) {}

void f1() {
    int (*ptr)(int*, int, double) = func1;
                                  //^^^^^ error: initializer type mismatch: expected MaybeUninit<fn(*mut i32, i32, f64) -> i32>, got func1
}

void f2() {
    receiver1(func1);
    receiver1((void (*)(int))func1);
            //^^^^^^^^^^^^^^^^^^^^ error: call `receiver1` type mismatch at arg 0: expected MaybeUninit<fn(i32, *mut i32) -> ()>, got MaybeUninit<fn(i32) -> ()>
}

int main() {
}
