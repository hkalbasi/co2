//@ mode: c
//@ run-status: 0

struct Inner {
    int vals[4];
};

struct Outer {
    struct Inner inner;
};

struct Outer items[2] = {
    [0].inner.vals[1 + 0 ... 2 + 0] = 7,
    [1].inner.vals[0] = 3,
    [1].inner.vals[3] = 9,
};

int main(void) {
    if (items[0].inner.vals[0] != 0) {
        return 1;
    }
    if (items[0].inner.vals[1] != 7 || items[0].inner.vals[2] != 7) {
        return 2;
    }
    if (items[0].inner.vals[3] != 0) {
        return 3;
    }
    if (items[1].inner.vals[0] != 3 || items[1].inner.vals[3] != 9) {
        return 4;
    }
    return 0;
}
