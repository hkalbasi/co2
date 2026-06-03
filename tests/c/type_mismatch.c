//@ mode: c
//@ compile-fail

struct A {
    int x;
};

struct B {
    int x;
};

union U {
    int x;
};

void returns_void(void) {
}

void takes_b(struct B b) {
}

void assign_mismatch(void) {
    struct A a = { 1 };
    struct B b = { 2 };
    b = a;
//  ^^^^^ error: assignment type mismatch: expected co2(struct B), got co2(struct A)
}

void struct_pointer_assignment_mismatch(void) {
    struct A a = { 1 };
    int *p = 0;
    p = a;
//  ^^^^^ error: assignment type mismatch: expected *mut i32, got co2(struct A)
}

void struct_union_assignment_mismatch(void) {
    struct A a = { 1 };
    union U u = { 2 };
    u = a;
//  ^^^^^ error: assignment type mismatch: expected co2(union U), got co2(struct A)
}

void initializer_mismatch(void) {
    struct A a = { 1 };
    struct B b = a;
//               ^ error: initializer type mismatch: expected co2(struct B), got co2(struct A)
}

void aggregate_initializer_mismatch(void) {
    struct A a = { 1 };
    int xs[1] = { a };
//                ^ error: initializer type mismatch: expected i32, got co2(struct A)
}

void void_initializer_mismatch(void) {
    int x = returns_void();
//          ^^^^^^^^^^^^^^ error: initializer type mismatch: expected i32, got ()
}

void call_mismatch(void) {
    struct A a = { 1 };
    takes_b(a);
//  ^^^^^^^^^^ error: call argument type mismatch at index 0: expected co2(struct B), got co2(struct A)
}

int return_mismatch(void) {
    struct A a = { 1 };
    return a;
//         ^ error: return type mismatch: expected i32, got co2(struct A)
}

void ternary_mismatch(int cond) {
    struct A a = { 1 };
    struct A *p = 0;
    (void)(cond ? a : p);
//        ^^^^^^^^^^^^^^ error: ternary operator branches have mismatched types: expected co2(struct A), got *mut co2(struct A)
}

void binary_mismatch(void) {
    struct A a = { 1 };
    (void)(a + 1);
//        ^^^^^^^ error: binary op type mismatch: expected co2(struct A), got i32
}

void anonymous_assignment_mismatch(void) {
    struct { int x; } a = { 1 };
    struct { int x; } b = { 2 };
    b = a;
//  ^^^^^ error: assignment type mismatch: expected co2(struct #4), got co2(struct #3)
}

typedef struct { int x; } Foo;

void anonymous_assignment_mismatch2(void) {
    struct { int x; } a = { 1 };
    Foo b = { 2 };
    b = a;
//  ^^^^^ error: assignment type mismatch: expected Foo, got co2(struct #6)
}

int (*array_pointer_return_mismatch(void))[10] {
    int *p = 0;
    return p;
//         ^ error: return type mismatch: expected *mut [i32; 10], got *mut i32
}

int main(void) {
    return 0;
}
