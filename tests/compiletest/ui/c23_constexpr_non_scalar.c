//@ mode: c
//@ compile-fail

struct Pair {
    int left;
    int right;
};

constexpr struct Pair pair = { 1, 2 };
      //  ^^^^^^^^^^^ error: `constexpr` object type must be scalar

int main(void) {
    return pair.left;
}
