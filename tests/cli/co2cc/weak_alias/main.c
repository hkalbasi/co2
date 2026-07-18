//@ run-status: 0

// References all symbols. `crypt_r` must resolve to `__crypt_r` via the weak
// alias, `weak_only` must resolve to the strong override (99), and the plain
// strong symbol must behave normally.
int crypt_r(int x);
int weak_only(void);
int strong_sym(void);

int main(void) {
    if (crypt_r(5) != 10) {
        return 1;
    }
    if (weak_only() != 99) {
        return 2;
    }
    if (strong_sym() != 10) {
        return 3;
    }
    return 0;
}
