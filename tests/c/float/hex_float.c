//@ mode: c
//@ run-status: 0

int main(void) {
    double x = 0x1p4;
    return x == 16.0 ? 0 : 1;
}
