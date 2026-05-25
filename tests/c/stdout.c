//@ mode: c
//@ run-status: 0
//@ run-stdout: ok 3\n

int printf(const char *, ...);

int main() {
    printf("ok %d\n", 3);
    return 0;
}
