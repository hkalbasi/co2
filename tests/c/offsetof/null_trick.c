//@ mode: c
//@ run-status: 0

#include <stddef.h>
#include <assert.h>

struct S { char a; int b; };
struct T { int off; } t = { (int)(long)&((struct S *)0)->b };
                               //^^^^^^^^^^^^^^^^^^^^^^^^^ warning: dereferencing null is UB, use offsetof macro

int main(void){
    assert(t.off == offsetof(struct S, b));

    int off = (int)&((struct S *)0)->b;
            //^^^^^^^^^^^^^^^^^^^^^^^^ warning: dereferencing null is UB, use offsetof macro
    assert(off == offsetof(struct S, b));
    return 0;
}
