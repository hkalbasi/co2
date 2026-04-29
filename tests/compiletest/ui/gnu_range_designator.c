//@ mode: c
//@ compile-fail

int table[2] = {
    [0 ... 1] = 0,
//  ^^^^^^^^^ unsupported GNU range designator
};

int main(void) {
    return table[0];
}
