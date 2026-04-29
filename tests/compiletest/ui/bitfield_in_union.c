//@ mode: c
//@ compile-fail

union Bad {
    unsigned int a : 1;
//               ^^^^^ error: bitfields in unions are not supported yet
};

int main(void) {
    return 0;
}
