//@ mode: c
//@ compile-fail

#define BUILD_ASSERT_OR_ZERO(cond) \
    (sizeof(char [1 - 2 * !(cond)]) - 1)

void f(void)
{
    int xs[BUILD_ASSERT_OR_ZERO(0)];
         //^^^^^^^^^^^^^^^^^^^^^^^ error: array size must be a non-negative integer, got -1
}
