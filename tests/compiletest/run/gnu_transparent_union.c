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
} transparent_u __attribute__((__transparent_union__));

void func(transparent_u u) {
    (void)u;
}

int main() {
    struct A a = {0};
    func(&a);
    return 0;
}
