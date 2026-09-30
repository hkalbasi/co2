//@ mode: c
//@ compile-fail

// An enum without enumerators is invalid (gcc: `empty enum is invalid`).

enum E {};
    // ^^ error: empty enum is invalid

int main(void) {
    return 0;
}
