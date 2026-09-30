//@ mode: c
//@ compile-fail

// Same as empty.c, but with a C23 fixed underlying type.

enum F : int {};
           //^^ error: empty enum is invalid

int main(void) {
    return 0;
}
