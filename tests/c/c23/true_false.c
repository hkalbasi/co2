//@ mode: c
//@ run-status: 0

#define case bool

int main() {
    if (false && true) {
        return 1;
    }
    if (sizeof(false) != 1) {
        return 2;
    }
    if (sizeof(bool) != 1) {
        return 2;
    }
    if (sizeof(case) != 1) {
        return 2;
    }
    return 0;
}
