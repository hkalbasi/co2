//@ mode: c
//@ compile-fail

int f() {
    return missing;
    //     ^^^^^^^ error: Unresolved name missing
}

int f2() {
    return x(2);
         //^ error: Unresolved name x
}
