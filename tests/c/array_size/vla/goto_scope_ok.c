//@ mode: c
//@ run-status: 0
//
// C11 6.8.6.1p1 only forbids *entering* the scope of a variably modified
// identifier, so these jumps are legal.

static int leave_scope(int n) {
    int r = 0;
    {
        int a[n];
        a[0] = 7;
        r = a[0];
        goto done; // leaves `a`'s scope
    }
done:
    return r;
}

static int leave_backward(int n) {
    int i = 0;
loop:
    {
        int a[n];
        a[0] = ++i;
        if (a[0] < 2) goto loop; // leaves `a`'s scope each iteration
    }
    return i;
}

static int label_before_decl(int n) {
    goto skip;
skip:
    ; // `a` is not in scope here yet
    int a[n];
    a[0] = 9;
    return a[0];
}

static int vla_param(int n, int a[n]) {
    goto done; // a parameter is in scope for the whole body
done:
    return a[0];
}

int main(void) {
    int seed[1] = {1};
    if (leave_scope(1) != 7) return 1;
    if (leave_backward(1) != 2) return 2;
    if (label_before_decl(1) != 9) return 3;
    if (vla_param(1, seed) != 1) return 4;
    return 0;
}
