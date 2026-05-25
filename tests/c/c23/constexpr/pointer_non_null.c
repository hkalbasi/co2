//@ mode: c
//@ compile-fail

int ext = 1;
constexpr int *ptr = &ext;
                 //  ^^^^ error: `constexpr` pointer initializer must be null

int main(void) {
    return *ptr;
}
