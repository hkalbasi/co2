//@ mode: c
//@ run-status: 0

union U {
    int a;
    int b;
};

int main(void) {
    union U u = {.b = 7};
    return u.a != 7;
}
