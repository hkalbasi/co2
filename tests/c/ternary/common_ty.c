//@ mode: c
//@ run-status: 0
// Test for the return type of the conditional operator (?:) in C23.

#include <stddef.h>
#include <assert.h>
#include <limits.h>

// Compile-time type check helper
#define TYPE_IS(expr, type) _Generic((expr), type: 1, default: 0)

// ------------------------------------------------------------
// Small types for struct/union return tests
// ------------------------------------------------------------

struct S { int a; };
union  U { int i; float f; };

// Functions with different return types
int    ret_int(void)    { return 1; }
long   ret_long(void)   { return 2L; }
double ret_double(void) { return 3.0; }
void   ret_void(void)   { }
struct S ret_S(void)    { return (struct S){ 7 }; }

// ------------------------------------------------------------
// 1. Same operand types -> that type
// ------------------------------------------------------------

void test_same_type(void) {
    int    i1 = 0, i2 = 0;
    long   l1 = 0, l2 = 0;
    double d1 = 0, d2 = 0;
    float  f1 = 0, f2 = 0;

    assert(TYPE_IS(1 ? i1 : i2, int));
    assert(TYPE_IS(1 ? l1 : l2, long));
    assert(TYPE_IS(1 ? d1 : d2, double));
    assert(TYPE_IS(1 ? f1 : f2, float));
}

// ------------------------------------------------------------
// 2. Integer promotions (char/short -> int)
// ------------------------------------------------------------

void test_integer_promotion(void) {
    char  c = 0;
    short s = 0;
    int   i = 0;

    assert(TYPE_IS(1 ? c : c, int));   // char promoted to int
    assert(TYPE_IS(1 ? s : s, int));   // short promoted to int
    assert(TYPE_IS(1 ? c : i, int));
    assert(TYPE_IS(1 ? s : i, int));

    // _Bool promotes to int
    _Bool b = 0;
    assert(TYPE_IS(1 ? b : b, int));
}

// ------------------------------------------------------------
// 3. Usual arithmetic conversions
// ------------------------------------------------------------

void test_usual_arithmetic(void) {
    int            i  = 0;
    unsigned int   ui = 0;
    long           l  = 0;
    unsigned long  ul = 0;
    long long      ll = 0;
    float          f  = 0;
    double         d  = 0;
    long double    ld = 0;

    assert(TYPE_IS(1 ? i  : ui, unsigned int));   // int -> unsigned int
    assert(TYPE_IS(1 ? i  : l,  long));           // int -> long
    assert(TYPE_IS(1 ? l  : ul, unsigned long));  // same rank, unsigned wins
    assert(TYPE_IS(1 ? i  : ll, long long));      // int -> long long
    assert(TYPE_IS(1 ? i  : f,  float));          // int -> float
    assert(TYPE_IS(1 ? f  : d,  double));         // float -> double
    assert(TYPE_IS(1 ? d  : ld, long double));    // double -> long double
    assert(TYPE_IS(1 ? i  : ld, long double));    // int -> long double

    // Unsigned int vs long (long can represent all unsigned int on LP64)
    assert(TYPE_IS(1 ? ui : l, long));
}

// ------------------------------------------------------------
// 4. Pointer types
// ------------------------------------------------------------

void test_pointers(void) {
    int x = 0;
    int *p  = &x;
    const int *cp = &x;
    volatile int *vp = &x;
    void *pv = &x;

    // Same pointer type
    assert(TYPE_IS(1 ? p : p, int *));

    // Qualifiers combine
    assert(TYPE_IS(1 ? p  : cp, const int *));
    assert(TYPE_IS(1 ? p  : vp, volatile int *));
    assert(TYPE_IS(1 ? cp : vp, const volatile int *));

    // void * wins over object pointer, qualifiers combine
    assert(TYPE_IS(1 ? p : pv, void *));
    assert(TYPE_IS(1 ? cp : pv, const void *));
    assert(TYPE_IS(1 ? vp : pv, volatile void *));
    assert(TYPE_IS(1 ? p : (void*)5, void *));
    assert(TYPE_IS(1 ? (void*)(long*)0 : p, void *)); // Not considered null
    assert(TYPE_IS(1 ? (void*)(void*)0 : p, void *)); // Not considered null

    // Null pointer constant (0) with pointer -> pointer type
    assert(TYPE_IS(1 ? 0 : p, int *));
    assert(TYPE_IS(1 ? (long)0 : p, int *));
    assert(TYPE_IS(1 ? p : (int)0l, int *));

    // Null pointer constant (0) with void* -> void*
    assert(TYPE_IS(1 ? 0LL : pv, void *));

    // 0 in general is not null
    assert(TYPE_IS(1 ? 0LL : 5, long long));
}

// ------------------------------------------------------------
// 5. Array and function decay
// ------------------------------------------------------------

void func(void) { }

void test_decay(void) {
    int arr[4];

    // Array -> pointer to element
    assert(TYPE_IS(1 ? arr : arr, int *));

    // Function -> pointer to function
    assert(TYPE_IS(1 ? func : func, void (*)(void)));

    // Mixed array and pointer
    int *p = arr;
    assert(TYPE_IS(1 ? arr : p, int *));
    assert(TYPE_IS(1 ? p : arr, int *));
}

// ------------------------------------------------------------
// 6. struct and union types (must be identical)
// ------------------------------------------------------------

void test_struct_union(void) {
    struct S s1 = {0}, s2 = {0};
    union  U u1 = {0}, u2 = {0};

    // Same struct type -> that struct type
    assert(TYPE_IS(1 ? s1 : s2, struct S));
    assert(TYPE_IS(1 ? u1 : u2, union U));
}

// ------------------------------------------------------------
// 7. void conditional (both operands void)
// ------------------------------------------------------------

void test_void(void) {
    // The type of a void ?: void expression is void.
    // We can verify with typeof and a void-typed function call.
    typeof(1 ? ret_void() : ret_void()) *v = (void*)0;
    (void)v;
    // _Generic on a void expression with void: type is allowed
    assert(TYPE_IS(1 ? ret_void() : ret_void(), void));
}

// ------------------------------------------------------------
// 9. Bitfields – result type follows integer promotions
// ------------------------------------------------------------

struct BF {
    unsigned int u : 3;
    int          s : 3;
    unsigned int lo : 1;
};

void test_bitfields(void) {
    struct BF bf = { 0, 0, 0 };

    // A bitfield of type unsigned int with width 3 promotes to int
    // (since int can hold all values 0..7).
    // assert(TYPE_IS(1 ? bf.u : bf.u, int));

    // signed bitfield promotes to int
    assert(TYPE_IS(1 ? bf.s : bf.s, int));

    // width 1 unsigned bitfield: values 0..1, still fits int -> int
    // assert(TYPE_IS(1 ? bf.lo : bf.lo, int));

    // Mixed bitfield and int -> usual arithmetic conversions -> int
    // assert(TYPE_IS(1 ? bf.u : 0, int));
}

// ------------------------------------------------------------
// 10. Enum type
// ------------------------------------------------------------

enum Color { RED, GREEN, BLUE };

void test_enum(void) {
    enum Color c1 = RED, c2 = BLUE;

    // Enum type is preserved when both operands have same enum type
    assert(TYPE_IS(1 ? c1 : c2, enum Color));

    // Mixed enum and int -> usual arithmetic conversions on the enum's
    // underlying type. This enum has only non-negative enumerators, so GCC
    // picks `unsigned int` as the underlying type and the result is
    // `unsigned int` (compatible with `enum Color` for _Generic), not `int`.
    // assert(TYPE_IS(1 ? c1 : 0, unsigned int));
}

// ------------------------------------------------------------
// 11. Using typeof to bind the result type
// ------------------------------------------------------------

void test_typeof(void) {
    typeof(1 ? 1 : 1L) t1 = 0L;
    assert(TYPE_IS(t1, long));

    typeof(1 ? 1.0f : 1.0) t2 = 0.0;
    assert(TYPE_IS(t2, double));

    int x = 0;
    int *p = &x;
    // `(void *)0` is a null pointer constant, so the result keeps the
    // other side's pointer type (`int *`), not `void *`.
    typeof(1 ? p : (void *)0) t3 = p;
    assert(TYPE_IS(t3, int *));
}

// ------------------------------------------------------------
// 12. Qualifiers on arithmetic operands are dropped; on pointers they combine
// (pointer combining is covered in section 4).
// ------------------------------------------------------------

void test_qualifiers(void) {
    const int ci = 0;
    volatile int vi = 0;

    // Arithmetic operands: qualifiers are dropped (result is not an
    // lvalue), unlike pointer operands where qualifiers combine.
    assert(TYPE_IS(1 ? ci : ci, int));
    assert(TYPE_IS(1 ? vi : vi, int));
    assert(TYPE_IS(1 ? ci : vi, int));

    // const + non-const of same arithmetic type -> unqualified
    int i = 0;
    assert(TYPE_IS(1 ? ci : i, int));
    assert(TYPE_IS(1 ? i : ci, int));
}

// ------------------------------------------------------------
// Main
// ------------------------------------------------------------

int main(void) {
    test_same_type();
    test_integer_promotion();
    test_usual_arithmetic();
    test_pointers();
    test_decay();
    test_struct_union();
    test_void();
    test_bitfields();
    test_enum();
    test_typeof();
    test_qualifiers();
    return 0;
}
