//@ mode: c
//@ compile-fail

#include <stddef.h>

struct S { int x; int y; };

#define GET_ENTRY(ptr, member) \
    ((void *)((char *)(ptr) + offsetof(struct S, member)))

void f1(void)
{
    struct S s;
    void *p = GET_ENTRY(&s, zzzz); int x = 2;
                          //^^^^ error: offsetof: field 'zzzz' not found in type
}

#define FIELD_NAME(x) field_##x

void f2(void)
{
    struct S s;
    void *p = GET_ENTRY(&s, FIELD_NAME(zzzz)); int x = 2;
                          //^^^^^^^^^^^^^^^^ error: offsetof: field 'field_zzzz' not found in type
}
