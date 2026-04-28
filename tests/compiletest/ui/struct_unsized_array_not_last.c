//@ mode: c
//@ compile-fail

// Unsized array must be the last field in a struct
// This tests that the compiler rejects unsized arrays in non-last positions

struct Bad1 {
    int arr[];  // not last - has 'int x' after
//      ^^^^^ error: unsized array is not a first-class declaration type in this context
    int x;
};

int main() {
    return 0;
}
