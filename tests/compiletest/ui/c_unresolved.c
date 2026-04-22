//@ mode: c
//@ compile-fail

int foo() {
    missing;
  //^^^^^^^ error: Unresolved name
}


int main() {
    return missing;
    //     ^^^^^^^ error: Unresolved name
}
