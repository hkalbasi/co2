//@ mode: c
//@ run-status: 0

#include <stddef.h>

#define BUILD_ASSERT_OR_ZERO(cond) \
        (sizeof(char [1 - 2*!(cond)]) - 1)

#define BARF_UNLESS_AN_ARRAY(arr) \
        BUILD_ASSERT_OR_ZERO(!__builtin_types_compatible_p(__typeof__(arr), \
                                                           __typeof__(&(arr)[0])))

#define ARRAY_SIZE(x) (sizeof(x) / sizeof((x)[0]) + BARF_UNLESS_AN_ARRAY(x))

void test_func() {
    const char *paths[2];
    char *to_free[ARRAY_SIZE(paths)] = { 0 };
    int i;
    for (i = 0; i < (int)ARRAY_SIZE(to_free); i++) {
        (void)to_free[i];
    }
}

struct object_id {
    unsigned char hash[32];
    unsigned int algo;
};

int main() {
    BUILD_ASSERT_OR_ZERO(offsetof(struct object_id, hash) <
                         offsetof(struct object_id, algo));
    test_func();
    return 0;
}
