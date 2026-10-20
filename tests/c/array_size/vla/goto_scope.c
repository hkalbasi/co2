//@ mode: c
//@ compile-fail
//
// C11 6.8.6.1p1: no jump may enter the scope of a variably modified
// identifier, whether it is a forward goto, a backward goto, a sibling-block
// goto, or a computed goto.

void forward(int n) {
    goto skip;
    int a[n];
    //  ^^^^ error: jump into scope of variable length array
skip:
    ;
}

void backward(int n) {
    {
        int a[n];
        //  ^^^^ error: jump into scope of variable length array
    again:
        a[0] = 0;
    }
    goto again;
}

void sibling(int n) {
    {
        goto target;
    }
    {
        int c[n];
        //  ^^^^ error: jump into scope of variable length array
    target:
        c[0] = 0;
    }
}

void computed(int n) {
    void *p = &&target;
    goto *p;
    {
        int b[n];
        //  ^^^^ error: jump into scope of variable length array
    target:
        b[0] = 0;
    }
}
