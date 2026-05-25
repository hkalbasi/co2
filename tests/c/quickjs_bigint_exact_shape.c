//@ mode: c

static void js_bigint_set_short(int *buf, int value) {
    *buf = value;
}

static int quickjs_shaped_branch(int tag, int value) {
    int *p;
    int buf;

    if (tag == 0) {
        return 0;
    }

    goto slow_big_int;

slow_big_int:
    p = &buf;
    js_bigint_set_short(p, value);
    return buf;
}

int main(void) {
    return quickjs_shaped_branch(1, 7) == 7 ? 0 : 1;
}
