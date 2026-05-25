//@ mode: c
//@ run-status: 0

#include <stdint.h>

typedef struct Packed {
    unsigned int low : 3;
    uint16_t high : 5;
    signed int delta : 5;
    uint8_t tag : 6;
} Packed;

typedef struct InnerBits {
    unsigned int x : 4;
    unsigned int y : 4;
} InnerBits;

typedef struct Container {
    InnerBits inner;
    unsigned int tail;
} Container;

typedef struct AnonymousBits {
    struct {
        unsigned int left : 3;
        int8_t right : 5;
    };
    unsigned int tail;
} AnonymousBits;

static int check_load_and_store(void) {
    Packed p = {5, 17, -3, 12};

    if (p.low != 5 || p.high != 17 || p.delta != -3 || p.tag != 12) {
        return 1;
    }
    if (p.low + p.high + p.tag != 34) {
        return 2;
    }

    p.high = 9;
    if (p.low != 5 || p.high != 9 || p.delta != -3 || p.tag != 12) {
        return 3;
    }

    p.low += 2;
    if (p.low != 7 || p.high != 9 || p.delta != -3 || p.tag != 12) {
        return 4;
    }

    p.delta = -7;
    if (p.low != 7 || p.high != 9 || p.delta != -7 || p.tag != 12) {
        return 5;
    }

    p.tag += 5;
    if (p.low != 7 || p.high != 9 || p.delta != -7 || p.tag != 17) {
        return 6;
    }

    return 0;
}

static int check_initializer_tree(void) {
    Container c = {{9, 3}, 77};

    if (c.inner.x != 9 || c.inner.y != 3 || c.tail != 77) {
        return 7;
    }

    c.inner.y = 10;
    if (c.inner.x != 9 || c.inner.y != 10 || c.tail != 77) {
        return 8;
    }

    return 0;
}

static int check_anonymous_fields(void) {
    AnonymousBits value = {6, -13, 41};

    if (value.left != 6 || value.right != -13 || value.tail != 41) {
        return 9;
    }

    value.left = 2;
    value.right += 4;
    if (value.left != 2 || value.right != -9 || value.tail != 41) {
        return 10;
    }

    return 0;
}

int main(void) {
    int rc;

    rc = check_load_and_store();
    if (rc != 0) {
        return rc;
    }

    rc = check_initializer_tree();
    if (rc != 0) {
        return rc;
    }

    rc = check_anonymous_fields();
    if (rc != 0) {
        return rc;
    }

    return 0;
}
