//@ mode: c
//@ run-status: 0

typedef int fn;

fn foo() {
    return 5;
}

int foo2() {
    int u64 = 64;
    return u64;
}

int main() {
    if (foo() != 5) {
        return 1;
    }
    if (foo2() != 64) {
        return 2;
    }
    return 0;
}
