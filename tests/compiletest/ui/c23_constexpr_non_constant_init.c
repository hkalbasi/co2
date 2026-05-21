//@ mode: c
//@ compile-fail

int ext = 1;
constexpr int x = ext;
              //  ^^^ error: `constexpr` initializer must be a constant expression

int main(void) {
    return x;
}
