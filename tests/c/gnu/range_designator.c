//@ mode: c
//@ run-status: 0

int table[4] = {
    [0 ... 2] = 7,
    [3] = 9,
};

int main(void) {
    return table[0] == 7 && table[1] == 7 && table[2] == 7 && table[3] == 9 ? 0 : 1;
}
