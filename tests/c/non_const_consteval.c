//@ mode: c
//@ compile-fail

int non_const_fn(void) {
    return 5;
}

int fn_static() {
    static int a = non_const_fn();
                 //^^^^^^^^^^^^^^ error: cannot call non-const function
    return a;
}

int fn_designator() {
    int a[] = { [non_const_fn()] = 5 };
               //^^^^^^^^^^^^^^ error: cannot call non-const function
}

int fn_case() {
    switch (2) {
        case 3:
        case non_const_fn():
           //^^^^^^^^^^^^^^ error: cannot call non-const function
            return non_const_fn();
    }
}

int fn_array_size() {
    int a[non_const_fn()];
        //^^^^^^^^^^^^^^ error: cannot call non-const function
}

int div_zero_static() {
    static int a = 1 / 0;
                 //^^^^^ error: division by zero happened in const eval
    return a;
}

int div_zero_designator() {
    int a[] = { [1 / 0] = 5 };
               //^^^^^ error: division by zero happened in const eval
}

int div_zero_case() {
    switch (2) {
        case 3:
        case 1 / 0:
           //^^^^^ error: division by zero happened in const eval
            return 5;
    }
}

int div_zero_array_size() {
    int a[1 / 0];
        //^^^^^ error: division by zero happened in const eval
}

int rem_zero_array_size() {
    int a[5 % 0];
        //^^^^^ error: division by zero happened in const eval
}

int shift_array_size() {
    int a[1 << 200];
        //^^^^^^^^ error: shift out of bounds in const eval
}
