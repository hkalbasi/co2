//@ mode: c
//@ run-status: 0

/* Lua's loadlib.c uses a double __extension__ cast to convert the void*
 * returned by dlsym() to a C function pointer:
 *
 *   #define cast_func(p)   (__extension__ (voidf)(p))
 *   #define cast_Lfunc(p)  ((lua_CFunction)(cast_func(p)))
 *
 * When dlsym() returns NULL the resulting function pointer must compare
 * equal to NULL.  A co2cc bug caused LLVM to tag the intermediate Rust
 * fn() value as nonnull, so the comparison was optimised to always-false.
 *
 * The test uses a volatile sink to prevent the compiler from folding away
 * the NULL at compile time. */

typedef void (*voidf)(void);
typedef int (*FnPtr)(void *);

/* Exact macro spelling from Lua's llimits.h / loadlib.c */
#define cast_func(p)   (__extension__ (voidf)(p))
#define cast_Lfunc(p)  ((FnPtr)(cast_func(p)))

int main(void) {
    /* volatile prevents compile-time constant-folding of the NULL. */
    void * volatile sink = (void *)0;
    void *raw = sink;

    FnPtr f = cast_Lfunc(raw);

    /* f must be NULL: return 0 on success, 1 on failure. */
    return f != (FnPtr)0;
}
