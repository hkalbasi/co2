//@ mode: c
//@ compile-fail

struct Bad {
    float a : 1;
//        ^^^^^ error: bitfield type must be an integer or boolean type
};

int main(void) {
    return 0;
}
