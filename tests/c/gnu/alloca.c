//@ mode: c
//@ run-status: 0
// Test for alloca() — GNU extension, also available as __builtin_alloca.
// alloca(size) allocates `size` bytes on the stack of the *calling*
// function. The memory is automatically freed when the function returns
// (not when the block containing the call exits). The returned pointer
// is suitably aligned for any object type and is valid until function
// return. Calling alloca with 0 or with a huge size is dangerous; we
// test 0 only where it is well-defined (GCC returns a valid pointer).

#include <stddef.h>
#include <assert.h>
#include <string.h>
#include <stdint.h>

#include <alloca.h>

// ------------------------------------------------------------------
// 1. Basic: allocate one int, write, read
// ------------------------------------------------------------------

static void test_basic(void) {
    int *p = alloca(sizeof(int));
    assert(p != NULL);
    *p = 42;
    assert(*p == 42);

    // alloca and __builtin_alloca are the same thing.
    int *q = __builtin_alloca(sizeof(int));
    assert(q != NULL);
    *q = 7;
    assert(*q == 7);
}

// ------------------------------------------------------------------
// 2. Alignment: the result is maximally aligned, like malloc's
//    alignment guarantee (>= _Alignof(max_align_t) in practice).
// ------------------------------------------------------------------

static void test_alignment(void) {
    // Allocate a few differently-sized buffers and check alignment
    // for the widest fundamental types.
    for (size_t n = 1; n <= 64; n++) {
        void *p = alloca(n);
        assert(p != NULL);
        assert(((uintptr_t)p % _Alignof(max_align_t)) == 0);
    }
}

// ------------------------------------------------------------------
// 3. Types: allocate arrays, structs, aligned buffers
// ------------------------------------------------------------------

struct S { int a; double b; char c[3]; };

static void test_types(void) {
    // Array of ints
    int *arr = alloca(10 * sizeof(int));
    for (int i = 0; i < 10; i++) arr[i] = i * i;
    for (int i = 0; i < 10; i++) assert(arr[i] == i * i);

    // Struct
    struct S *s = alloca(sizeof(struct S));
    s->a = 1;
    s->b = 2.5;
    s->c[0] = 'x'; s->c[1] = 'y'; s->c[2] = 'z';
    assert(s->a == 1 && s->b == 2.5);
    assert(s->c[0] == 'x' && s->c[1] == 'y' && s->c[2] == 'z');

    // Array of structs
    struct S *sarr = alloca(4 * sizeof(struct S));
    for (int i = 0; i < 4; i++) {
        sarr[i].a = i;
        sarr[i].b = i * 1.5;
    }
    for (int i = 0; i < 4; i++) {
        assert(sarr[i].a == i);
        assert(sarr[i].b == i * 1.5);
    }

    // _Alignas / max_align_t-aligned buffer
    void *buf = alloca(64);
    assert(((uintptr_t)buf % _Alignof(double)) == 0);
    memset(buf, 0xAA, 64);
    unsigned char *b = buf;
    for (int i = 0; i < 64; i++) assert(b[i] == 0xAA);
}

// ------------------------------------------------------------------
// 4. Zero-size alloca: GCC returns a unique pointer that must not be
//    dereferenced; it need not be NULL.
// ------------------------------------------------------------------

static void test_zero(void) {
    void *p = alloca(0);
    // Allowed to be non-NULL; we must not dereference it.
    (void)p;
    // GCC: successive alloca(0) calls return distinct pointers.
    void *q = alloca(0);
    (void)q;
    // No assertion required; the important thing is that this does not
    // crash or corrupt the stack.
}

// ------------------------------------------------------------------
// 5. Variable size (runtime value)
// ------------------------------------------------------------------

static void test_variable_size(size_t n) {
    unsigned char *p = alloca(n);
    assert(p != NULL);
    for (size_t i = 0; i < n; i++) p[i] = (unsigned char)i;
    for (size_t i = 0; i < n; i++) assert(p[i] == (unsigned char)i);
}

// ------------------------------------------------------------------
// 6. Multiple allocations are distinct and don't overlap.
// ------------------------------------------------------------------

static void test_no_overlap(void) {
    int *a = alloca(sizeof(int));
    int *b = alloca(sizeof(int));
    int *c = alloca(sizeof(int));
    assert(a != b);
    assert(b != c);
    assert(a != c);

    *a = 1; *b = 2; *c = 3;
    assert(*a == 1);
    assert(*b == 2);
    assert(*c == 3);
}

// ------------------------------------------------------------------
// 7. Memory lifetime: alloca memory survives block exit and is only
//    released at function return. This is the key difference from VLAs.
// ------------------------------------------------------------------

static void test_lifetime_block_exit(void) {
    int *p;
    {
        p = alloca(sizeof(int));
        *p = 99;
    }
    // p is still valid here — alloca memory lives until function return.
    assert(*p == 99);

    int *q;
    {
        int arr[3];              // VLA-like lifetime (not alloca)
        q = alloca(3 * sizeof(int));
        q[0] = 10; q[1] = 20; q[2] = 30;
        (void)arr;
    }
    // q is still valid.
    assert(q[0] == 10 && q[1] == 20 && q[2] == 30);
}

// ------------------------------------------------------------------
// 8. alloca in a loop: memory accumulates; nothing is freed inside
//    the loop. This is why alloca must never be used in loops with
//    unbounded iteration counts.
// ------------------------------------------------------------------

static void test_loop(void) {
    // Bounded loop with a few iterations; each alloca grows the stack
    // and none is released until function return.
    int *ptrs[8];
    for (int i = 0; i < 8; i++) {
        ptrs[i] = alloca(sizeof(int));
        *ptrs[i] = i;
    }
    for (int i = 0; i < 8; i++) {
        assert(*ptrs[i] == i);
    }
    // All 8 ints are simultaneously live.
    for (int i = 0; i < 8; i++) assert(ptrs[i] != NULL);
}

// ------------------------------------------------------------------
// 9. Return a pointer to alloca memory from a helper (must NOT be used
//    after the helper returns). We only verify it works while the
//    helper is on the stack; we don't return it past the function.
// ------------------------------------------------------------------

static void use_during_call(int *p, int n) {
    for (int i = 0; i < n; i++) assert(p[i] == i * 7);
}

static void test_nested_call(void) {
    int *p = alloca(5 * sizeof(int));
    for (int i = 0; i < 5; i++) p[i] = i * 7;
    use_during_call(p, 5);
}

// ------------------------------------------------------------------
// 10. Interaction with VLAs: both live on the stack; alloca memory
//     outlives the enclosing block, VLA memory does not.
// ------------------------------------------------------------------

static void test_vla_interaction(void) {
    int *p = alloca(4 * sizeof(int));
    for (int i = 0; i < 4; i++) p[i] = i + 1;

    {
        int vla[4];
        for (int i = 0; i < 4; i++) vla[i] = 100 + i;
        assert(vla[0] == 100 && vla[3] == 103);
    }
    // VLA is out of scope; p is still alive.
    assert(p[0] == 1 && p[1] == 2 && p[2] == 3 && p[3] == 4);
}

// ------------------------------------------------------------------
// 11. sizeof on alloca result: not an expression of array type, so
//     sizeof(alloca(...)) is sizeof(void*) == sizeof pointer, not the
//     allocated size.
// ------------------------------------------------------------------

static void test_sizeof(void) {
    void *p = alloca(128);
    // sizeof(p) is pointer size, not 128.
    assert(sizeof(p) == sizeof(void *));
    // The allocated size is only known to us; we can use it via a cast.
    memset(p, 0, 128);
    unsigned char *b = p;
    for (int i = 0; i < 128; i++) assert(b[i] == 0);
}

// ------------------------------------------------------------------
// 12. Zero-initialise via memset (alloca does not zero memory).
//     We don't test "is it zero" because that's not guaranteed.
// ------------------------------------------------------------------

static void test_not_zeroed(void) {
    // We don't assume anything about initial contents; just write.
    int *p = alloca(sizeof(int));
    *p = 0;
    assert(*p == 0);
}

// ------------------------------------------------------------------
// 13. String buffers: build a string on the stack.
// ------------------------------------------------------------------

static void test_string(void) {
    char *buf = alloca(32);
    strcpy(buf, "hello");
    assert(strcmp(buf, "hello") == 0);
    strcat(buf, ", world");
    assert(strcmp(buf, "hello, world") == 0);
    assert(buf[0] == 'h');
    assert(strlen(buf) == 12);
}

// ------------------------------------------------------------------
// 14. Read and write with memcpy / memmove / memcmp.
// ------------------------------------------------------------------

static void test_mem_ops(void) {
    unsigned char *a = alloca(32);
    unsigned char *b = alloca(32);
    for (int i = 0; i < 32; i++) a[i] = (unsigned char)(i * 3);
    memcpy(b, a, 32);
    assert(memcmp(a, b, 32) == 0);

    memmove(a + 4, a, 28);
    // After the move, a[4..31] == original a[0..27]
    for (int i = 4; i < 32; i++) {
        assert(a[i] == (unsigned char)((i - 4) * 3));
    }
}

// ------------------------------------------------------------------
// 15. Recursion: each recursion frame gets its own alloca memory.
// ------------------------------------------------------------------

static int recurse(int depth, int *prev_ptr) {
    int *p = alloca(sizeof(int));
    *p = depth;
    if (prev_ptr != NULL) {
        // Memory from the caller's frame is still valid (it's above us
        // on the stack), and its value is the caller's depth.
        assert(*prev_ptr == depth + 1);
        // It must be a different address from ours.
        assert(p != prev_ptr);
    }
    if (depth == 0) return 0;
    return recurse(depth - 1, p) + *p;
}

static void test_recursion(void) {
    int sum = recurse(5, NULL);   // 5 + 4 + 3 + 2 + 1 + 0 = 15
    assert(sum == 15);
}

// ------------------------------------------------------------------
// 16. Large-ish allocation (a few KB) works and is aligned.
// ------------------------------------------------------------------

static void test_large(void) {
    size_t n = 16 * 1024;                 // 16 KiB
    unsigned char *p = alloca(n);
    assert(p != NULL);
    assert(((uintptr_t)p % _Alignof(max_align_t)) == 0);
    for (size_t i = 0; i < n; i += 4096) p[i] = 0x55;
    for (size_t i = 0; i < n; i += 4096) assert(p[i] == 0x55);
}

// ------------------------------------------------------------------
// 17. Store pointers in the alloca'd region (pointer array)
// ------------------------------------------------------------------

static void test_pointer_array(void) {
    int **pp = alloca(4 * sizeof(int *));
    for (int i = 0; i < 4; i++) {
        pp[i] = alloca(sizeof(int));
        *pp[i] = i * 11;
    }
    for (int i = 0; i < 4; i++) {
        assert(pp[i] != NULL);
        assert(*pp[i] == i * 11);
    }
}

// ------------------------------------------------------------------
// 18. Use alloca as function argument (temporary buffer).
// ------------------------------------------------------------------

static size_t sum_bytes(const unsigned char *p, size_t n) {
    size_t s = 0;
    for (size_t i = 0; i < n; i++) s += p[i];
    return s;
}

static void test_temp_buffer(void) {
    unsigned char *p = alloca(10);
    for (int i = 0; i < 10; i++) p[i] = (unsigned char)i;
    assert(sum_bytes(p, 10) == 45);
}

// ------------------------------------------------------------------
// 19. Building a linked list on the stack (alloca'd nodes)
// ------------------------------------------------------------------

struct node { int v; struct node *next; };

static void test_linked_list(void) {
    struct node *head = NULL;
    for (int i = 4; i >= 0; i--) {
        struct node *n = alloca(sizeof *n);
        n->v = i;
        n->next = head;
        head = n;
    }
    int expect = 0;
    for (struct node *it = head; it; it = it->next) {
        assert(it->v == expect++);
    }
    assert(expect == 5);
}

// ------------------------------------------------------------------
// 20. Alignment of specific types: alloca result must be usable as
//     any type including long double, double, and large-aligned ones.
// ------------------------------------------------------------------

static void test_specific_alignment(void) {
    long double *ld = alloca(sizeof(long double));
    *ld = 3.14L;
    assert(((uintptr_t)ld % _Alignof(long double)) == 0);
    assert(*ld > 3.13L && *ld < 3.15L);

    double *d = alloca(sizeof(double));
    *d = 2.5;
    assert(((uintptr_t)d % _Alignof(double)) == 0);
    assert(*d == 2.5);

    _Alignas(16) char *buf = alloca(32);
    assert(((uintptr_t)buf % 16) == 0);
    memset(buf, 0, 32);
}

// ------------------------------------------------------------------
// main
// ------------------------------------------------------------------

int main(void) {
    test_basic();
    test_alignment();
    test_types();
    test_zero();
    test_variable_size(0);
    test_variable_size(1);
    test_variable_size(7);
    test_variable_size(100);
    test_no_overlap();
    test_lifetime_block_exit();
    test_loop();
    test_nested_call();
    test_vla_interaction();
    test_sizeof();
    test_not_zeroed();
    test_string();
    test_mem_ops();
    test_recursion();
    test_large();
    test_pointer_array();
    test_temp_buffer();
    test_linked_list();
    test_specific_alignment();
    return 0;
}
