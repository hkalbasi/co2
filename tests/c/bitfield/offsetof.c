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
