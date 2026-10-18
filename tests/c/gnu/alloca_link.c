//@ mode: c
//@ run-status: 0
// A program that mentions `alloca` but never executes it must still link.
// Both an alloca in a constant-dead branch and an alloca used as the
// (unevaluated) operand of `sizeof` leave the backing store's drop glue in the
// object file; if the allocation path is dropped as dead code the link fails
// with `undefined symbol: <alloc::raw_vec::RawVecInner>::deallocate`.
#include <alloca.h>
#include <assert.h>

static void dead_branch(void) {
    if (0) {
        int *p = alloca(sizeof(int));
        *p = 1;
    }
}

static int never_evaluated(void) {
    return (int)sizeof(__builtin_alloca(4));
}

int main(void) {
    dead_branch();
    assert(never_evaluated() == (int)sizeof(void *));
    return 0;
}
