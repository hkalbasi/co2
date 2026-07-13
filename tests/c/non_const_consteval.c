//@ mode: c
//@ compile-fail

int non_const_fn(void) {
    return 5;
}

int f1() {
    static int a = non_const_fn();
                 //^^^^^^^^^^^^^^ error: cannot call non-const function
    return a;
}

int f2() {
    int a[] = { [non_const_fn()] = 5 };
               //^^^^^^^^^^^^^^ error: cannot call non-const function
}

int f3() {
    switch (2) {
        case 3:
        case non_const_fn():
           //^^^^^^^^^^^^^^ error: cannot call non-const function
            return non_const_fn();
    }
}

int f4() {
    int a[non_const_fn()];
        //^^^^^^^^^^^^^^ error: cannot call non-const function
}
