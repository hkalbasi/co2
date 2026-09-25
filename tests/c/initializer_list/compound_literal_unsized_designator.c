//@ mode: c
//@ run-status: 0

#include <assert.h>

// Unsized compound literals must infer their length from the initializer,
// including designators (same as `int x[] = { [2] = 5 };` locals).

int main(void) {
    // Single designator grows the array to index+1.
    int *p = (int[]){ [2] = 5 };
    assert(p[0] == 0 && p[1] == 0 && p[2] == 5);

    // Mixed positional + designator.
    int *q = (int[]){ 1, [3] = 8 };
    assert(q[0] == 1 && q[1] == 0 && q[2] == 0 && q[3] == 8);

    // Positional-only still infers the item count.
    int *r = (int[]){ 1, 2, 3, 4 };
    assert(r[0] == 1 && r[1] == 2 && r[2] == 3 && r[3] == 4);

    // String literal still contributes its NUL-terminated length.
    char *s = (char[]){ "ab" };
    assert(s[0] == 'a' && s[1] == 'b' && s[2] == '\0');

    return 0;
}
