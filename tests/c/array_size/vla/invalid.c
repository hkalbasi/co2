//@ mode: c
//@ compile-fail
//
// VLA constraints from C11 that CO2 must reject:
//   6.2.4p6    an object with variably modified type must have no linkage
//   6.5.2.5p2  a compound literal cannot have VLA type
//   6.7.2.1p9  a struct/union member cannot have variably modified type
//   6.7.6.2    array declarator constraints ([*], static/qualifiers, integer bound)
//   6.7.9p3    a VLA cannot be initialized
//   6.8.6.1p1  no jump into the scope of a VLA
//
// Cases marked "currently accepted" compile today and miscompile instead of
// failing: they are the reason this test is red.

// ------------------------------------------------------- storage / linkage

void vla_static_storage(int n) {
    static int a[n];
    //         ^^^^ error: variable length array cannot have static storage duration
}

void vla_linkage(int n) {
    extern int a[n];
    //         ^^^^ error: variable length array cannot have linkage
}

// ---------------------------------------------- initialization / assignment

void vla_initialized(int n) {
    int a[n] = {0}; // currently accepted
    //  ^^^^ error: variable length array cannot be initialized
}

void vla_assignment(int n) {
    int a[n], b[n];
    a = b; // currently accepted
    //  ^ error: array type is not assignable
}

int vla_return(void) {
    int n = 3;
    int a[n];
    return a; // currently accepted
    //     ^ error: cannot return a variable length array
}

// --------------------------------------------------------------- members

void vla_struct_member(int n) {
    struct S { int m[n]; }; // currently accepted
    //             ^^^^ error: variable length array cannot be a struct member
}

void vla_union_member(int n) {
    union U { int m[n]; };
    //            ^^^^ error: variable length array cannot be a union member
}

// -------------------------------------------------------- goto into scope

void vla_goto_into_scope(int n) {
    goto skip;
    int a[n]; // currently accepted
    //  ^^^^ error: jump into scope of variable length array
skip:
    ;
}

// -------------------------------------------------- constant expressions

void vla_static_assert(int n) {
    int a[n];
    _Static_assert(sizeof(a) == 20, "");
    //             ^^^^^^^^^ error: static assertion expression is not a constant expression
}

void vla_enum(int n) {
    int a[n];
    enum { E = sizeof(a) };
    //         ^^^^^^^^^ error: enumerator value is not an integer constant expression
}

void vla_case(int n) {
    int a[n];
    switch (n) {
    case sizeof(a): // currently accepted
    //   ^^^^^^^^^ error: case label is not an integer constant expression
        break;
    }
}

void vla_static_init(int n) {
    int a[n];
    static int s = sizeof(a);
    //             ^^^^^^^^^ error: initializer is not a constant expression
}

void vla_bitfield_width(int n) {
    int a[n];
    struct B { unsigned b : sizeof(a); };
    //                      ^^^^^^^^^ error: bit-field width is not an integer constant expression
}

// ---------------------------------------------------------- element types

void vla_of_void(int n) {
    void a[n]; // currently accepted
    //   ^^^^ error: array element type cannot be void
}

struct Incomplete;
void vla_of_incomplete(int n) {
    struct Incomplete a[n];
    //                ^^^^ error: array element type must be complete
}

void vla_of_function(int n) {
    int a[n](void);
    //  ^^^^^^^^^^ error: array element type cannot be a function type
}

// -------------------------------------------------------------- bound type

void vla_float_bound(void) {
    double d = 3.0;
    int a[d]; // currently accepted
    //  ^^^^ error: array size must have integer type
}

void vla_pointer_bound(void) {
    int *p = 0;
    int a[p]; // currently accepted
    //  ^^^^ error: array size must have integer type
}

// ------------------------------------------------------- declarator syntax

void vla_star_local(int n) {
    int a[*];
    //   ^^^ error: `[*]` is only allowed in a function prototype
}

void vla_star_definition(int n, int a[*]) { // currently accepted
    //                               ^^^ error: `[*]` is only allowed in a function prototype
    (void)a;
}

void vla_star_pointer(void) {
    int (*p)[*]; // currently accepted
    //      ^^^ error: `[*]` is only allowed in a function prototype
    (void)p;
}

void vla_static_local(void) {
    int a[static 3]; // currently accepted
    //   ^^^^^^^^^^ error: `static` and type qualifiers are only allowed in parameter array declarators
}

// ------------------------------------------------------ compound literals

void vla_compound_literal(int n) {
    (void)(int[n]){0}; // currently accepted
    //    ^^^^^^^^ error: compound literal cannot have variable length array type
}

// --------------------------------------------------------------- type errors

void vla_pointer_type_mismatch(int n) {
    int i;
    int (*p)[n] = &i; // currently accepted
    //            ^^ error: mismatched types
    (void)p;
}
