//@ mode: c
//@ run-status: 0
// Test for variable-length arrays (VLAs) — C99, optional in C11/C23.
// Compile with: gcc -std=c11 -Wall -Wextra tests/c/vla.c

#include <stddef.h>
#include <assert.h>

// ------------------------------------------------------------------
// 1. Basic VLA
// ------------------------------------------------------------------

static void test_basic(void) {
    int n = 5;
    int a[n];
    assert(sizeof(a) == (size_t)n * sizeof(int));

    for (int i = 0; i < n; i++) a[i] = i * i;
    for (int i = 0; i < n; i++) assert(a[i] == i * i);

    // VLA of size 1 is still a VLA (size is not an ICE)
    int m = 1;
    int b[m];
    b[0] = 42;
    assert(b[0] == 42);
    assert(sizeof(b) == sizeof(int));
}

// ------------------------------------------------------------------
// 2. Multidimensional VLA
// ------------------------------------------------------------------

static void test_multidim(void) {
    int n = 3, m = 4;
    int a[n][m];
    assert(sizeof(a)      == (size_t)n * m * sizeof(int));
    assert(sizeof(a[0])   == (size_t)m * sizeof(int));
    assert(sizeof(a[0][0]) == sizeof(int));

    for (int i = 0; i < n; i++)
        for (int j = 0; j < m; j++)
            a[i][j] = i * 100 + j;

    for (int i = 0; i < n; i++)
        for (int j = 0; j < m; j++)
            assert(a[i][j] == i * 100 + j);

    // 3D VLA
    int k = 2;
    int b[n][m][k];
    assert(sizeof(b)         == (size_t)n * m * k * sizeof(int));
    assert(sizeof(b[0])      == (size_t)m * k * sizeof(int));
    assert(sizeof(b[0][0])   == (size_t)k * sizeof(int));
    assert(sizeof(b[0][0][0]) == sizeof(int));
}

// ------------------------------------------------------------------
// 3. sizeof on VLA (runtime value) vs. sizeof on fixed array
//    (integer constant expression).
// ------------------------------------------------------------------

static void test_sizeof(void) {
    // Fixed array: sizeof is a constant expression.
    enum { SZ = 8 };
    int fixed[SZ];
    _Static_assert(sizeof(fixed) == SZ * sizeof(int),
                   "fixed array sizeof is constant");
    assert(sizeof(fixed) == SZ * sizeof(int));

    // VLA: sizeof is a runtime value, not a constant expression.
    int n = 8;
    int vla[n];
    assert(sizeof(vla) == (size_t)n * sizeof(int));
    // _Static_assert(sizeof(vla) == SZ * sizeof(int), ""); // error

    // sizeof of a VLA type-name
    int m = 4;
    assert(sizeof(int[m]) == (size_t)m * sizeof(int));

    // `const int` is not an ICE in C, so `int[cn]` is a VLA.
    const int cn = 6;
    int cvla[cn];
    assert(sizeof(cvla) == 6 * sizeof(int));
}

// ------------------------------------------------------------------
// 4. sizeof of a VLA variable does not re-evaluate the size expression
// ------------------------------------------------------------------

static int compute_calls = 0;
static int compute(int v) { compute_calls++; return v; }

static void test_sizeof_no_reeval(void) {
    compute_calls = 0;
    int n = compute(4);
    assert(compute_calls == 1);

    int a[n];
    assert(compute_calls == 1);      // declaration uses n's value, no call

    size_t s = sizeof(a);
    assert(compute_calls == 1);      // sizeof reads the stored size
    assert(s == 4 * sizeof(int));
}

// ------------------------------------------------------------------
// 5. VLA typedef
// ------------------------------------------------------------------

static void test_typedef(void) {
    int n = 5;
    typedef int row_t[n];            // VLA typedef
    row_t r;
    for (int i = 0; i < n; i++) r[i] = i * i;
    for (int i = 0; i < n; i++) assert(r[i] == i * i);

    assert(sizeof(r)     == (size_t)n * sizeof(int));
    assert(sizeof(row_t) == (size_t)n * sizeof(int));

    n += 3;
    row_t r2;
    assert(sizeof(r2)    == (size_t)(n - 3) * sizeof(int));
    assert(sizeof(row_t) == (size_t)(n - 3) * sizeof(int));
}

// ------------------------------------------------------------------
// 6. VLA as function parameter — adjusted to pointer
// ------------------------------------------------------------------

// `int a[n]` parameter is adjusted to `int *a`.
static size_t param_sizeof_ptr(int n, int a[n]) {
    (void)n; (void)a;
    return sizeof(a);                // sizeof(int *), not VLA
}

// `*a` is `int`, so sizeof(*a) is sizeof(int), not n * sizeof(int).
static size_t param_sizeof_deref(int n, int a[n]) {
    (void)n; (void)a;
    return sizeof(*a);
}

// Pointer to VLA: `int (*a)[n]`. `*a` has type `int[n]`.
static size_t param_sizeof_vla_ptr(int n, int (*a)[n]) {
    (void)a;
    return sizeof(*a);               // n * sizeof(int)
}

static int sum_vla(int n, int a[n]) {
    int s = 0;
    for (int i = 0; i < n; i++) s += a[i];
    return s;
}

static void test_vla_param(void) {
    int n = 5;
    int arr[5] = { 1, 2, 3, 4, 5 };

    assert(param_sizeof_ptr(n, arr)   == sizeof(int *));
    assert(param_sizeof_deref(n, arr) == sizeof(int));
    assert(param_sizeof_vla_ptr(n, &arr) == 5 * sizeof(int));

    assert(sum_vla(n, arr) == 15);

    int m = 10;
    int arr2[10];
    for (int i = 0; i < m; i++) arr2[i] = i;
    assert(param_sizeof_vla_ptr(m, &arr2) == 10 * sizeof(int));
    assert(sum_vla(m, arr2) == 45);
}

// ------------------------------------------------------------------
// 7. VLA parameter with `static` (requires at least N elements)
// ------------------------------------------------------------------

static int sum_vla_static(int a[static 5]) {
    int s = 0;
    for (int i = 0; i < 5; i++) s += a[i];
    return s;
}

static void test_vla_param_static(void) {
    int arr[5] = { 10, 20, 30, 40, 50 };
    assert(sum_vla_static(arr) == 150);
}

// ------------------------------------------------------------------
// 8. VLA parameter with `[*]` in a prototype
// ------------------------------------------------------------------

// Prototype with `[*]` — allowed only in prototype scope.
static void fill_index(int n, int a[*]);

static void fill_index(int n, int a[n]) {
    for (int i = 0; i < n; i++) a[i] = i;
}

static void test_vla_param_star(void) {
    int arr[3];
    fill_index(3, arr);
    assert(arr[0] == 0 && arr[1] == 1 && arr[2] == 2);
}

// ------------------------------------------------------------------
// 9. const-qualified VLA parameter (const applies to the element)
// ------------------------------------------------------------------

static int sum_const_vla(int n, const int a[n]) {
    int s = 0;
    for (int i = 0; i < n; i++) s += a[i];
    return s;
}

static void test_const_vla_param(void) {
    const int arr[3] = { 4, 5, 6 };
    assert(sum_const_vla(3, arr) == 15);
}

// ------------------------------------------------------------------
// 10. Pointer to VLA
// ------------------------------------------------------------------

static void test_ptr_to_vla(void) {
    int n = 3, m = 4;
    int arr[m][n];
    for (int i = 0; i < m; i++)
        for (int j = 0; j < n; j++)
            arr[i][j] = i * 10 + j;

    int (*p)[n] = arr;
    assert(sizeof(*p) == (size_t)n * sizeof(int));

    for (int i = 0; i < m; i++)
        for (int j = 0; j < n; j++)
            assert(p[i][j] == i * 10 + j);

    // Advancing by one row moves n * sizeof(int) bytes.
    assert((char *)(p + 1) - (char *)p == n * (int)sizeof(int));
    p++;
    assert(p[0][0] == 10);
    p++;
    assert(p[0][0] == 20);
}

// ------------------------------------------------------------------
// 11. VLA lifetime: block scope
// ------------------------------------------------------------------

static void test_lifetime(void) {
    int n = 3;
    {
        int a[n];
        a[0] = 42;
        assert(a[0] == 42);
    }
    // a is out of scope; its storage is reclaimed at block exit.

    if (n > 0) {
        int b[n];
        b[0] = 7;
        assert(b[0] == 7);
    }
}

// ------------------------------------------------------------------
// 12. VLA inside a switch case block
// ------------------------------------------------------------------

static void test_vla_switch(void) {
    int n = 5;
    switch (n) {
        case 5: {
            int a[n];
            a[0] = 99;
            assert(a[0] == 99);
            assert(sizeof(a) == (size_t)n * sizeof(int));
            break;
        }
        default:
            break;
    }
}

// ------------------------------------------------------------------
// 13. VLA in a loop — each iteration has its own VLA
// ------------------------------------------------------------------

static void test_loop(void) {
    for (int i = 1; i <= 4; i++) {
        int a[i];
        for (int j = 0; j < i; j++) a[j] = i * 10 + j;
        for (int j = 0; j < i; j++) assert(a[j] == i * 10 + j);
        assert(sizeof(a) == (size_t)i * sizeof(int));

        typedef int b_t[i];

        b_t b1;
        assert(sizeof(b1) == (size_t)i * sizeof(int));
    
        i += 3;
        typedef int c_t[i];
        c_t c;
        assert(sizeof(c) == (size_t)i * sizeof(int));
        b_t b2;
        i -= 3;
        assert(sizeof(b2) == (size_t)i * sizeof(int));
    }
}

// ------------------------------------------------------------------
// 14. VLA size computed from a function call
// ------------------------------------------------------------------

static void test_computed_size(void) {
    compute_calls = 0;
    int a[compute(3)];
    assert(compute_calls == 1);      // size expression evaluated once
    for (int i = 0; i < 3; i++) a[i] = i;
    for (int i = 0; i < 3; i++) assert(a[i] == i);
    assert(sizeof(a) == 3 * sizeof(int));
}

// ------------------------------------------------------------------
// 15. const int vs. enum constant as array size
//     const int is NOT an ICE in C → VLA
//     enum constant IS an ICE in C → fixed array
// ------------------------------------------------------------------

static void test_const_vs_enum(void) {
    const int cn = 5;
    int vla[cn];                     // VLA (const int is not an ICE)
    assert(sizeof(vla) == 5 * sizeof(int));

    enum { EN = 5 };
    int fixed[EN];                   // fixed array (enum is an ICE)
    _Static_assert(sizeof(fixed) == EN * sizeof(int),
                   "fixed array, constant sizeof");
    assert(sizeof(fixed) == 5 * sizeof(int));
}

// ------------------------------------------------------------------
// 16. Multidimensional pointer arithmetic with VLA row stride
// ------------------------------------------------------------------

static void test_row_stride(void) {
    int n = 4, m = 5;
    int a[n][m];
    for (int i = 0; i < n; i++)
        for (int j = 0; j < m; j++)
            a[i][j] = i * m + j;

    int (*p)[m] = a;
    for (int i = 0; i < n; i++) {
        for (int j = 0; j < m; j++)
            assert(p[i][j] == i * m + j);
        if (i + 1 < n) {
            assert((char *)(p + i + 1) - (char *)(p + i)
                   == m * (int)sizeof(int));
        }
    }
}

// ------------------------------------------------------------------
// 17. sizeof of VLA does not change when elements are modified
// ------------------------------------------------------------------

static void test_sizeof_stable(void) {
    int n = 5;
    int a[n];
    size_t s1 = sizeof(a);
    a[0] = 123;
    a[n - 1] = -7;
    size_t s2 = sizeof(a);
    assert(s1 == s2);
    assert(s1 == 5 * sizeof(int));
}

// ------------------------------------------------------------------
// main
// ------------------------------------------------------------------

int main(void) {
    test_basic();
    test_multidim();
    test_sizeof();
    test_sizeof_no_reeval();
    test_typedef();
    test_vla_param();
    test_vla_param_static();
    test_vla_param_star();
    test_const_vla_param();
    test_ptr_to_vla();
    test_lifetime();
    test_vla_switch();
    test_loop();
    test_computed_size();
    test_const_vs_enum();
    test_row_stride();
    test_sizeof_stable();
    return 0;
}
