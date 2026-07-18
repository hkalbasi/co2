//@ run-status: 0

// Defines the strong target for the alias and a couple of plain symbols.
int __crypt_r(int x) {
    return x * 2;
}

int strong_sym(void) {
    return 10;
}
