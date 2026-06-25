//@ mode: c
//@ run-status: 0

#include <stdio.h>
#include <string.h>
/* Simulate tcctok.h DEF expansion in a string initializer */
#define STR(s) s
static const char *keywords[] = {
    STR("sizeof"),
    STR("__attribute"),
    STR("__inline"),
    STR("__inline__"),
    STR("__restrict"),
    STR("__extension__"),
    STR("_Complex"),
    STR("_Noreturn"),
    STR("define"),
};

int main(void) {
    int errors = 0;
    struct { const char *str; int len; } expected[] = {
        {"sizeof", 6},
        {"__attribute", 11},
        {"__inline", 8},
        {"__inline__", 10},
        {"__restrict", 10},
        {"__extension__", 13},
        {"_Complex", 8},
        {"_Noreturn", 9},
        {"define", 6},
    };
    int n = sizeof(expected) / sizeof(expected[0]);
    for (int i = 0; i < n; i++) {
        int actual_len = strlen(keywords[i]);
        int ok = actual_len == expected[i].len
              && memcmp(keywords[i], expected[i].str, actual_len) == 0;
        printf("[%d] \"%s\" (len=%d) %s\n",
               i, keywords[i], actual_len,
               ok ? "OK" : "MISCOMPILED");
        if (!ok) errors++;
    }
    printf("\n%d errors out of %d entries\n", errors, n);
    return errors != 0;
}
