//@ mode: c
//@ compile-fail

int f1() {
  int *p;
  long *q;
  int x = 1;
  long y = 2;

  p = &x;
  q = &y;

  void *result = 1 ? p : q;
//               ^^^^^^^^^ error: ternary operator branches have mismatched types: expected *mut i32, got *mut i64

  return 0;
}

void f2(bool b, int *p) {
  b ? p : 5;
//^^^^^^^^^ error: ternary operator branches have mismatched types: expected *mut i32, got i32
}

void f3(bool b, int *p) {
  b ? p : (long*)0;
//^^^^^^^^^^^^^^^^ error: ternary operator branches have mismatched types: expected *mut i32, got *mut i64
}

int main() {
  return 0;
}
