//@ mode: c
//@ run-status: 0

typedef int fn;

fn foo() {
    return 5;
}

int main() {
    if (foo() != 5) {
        return 1;
    }
    return 0;
}
