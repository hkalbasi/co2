//@ mode: c
//@ compile-fail

void f1(int* a, int* b) {
    a + b;
  //^^^^^ error: type error: adding two pointers is invalid
}

void f2(int* a, int* b) {
    a[b];
  //^^^^ error: type error: adding two pointers is invalid
}

void f3(int a, int b) {
    a[b];
  //^^^^ error: subscript requires one pointer and one integer operand
}

int main() {}
