//@ mode: c
//@ run-status: 0

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
