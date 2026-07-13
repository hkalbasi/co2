//@ mode: c
//@ compile-fail

int f() {
    return missing;
    //     ^^^^^^^ error: unresolved name missing
}

int f2() {
    return x(2);
         //^ error: unresolved name x
}
