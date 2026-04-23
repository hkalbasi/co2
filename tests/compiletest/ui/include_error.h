//@ mode: c
//@ compile-fail

int f() {
    return missing;
    //     ^^^^^^^ error: Unresolved name
}

int f2() {
    int x = 5;
    return x(2);
         //^^^^ Type is not callable
}
