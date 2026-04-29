//@ mode: c
//@ compile-fail

struct Bits {
    unsigned int a : 3;
    unsigned int b : 5;
};

int main(void) {
    struct Bits bits = {1, 2};
    unsigned int *ptr = &bits.a;
//                      ^^^^^^^ error: cannot take address of non-place expression
    return ptr != 0;
}
