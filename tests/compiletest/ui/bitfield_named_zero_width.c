//@ mode: c
//@ compile-fail

struct Bad {
    unsigned int a : 0;
//               ^^^^^ error: named zero-width bitfields are invalid
};

int main(void) {
    return 0;
}
