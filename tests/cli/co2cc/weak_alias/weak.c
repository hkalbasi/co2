//@ run-status: 0

// `crypt_r` is a weak alias of the strong `__crypt_r` defined in target.c.
// `weak_only` is a weak symbol: it is defined here weakly, but can be
// overridden by a strong definition provided in another translation unit.
int __crypt_r(int x);

int crypt_r(int x) __attribute__((__weak__, __alias__("__crypt_r")));

int weak_only(void) __attribute__((__weak__)) {
    return 42;
}
