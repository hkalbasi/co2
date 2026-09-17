//@ mode: c
//@ run-status: 0
// Test for offsetof (C89/C99/C11/C23) – all forms

#include <stddef.h>
#include <assert.h>

// ------------------------------------------------------------
// Basic struct
// ------------------------------------------------------------

struct point {
    int x;
    int y;
};

struct mixed {
    char a;
    int b;
    char c;
    long d;
    double e;
};

struct with_array {
    int arr[4];
    char c;
    long l;
};

struct with_nested {
    int a;
    struct point p;
    int b;
};

struct with_union {
    int a;
    union {
        int i;
        double d;
        char c[8];
    } u;
    int b;
};

struct deep_inner {
    int x;
    int vals[3];
};

struct deep_mid {
    int tag;
    struct deep_inner elems[4];
};

struct deep_outer {
    int head;
    struct deep_mid groups[2];
    struct deep_mid tail;
};

struct with_bitfields {
    unsigned a : 3;
    unsigned b : 5;
    int c;
    unsigned d : 1;
};

struct empty_like {
    char c;
};

union u {
    int i;
    double d;
    char c[16];
};

// ------------------------------------------------------------
// 1. Basic offsets in a simple struct
// ------------------------------------------------------------

void test_basic(void) {
    assert(offsetof(struct point, x) == 0);
    assert(offsetof(struct point, y) == sizeof(int));

    // Total size must be at least offset + size of last member
    assert(sizeof(struct point) >= offsetof(struct point, y) + sizeof(int));
}

// ------------------------------------------------------------
// 2. Offsets with alignment padding
// ------------------------------------------------------------

void test_padding(void) {
    // char a at 0
    assert(offsetof(struct mixed, a) == 0);
    // int b must be aligned to _Alignof(int)
    assert(offsetof(struct mixed, b) % _Alignof(int) == 0);
    assert(offsetof(struct mixed, b) >= 1);
    // char c right after b
    assert(offsetof(struct mixed, c) == offsetof(struct mixed, b) + sizeof(int));
    // long d must be aligned
    assert(offsetof(struct mixed, d) % _Alignof(long) == 0);
    // double e must be aligned
    assert(offsetof(struct mixed, e) % _Alignof(double) == 0);
    // Offsets must be increasing
    assert(offsetof(struct mixed, a) < offsetof(struct mixed, b));
    assert(offsetof(struct mixed, b) < offsetof(struct mixed, c));
    assert(offsetof(struct mixed, c) < offsetof(struct mixed, d));
    assert(offsetof(struct mixed, d) < offsetof(struct mixed, e));
}

// ------------------------------------------------------------
// 3. Array members
// ------------------------------------------------------------

void test_array(void) {
    assert(offsetof(struct with_array, arr) == 0);
    assert(offsetof(struct with_array, c) == sizeof(int[4]));
    // l must be aligned
    assert(offsetof(struct with_array, l) % _Alignof(long) == 0);
    assert(offsetof(struct with_array, l) >= sizeof(int[4]) + sizeof(char));

    // offsetof of an array element (using array subscript)
    // offsetof(struct with_array, arr[0]) == offsetof(struct with_array, arr)
    assert(offsetof(struct with_array, arr[0]) == offsetof(struct with_array, arr));
    assert(offsetof(struct with_array, arr[2]) == offsetof(struct with_array, arr) + 2 * sizeof(int));
}

// ------------------------------------------------------------
// 4. Nested structs
// ------------------------------------------------------------

void test_nested(void) {
    assert(offsetof(struct with_nested, a) == 0);
    assert(offsetof(struct with_nested, p) % _Alignof(struct point) == 0);
    assert(offsetof(struct with_nested, p) >= sizeof(int));

    // Nested member access: offsetof(struct with_nested, p.x)
    assert(offsetof(struct with_nested, p.x) == offsetof(struct with_nested, p));
    assert(offsetof(struct with_nested, p.y) ==
           offsetof(struct with_nested, p) + offsetof(struct point, y));

    assert(offsetof(struct with_nested, b) ==
           offsetof(struct with_nested, p) + sizeof(struct point));
}

// ------------------------------------------------------------
// 4b. Deeply nested designators mixing arrays and fields
// ------------------------------------------------------------

void test_deep(void) {
    // field -> array -> field
    assert(offsetof(struct deep_outer, groups[1].tag) ==
           offsetof(struct deep_outer, groups) + sizeof(struct deep_mid));
    // field -> array -> array -> field (base equivalence)
    assert(offsetof(struct deep_outer, groups[0].elems[0].x) ==
           offsetof(struct deep_outer, groups) + offsetof(struct deep_mid, elems));
    // field -> array -> array -> array -> indexed element
    assert(offsetof(struct deep_outer, groups[1].elems[2].vals[1]) ==
           offsetof(struct deep_outer, groups) + sizeof(struct deep_mid) +
           offsetof(struct deep_mid, elems) + 2 * sizeof(struct deep_inner) +
           offsetof(struct deep_inner, vals) + sizeof(int));

    // field -> field -> array -> array -> indexed element
    assert(offsetof(struct deep_outer, tail.elems[2].vals[1]) ==
           offsetof(struct deep_outer, tail) +
           offsetof(struct deep_mid, elems) + 2 * sizeof(struct deep_inner) +
           offsetof(struct deep_inner, vals) + sizeof(int));
}

// ------------------------------------------------------------
// 5. Union members
// ------------------------------------------------------------

void test_union(void) {
    // All union members have offset 0
    assert(offsetof(union u, i) == 0);
    assert(offsetof(union u, d) == 0);
    assert(offsetof(union u, c) == 0);
    assert(offsetof(union u, c[0]) == 0);
    assert(offsetof(union u, c[5]) == 5);

    // Struct containing a union
    assert(offsetof(struct with_union, a) == 0);
    assert(offsetof(struct with_union, u) % _Alignof(union { int i; double d; char c[8]; }) == 0);
    // All members of the union have the same offset within the struct
    assert(offsetof(struct with_union, u.i) == offsetof(struct with_union, u));
    assert(offsetof(struct with_union, u.d) == offsetof(struct with_union, u));
    assert(offsetof(struct with_union, u.c) == offsetof(struct with_union, u));
}

// ------------------------------------------------------------
// 6. Bitfields – standard says offsetof is not defined for bitfields.
//    But we can test that it compiles on some compilers; however,
//    the C standard makes it undefined behavior / constraint violation.
//    We skip bitfields to remain portable.
// ------------------------------------------------------------

// ------------------------------------------------------------
// 7. Using offsetof to access a member via a pointer
// ------------------------------------------------------------

void test_access_via_pointer(void) {
    struct point p = { .x = 10, .y = 20 };
    char *base = (char *)&p;

    int *xptr = (int *)(base + offsetof(struct point, x));
    int *yptr = (int *)(base + offsetof(struct point, y));

    assert(*xptr == 10);
    assert(*yptr == 20);

    *yptr = 99;
    assert(p.y == 99);
}

// ------------------------------------------------------------
// 8. Using offsetof in a generic macro (container_of style)
// ------------------------------------------------------------

#define container_of(ptr, type, member) \
    ((type *)((char *)(ptr) - offsetof(type, member)))

struct container {
    int before;
    struct point p;
    int after;
};

void test_container_of(void) {
    struct container c = { .before = 1, .p = { .x = 2, .y = 3 }, .after = 4 };
    struct point *pp = &c.p;
    struct container *cp = container_of(pp, struct container, p);
    assert(cp == &c);
    assert(cp->before == 1);
    assert(cp->after == 4);
    assert(cp->p.x == 2 && cp->p.y == 3);
}

// ------------------------------------------------------------
// 9. offsetof returns size_t
// ------------------------------------------------------------

void test_type(void) {
    size_t off = offsetof(struct point, y);
    assert(off == sizeof(int));

    // _Generic check: the type of offsetof(...) should be size_t
    assert(_Generic(offsetof(struct point, x), size_t: 1, default: 0));
}

// ------------------------------------------------------------
// 10. Constant expression: usable in static initializers and array sizes
// ------------------------------------------------------------

struct padded {
    char c;
    int i;
};

// Use offsetof in an array size (must be integer constant expression)
char offset_arr[offsetof(struct padded, i) + 1];
char *offset_arr_p = offset_arr;  // avoid unused warning

// Use offsetof in an enum constant
enum { OFF_I = offsetof(struct padded, i) };

void test_constant_expression(void) {
    assert(OFF_I == offsetof(struct padded, i));
    assert(sizeof(offset_arr) == offsetof(struct padded, i) + 1);
}

// ------------------------------------------------------------
// 11. Array of structs: offset + element index
// ------------------------------------------------------------

void test_array_of_structs(void) {
    struct point pts[3] = { {1,2}, {3,4}, {5,6} };
    char *base = (char *)pts;

    // Address of pts[1].y
    int *yptr = (int *)(base + sizeof(struct point) * 1 + offsetof(struct point, y));
    assert(*yptr == 4);

    // Address of pts[2].x
    int *xptr = (int *)(base + sizeof(struct point) * 2 + offsetof(struct point, x));
    assert(*xptr == 5);
}

// ------------------------------------------------------------
// 12. offsetof with typedef'd struct
// ------------------------------------------------------------

typedef struct point point_t;

void test_typedef(void) {
    assert(offsetof(point_t, x) == 0);
    assert(offsetof(point_t, y) == sizeof(int));
}

// ------------------------------------------------------------
// 13. Zero-sized member? Not standard, skip.
// ------------------------------------------------------------

// ------------------------------------------------------------
// Main
// ------------------------------------------------------------

int main(void) {
    test_basic();
    test_padding();
    test_array();
    test_nested();
    test_deep();
    test_union();
    test_access_via_pointer();
    test_container_of();
    test_type();
    test_constant_expression();
    test_array_of_structs();
    test_typedef();
    return 0;
}
