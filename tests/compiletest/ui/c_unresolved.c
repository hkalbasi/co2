//@ mode: c
//@ compile-fail

int f1() {
    missing;
  //^^^^^^^ error: Unresolved name
}

int f2() {
    missing x = 5;
  //^^^^^^^ error: Unresolved name
}

int f3() {
    int x = 2;
    x* y = &x;
     //^ error: Unresolved name
}

int main() {
    return missing;
    //     ^^^^^^^ error: Unresolved name
}
