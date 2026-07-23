//@ mode: c
//@ run-status: 0

// The propose of this test is to assert that we can compile C code that has UB,
// and rustc checks won't interfere with us, and at most give some warning.
// While we can't assert that the return code is always 0,
// I'm interested in detecting the change in the behavior when I update the rustc version.

int main0() {
    return 0;
}

int main1()
{
    int x;
    int y = x; // Reading x is UB
    struct { int a, b; } s;
    s.a = x;
    x = s.b;
    return 0;
}

int main2()
{
    int i, *q;
    void *p;

    i = i ? 0 : 0l; // Reading i is UB
    p = i ? (void *) 0 : 0;
    p = i ? 0 : (void *) 0;
    p = i ? 0 : (const void *) 0;
    q = i ? 0 : p;
    q = i ? p : 0;
    q = i ? q : 0;
    q = i ? 0 : q;

    return (int) 0;
}

int main3_helper(int x) {
    return x + 1;
}

int main3() {
    typedef int (*fn1)(int);
    typedef int (*fn2)(unsigned);
    fn1 a = main3_helper;
    fn2 b = (fn2)a;
    // b(1) is UB since it calls a fn(i32) -> i32 with a u32
    return b(1) != main3_helper(1);
}

int main4() {
    int v = 5;
    int *a = &v;
    int *b = &v;
    int *c = 0;

    if (!(a && b)) {
        return 1;
    }

    if (a && c) {
        return 2;
    }

    if (!(a || c)) {
        return 3;
    }

    if (!(a && !c)) {
        return 4;
    }

    int (*f)(int) = main3_helper;
    int (*g)(int) = 0;

    if (!main3_helper) {
        return 5;
    }
    if (!(f && main3_helper)) {
        return 6;
    }
    if (g && f) {
        return 7;
    }
    // While not technically UB, comparing function pointers can yield unexpected result.
    if (main3_helper != f) {
        return 8;
    }
    if (main3_helper == g) {
        return 9;
    }
    return 0;
}

int not_return(int x) {
    x += 3;
}

int main5() {
    not_return(2); // Calling this function is UB
    not_return(5);
    return 0;
}

typedef int (*main_ty)();

int main() {
    main_ty mains[] = {
        main0,
        main1, main2, main3, main4, main5,
    };
    
    int i;
    for (i = 0; i < sizeof(mains) / sizeof(mains[0]); i += 1) {
        if (mains[i]()) {
            return i;
        }
    }
    return 0;
}
