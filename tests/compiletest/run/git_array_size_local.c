//@ mode: c

/*
 * Reproduction of "missing local type" issue.
 * Triggered by ARRAY_SIZE on a local anonymous struct array.
 */

#define BUILD_ASSERT_OR_ZERO(cond) \
        (sizeof(char [1 - 2*!(cond)]) - 1)

#define BARF_UNLESS_AN_ARRAY(arr) \
        BUILD_ASSERT_OR_ZERO(!__builtin_types_compatible_p(__typeof__(arr), \
                                                           __typeof__(&(arr)[0])))

#define ARRAY_SIZE(x) (sizeof(x) / sizeof((x)[0]) + BARF_UNLESS_AN_ARRAY(x))

typedef int (*command_t)(int x);
int run_status(int x) { return x; }

void test_func() {
    struct {
        const char *string;
        command_t command;
    } command_list[] = {
        { "status", run_status },
    };
    int i;
    for (i = 0; i < (int)ARRAY_SIZE(command_list); i++) {
        (void)command_list[i].string;
    }
}

int main() {
    test_func();
    return 0;
}
