//@ mode: c

struct A {
    int a;
};

struct B {
    int b;
};

typedef union {
    struct A *a;
    struct B *b;
} transparent_u1 __attribute__((__transparent_union__));

void func1(transparent_u1 u) {
    (void)u;
}

typedef union {
    const struct A *a;
} transparent_u2 __attribute__((__transparent_union__));

void func2(transparent_u2 u) {
    (void)u;
}

typedef union {
    struct B *a;
    int b;
} transparent_u3 __attribute__((__transparent_union__));

void func3(transparent_u3 u) {
    (void)u;
}

int main() {
    struct A a = {0};
    func1(&a);
    func2(&a);
    func2((void*)&a);
    func3(5);
    return 0;
}
