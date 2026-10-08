//@ mode: c
//@ run-status: 0
// `case`/`default` are standalone labels (like `label:` after the goto fix).
// Covers the C23 relaxations:
//   * `case`/`default` at end of compound (no trailing statement)
//   * `case`/`default` followed by a declaration

#include <assert.h>

// ------------------------------------------------------------------
// 1. `case` at end of compound (no trailing statement)
// ------------------------------------------------------------------
static int t_case_end(int n) {
    int r = 0;
    switch (n) {
        case 1:
            r = 1;
            break;
        case 2:
    }
    return r; // 1 for n==1, 0 otherwise
}

// ------------------------------------------------------------------
// 2. `default` at end of compound (no trailing statement)
// ------------------------------------------------------------------
static int t_default_end(int n) {
    int r = 0;
    switch (n) {
        case 1:
            r = 1;
            break;
        default:
    }
    return r; // 1 for n==1, 0 otherwise
}

// ------------------------------------------------------------------
// 3. `case` followed by a declaration
// ------------------------------------------------------------------
static int t_case_decl(int n) {
    int r = 0;
    switch (n) {
        case 1:
            r = 10;
            break;
        case 2:
            int y = 5;
            r = y + 1;
            break;
        default:
            r = -1;
            break;
    }
    return r; // 6 for n==2
}

// ------------------------------------------------------------------
// 4. `default` followed by a declaration
// ------------------------------------------------------------------
static int t_default_decl(int n) {
    int r = 0;
    switch (n) {
        case 1:
            r = 10;
            break;
        default:
            int y = 7;
            r = y + 1;
            break;
    }
    return r; // 8 for n!=1
}

int main(void) {
    assert(t_case_end(1) == 1);
    assert(t_case_end(2) == 0);
    assert(t_case_end(3) == 0);
    assert(t_default_end(1) == 1);
    assert(t_default_end(2) == 0);
    assert(t_case_decl(1) == 10);
    assert(t_case_decl(2) == 6);
    assert(t_case_decl(9) == -1);
    assert(t_default_decl(1) == 10);
    assert(t_default_decl(2) == 8);
    return 0;
}
