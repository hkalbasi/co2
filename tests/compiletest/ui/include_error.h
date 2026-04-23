//@ mode: c
//@ compile-fail

int f() {
    return missing;
    //     ^^^^^^^ error: Unresolved name
}
