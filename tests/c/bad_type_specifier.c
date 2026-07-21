//@ mode: c
//@ compile-fail

int f1() {
    unsigned signed int x;
  //^^^^^^^^^^^^^^^^^^^ error: duplicate sign specifier found
}

int f2() {
    char int x;
  //^^^^^^^^ error: duplicate base specifier found
}

int f3() {
    int int x;
  //^^^^^^^ error: duplicate base specifier found
}

int main() {}