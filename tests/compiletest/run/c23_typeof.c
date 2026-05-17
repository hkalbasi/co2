//@ mode: c
//@ run-status: 0

#include <stddef.h>
#include <assert.h>

int sidefx_counter = 0;

int sidefx(void) {
    sidefx_counter++;
    return 1;
}

int func(double x) {
    return (int)x;
}

typedef int arr4_t[4];

int main(void) {
    // ------------------------------------------------------------
    // typeof(type)
    // ------------------------------------------------------------

    typeof(int) a = 1;
    assert(_Generic(a, int: 1, default: 0));

    typeof(const int) b = 2;
    assert(_Generic(b, int: 1, default: 0));

    typeof(unsigned long long) c = 3;
    assert(_Generic(c, unsigned long long: 1, default: 0));

    // ------------------------------------------------------------
    // typeof(expr)
    // ------------------------------------------------------------

    int x = 0;

    typeof(x) y = 1;
    assert(_Generic(y, int: 1, default: 0));

    const int cx = 2;

    typeof(cx) cy = 3;
    assert(_Generic(cy, int: 1, default: 0));

    // ------------------------------------------------------------
    // pointers
    // ------------------------------------------------------------

    int *p = &x;

    typeof(p) q = &y;
    assert(_Generic(q, int *: 1, default: 0));

    const int *cp = &cx;

    typeof(cp) cq = &cx;
    assert(_Generic(cq, const int *: 1, default: 0));

    // ------------------------------------------------------------
    // arrays
    // ------------------------------------------------------------

    int arr[10];

    typeof(arr) arr2 = {0};

    assert(sizeof(arr2) == sizeof(int[10]));

    arr4_t a4 = {0};

    typeof(a4) a4b = {0};

    assert(sizeof(a4b) == sizeof(int[4]));

    // ------------------------------------------------------------
    // function types
    // ------------------------------------------------------------

    typeof(func) *fp = func;

    assert(
        _Generic(fp, int (*)(double): 1, default: 0)
    );

    typeof(func(1.0)) r = 0;

    assert(_Generic(r, int: 1, default: 0));

    // ------------------------------------------------------------
    // nested typeof
    // ------------------------------------------------------------

    typeof(typeof(int)) nested = 1;

    assert(_Generic(nested, int: 1, default: 0));

    // ------------------------------------------------------------
    // anonymous struct
    // ------------------------------------------------------------

    typeof(struct {
        int a;
        long b;
    }) anon = {1, 2};

    assert(sizeof(anon) >= sizeof(int) + sizeof(long));

    // ------------------------------------------------------------
    // side effects must not execute
    // ------------------------------------------------------------

    sidefx_counter = 0;

    typeof(sidefx()) noeval = 0;

    assert(_Generic(noeval, int: 1, default: 0));

    if (sidefx_counter != 0)
        return 1;

    // ------------------------------------------------------------
    // comma operator
    // ------------------------------------------------------------

    typeof((x, 1.5)) d = 0;

    assert(_Generic(d, double: 1, default: 0));

    // ------------------------------------------------------------
    // conditional operator
    // ------------------------------------------------------------

    typeof(1 ? x : 1L) cond = 0;

    assert(_Generic(cond, long: 1, default: 0));

    // ------------------------------------------------------------
    // compound literal
    // ------------------------------------------------------------

    typeof((int){123}) cl = 0;

    assert(_Generic(cl, int: 1, default: 0));

    // ------------------------------------------------------------
    // declarator interaction
    // ------------------------------------------------------------

    typeof(int) *ptr = &x;

    assert(_Generic(ptr, int *: 1, default: 0));

    return 0;
}
