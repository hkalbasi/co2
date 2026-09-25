//@ mode: c
//@ run-status: 0

// Every integer constant expression with value 0 (or such an expression
// cast to `void *`) is a null pointer constant and must be accepted as a
// `constexpr` pointer initializer.

enum { ZERO = 0 };

int main(void) {
    constexpr int *p0 = 0;
    constexpr int *p1 = (void *)0;
    constexpr int *p2 = (void *)'\0';
    constexpr int *p3 = (void *)(1 - 1);
    constexpr int *p4 = (void *)ZERO;
    constexpr int *p5 = (void *)+0;

    if (p0 != 0) {
        return 1;
    }
    if (p1 != 0) {
        return 2;
    }
    if (p2 != 0) {
        return 3;
    }
    if (p3 != 0) {
        return 4;
    }
    if (p4 != 0) {
        return 5;
    }
    if (p5 != 0) {
        return 6;
    }
    return 0;
}
