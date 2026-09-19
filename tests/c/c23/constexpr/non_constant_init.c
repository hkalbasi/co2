//@ mode: c
//@ compile-fail

int ext = 1;
constexpr int y = (int)(1.2 + 3.4);
            //    ^^^^^^^^^^^^^^^^ error: `constexpr` initializer must be a constant expression
constexpr int x = ext;
              //  ^^^ error: `constexpr` initializer must be a constant expression

int main(void) {
    return x;
}
