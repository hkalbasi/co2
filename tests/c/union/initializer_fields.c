//@ mode: c
//@ run-status: 0

union U {
    int a;
    int b;
};

struct S {
    int head;
    union U u;
    int tail;
};

static union U by_designator = {.b = 7};
static struct S by_position = {1, 2, 3};

int main(void) {
    if (by_designator.a != 7 || by_designator.b != 7) {
        return 1;
    }
    if (by_position.head != 1) {
        return 2;
    }
    if (by_position.u.a != 2 || by_position.u.b != 2) {
        return 3;
    }
    if (by_position.tail != 3) {
        return 4;
    }
    return 0;
}
