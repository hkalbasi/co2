//@ mode: c
//@ compile-fail

struct Bits {
    unsigned int a : 3;
    unsigned int b : 5;
    unsigned int tail;
};

int main(void) {
    return __builtin_offsetof(struct Bits, a);
//                                         ^ error: offsetof: field 'a' is a bitfield
}

struct Inner {
    unsigned int x : 3;
    unsigned int y;
};

struct Outer {
    int head;
    unsigned int foo : 3;
    struct Inner bar[2];
    struct Inner baz;
};

int nested1(void) {
    return __builtin_offsetof(struct Outer, bar[1].x);
//                                                 ^ error: offsetof: field 'x' is a bitfield
}

int nested2(void) {
    return __builtin_offsetof(struct Outer, baz.x);
//                                              ^ error: offsetof: field 'x' is a bitfield
}

int nested3(void) {
    return __builtin_offsetof(struct Outer, baz.y);
    return __builtin_offsetof(struct Outer, foo);
//                                          ^^^ error: offsetof: field 'foo' is a bitfield
}
