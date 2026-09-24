//@ mode: c
//@ run-status: 0
// Test for GNU extension: arithmetic on void pointers.
// In ISO C, sizeof(void) is undefined and void* arithmetic is a
// constraint violation. GCC (and Clang) define sizeof(void) == 1 and
// allow void* arithmetic as if it were char* arithmetic.

#include <stddef.h>
#include <assert.h>

// ------------------------------------------------------------------
// 2. Basic void* arithmetic: +, -, ++, --, +=, -=
// ------------------------------------------------------------------

void test_basic_arithmetic(void) {
    char buf[16];
    for (int i = 0; i < 16; i++) buf[i] = (char)i;

    void *p = buf;
    void *q;

    // Pointer + integer
    q = p + 3;
    assert((char *)q == buf + 3);
    assert(*(char *)q == 3);

    // Integer + pointer
    q = 5 + (void *)p;
    assert((char *)q == buf + 5);
    assert(*(char *)q == 5);

    // Pointer - integer
    q = (void *)(buf + 10) - 4;
    assert((char *)q == buf + 6);
    assert(*(char *)q == 6);

    // Increment / decrement
    void *r = buf;
    r++;
    assert((char *)r == buf + 1);
    ++r;
    assert((char *)r == buf + 2);
    r--;
    assert((char *)r == buf + 1);
    --r;
    assert((char *)r == buf + 0);

    // Compound assignment
    void *s = buf;
    s += 7;
    assert((char *)s == buf + 7);
    s -= 2;
    assert((char *)s == buf + 5);
}

// ------------------------------------------------------------------
// 3. Pointer difference on void*
// ------------------------------------------------------------------

void test_difference(void) {
    char buf[20];
    void *a = buf + 3;
    void *b = buf + 11;

    ptrdiff_t d = b - a;
    assert(d == 8);

    d = a - b;
    assert(d == -8);

    // Same pointer -> 0
    assert(a - a == 0);

    // Type of the difference is ptrdiff_t
    assert(_Generic(b - a, ptrdiff_t: 1, default: 0));
}

// ------------------------------------------------------------------
// 4. Relational comparisons on void*
// ------------------------------------------------------------------

void test_comparisons(void) {
    char buf[10];
    void *a = buf + 1;
    void *b = buf + 5;
    void *c = buf + 5;

    assert(a < b);
    assert(a <= b);
    assert(b > a);
    assert(b >= a);
    assert(b == c);
    assert(a != b);
    assert(b <= c);
    assert(b >= c);
}

// ------------------------------------------------------------------
// 5. Indexing a void* (GNU extension: p[i] is *(char*)p at byte i)
// ------------------------------------------------------------------

void test_indexing(void) {
    char buf[8];
    for (int i = 0; i < 8; i++) buf[i] = (char)(i * 2);

    void *p = buf;

    // GNU extension: p[i] indexes bytes
    assert(((char *)p)[0] == 0);
    assert(((char *)p)[3] == 6);

    // Direct void* indexing
    assert(*(char *)(p + 4) == 8);
    assert(*(char *)(p + 7) == 14);

    // Modify through void*
    *(char *)(p + 2) = 99;
    assert(buf[2] == 99);
}

// ------------------------------------------------------------------
// 6. void* arithmetic on a heap buffer
// ------------------------------------------------------------------

#include <stdlib.h>
#include <string.h>

void test_heap(void) {
    void *base = malloc(32);
    assert(base != NULL);

    memset(base, 0, 32);

    // Fill via void* arithmetic
    for (int i = 0; i < 32; i++) {
        *(char *)(base + i) = (char)(i + 1);
    }

    // Verify
    for (int i = 0; i < 32; i++) {
        assert(*(char *)(base + i) == (char)(i + 1));
    }

    // Mid buffer
    void *mid = base + 16;
    assert(*(char *)mid == 17);
    assert((char *)mid - (char *)base == 16);

    free(base);
}

// ------------------------------------------------------------------
// 7. void* arithmetic with different integer types
// ------------------------------------------------------------------

void test_integer_types(void) {
    char buf[64];
    void *p = buf;

    // int
    assert((char *)(p + 1) == buf + 1);
    // long
    long l = 5;
    assert((char *)(p + l) == buf + 5);
    // unsigned
    unsigned u = 10;
    assert((char *)(p + u) == buf + 10);
    // size_t
    size_t sz = 20;
    assert((char *)(p + sz) == buf + 20);
    // ptrdiff_t
    ptrdiff_t pd = 30;
    assert((char *)(p + pd) == buf + 30);
}

// ------------------------------------------------------------------
// 8. void* arithmetic in _Generic
// ------------------------------------------------------------------

void test_generic(void) {
    void *p = NULL;
    void *q = p + 1;

    // The result of void* arithmetic is void*
    assert(_Generic(q, void *: 1, default: 0));
    assert(_Generic(p + 1, void *: 1, default: 0));
    assert(_Generic(1 + p, void *: 1, default: 0));
    assert(_Generic(p - 1, void *: 1, default: 0));

    // Difference is ptrdiff_t
    assert(_Generic(p - q, ptrdiff_t: 1, default: 0));
}

// ------------------------------------------------------------------
// 9. Interaction with casts and function pointers (indirect)
//    (void* arithmetic, then cast to a typed pointer)
// ------------------------------------------------------------------

void test_cast_after_arith(void) {
    int arr[4] = { 10, 20, 30, 40 };
    void *base = arr;

    // Advance 2 * sizeof(int) bytes, then cast
    int *elem = (int *)(base + 2 * sizeof(int));
    assert(*elem == 30);

    // Modify via void* + cast
    *(int *)(base + 3 * sizeof(int)) = 99;
    assert(arr[3] == 99);

    // Cast to a struct pointer
    struct S { int a; int b; };
    struct S s = { .a = 1, .b = 2 };
    void *sp = &s;
    int *a_ptr = (int *)(sp + offsetof(struct S, b));
    assert(*a_ptr == 2);
}

// ------------------------------------------------------------------
// 10. Building generic byte-wise operations with void*
// ------------------------------------------------------------------

void *mem_reverse(void *buf, size_t n) {
    char *lo = (char *)(buf);
    char *hi = (char *)(buf + n - 1);
    while (lo < hi) {
        char t = *lo;
        *lo++ = *hi;
        *hi-- = t;
    }
    return buf;
}

void test_byte_ops(void) {
    char buf[5] = { 'a', 'b', 'c', 'd', 'e' };
    mem_reverse(buf, 5);
    assert(buf[0] == 'e');
    assert(buf[1] == 'd');
    assert(buf[2] == 'c');
    assert(buf[3] == 'b');
    assert(buf[4] == 'a');
}

// ------------------------------------------------------------------
// 11. void** arithmetic (pointer to void*, same rules)
// ------------------------------------------------------------------

void test_void_double_pointer(void) {
    void *arr[3] = { (void *)1, (void *)2, (void *)3 };
    void **p = arr;

    assert(p[0] == (void *)1);
    assert(p[1] == (void *)2);
    assert(p[2] == (void *)3);

    // p + 1 advances by sizeof(void*) bytes (pointer arithmetic on void**)
    void **q = p + 1;
    assert(*q == (void *)2);
    assert((char *)q - (char *)p == sizeof(void *));
}

// ------------------------------------------------------------------
// Main
// ------------------------------------------------------------------

int main(void) {
    test_basic_arithmetic();
    test_difference();
    test_comparisons();
    test_indexing();
    test_heap();
    test_integer_types();
    test_generic();
    test_cast_after_arith();
    test_byte_ops();
    test_void_double_pointer();
    return 0;
}
