//@ mode: c
//@ run-status: 0

#include <assert.h>
#include <stdatomic.h>

int func(void) {
    return 123;
}

struct S {
    int x;
};

enum E {
    A,
    B
};

int main(void) {
    // ------------------------------------------------------------
    // integer types
    // ------------------------------------------------------------

    auto a = 1;
    assert(_Generic(a, int: 1, default: 0));

    auto b = 1u;
    assert(_Generic(b, unsigned int: 1, default: 0));

    auto c = 1L;
    assert(_Generic(c, long: 1, default: 0));

    auto d = 1ULL;
    assert(_Generic(d, unsigned long long: 1, default: 0));

    // ------------------------------------------------------------
    // floating types
    // ------------------------------------------------------------

    auto e = 1.0;
    assert(_Generic(e, double: 1, default: 0));

    auto f = 1.0f;
    assert(_Generic(f, float: 1, default: 0));

    auto g = 1.0L;
    assert(_Generic(g, long double: 1, default: 0));

    // ------------------------------------------------------------
    // character literal
    // ------------------------------------------------------------

    auto ch = 'a';
    assert(_Generic(ch, int: 1, default: 0));

    // ------------------------------------------------------------
    // top-level qualifiers are discarded
    // ------------------------------------------------------------

    const int ci = 42;

    auto x = ci;

    assert(_Generic(x, int: 1, default: 0));

    // ------------------------------------------------------------
    // pointer deduction
    // ------------------------------------------------------------

    int n = 0;

    auto p = &n;

    assert(_Generic(p, int *: 1, default: 0));

    // ------------------------------------------------------------
    // pointer to const
    // ------------------------------------------------------------

    const int cn = 5;

    auto cp = &cn;

    // TODO: remove int* here.
    assert(_Generic(cp, const int *: 1, int*: 2, default: 0));

    // ------------------------------------------------------------
    // array decay
    // ------------------------------------------------------------

    int arr[10];

    auto arrp = arr;

    assert(_Generic(arrp, int *: 1, default: 0));

    // ------------------------------------------------------------
    // string literal decay
    // ------------------------------------------------------------

    auto s = "hello";

    assert(_Generic(s, char *: 1, default: 0));

    // ------------------------------------------------------------
    // compound literals
    // ------------------------------------------------------------

    auto cl = (struct S){1};

    assert(_Generic(cl, struct S: 1, default: 0));

    assert(cl.x == 1);

    auto clp = &(int){123};

    assert(_Generic(clp, int *: 1, default: 0));
    assert(*clp == 123);

    // ------------------------------------------------------------
    // enums
    // ------------------------------------------------------------

    auto en = A;

    assert(_Generic(en, int: 1, default: 0));

    // ------------------------------------------------------------
    // function designator decay
    // ------------------------------------------------------------

    auto fp1 = func;

    assert(
        _Generic(fp1, int (*)(void): 1, default: 0)
    );

    // ------------------------------------------------------------
    // explicit address of function
    // ------------------------------------------------------------

    auto fp2 = &func;

    assert(
        _Generic(fp2, int (*)(void): 1, default: 0)
    );

    // ------------------------------------------------------------
    // typedefs disappear
    // ------------------------------------------------------------

    typedef unsigned long ulong;

    ulong ul = 1;

    auto ul2 = ul;

    assert(
        _Generic(ul2, unsigned long: 1, default: 0)
    );

    // ------------------------------------------------------------
    // atomic qualification
    // ------------------------------------------------------------

    _Atomic int ai = 7;

    auto ai2 = ai;

    assert(
        _Generic(ai2, int: 1, default: 0)
    );

    // ------------------------------------------------------------
    // comma operator
    // ------------------------------------------------------------

    auto comma = (1, 2.0);

    assert(
        _Generic(comma, double: 1, default: 0)
    );

    // ------------------------------------------------------------
    // conditional operator
    // ------------------------------------------------------------

    auto cond = 1 ? 1 : 1L;

    assert(
        _Generic(cond, long: 1, default: 0)
    );

    // ------------------------------------------------------------
    // nested auto declarations
    // ------------------------------------------------------------

    auto v1 = 10;
    auto v2 = v1;

    assert(_Generic(v2, int: 1, default: 0));

    return 0;
}
