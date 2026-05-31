//@ mode: c
//@ compile-fail

int arr[(float)4];
//      ^^^^^^^^ error: unsupported cast target in constant expression: Float(F32)

int main(void) {
    return 0;
}
