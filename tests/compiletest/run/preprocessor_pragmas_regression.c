//@ mode: c
//@ run-status: 0

#include <stdalign.h>

#include "preprocessor_pragmas_regression_once.h"
#include "preprocessor_pragmas_regression_once.h"

#define FLAG 4

#if defined FLAG
int defined_without_parens = FLAG;
#else
#error "defined FLAG should be true"
#endif

#if defined(FLAG)
int defined_with_parens = FLAG + 1;
#else
#error "defined(FLAG) should be true"
#endif

#if FLAG
int object_like_condition = 1;
#else
#error "FLAG should be truthy in #if"
#endif

#define VALUE 11
#pragma push_macro("VALUE")
#undef VALUE
#define VALUE 29
int pushed_value = VALUE;
#pragma pop_macro("VALUE")
int popped_value = VALUE;

#pragma pack(push)
#pragma pack(2)
struct Packed2 {
    char a;
    int b;
};
#pragma pack()
struct ResetPack {
    char a;
    int b;
};
#pragma pack(push, 1)
struct Packed1 {
    char a;
    int b;
};
#pragma pack(pop)
struct RestoredPack {
    char a;
    int b;
};

int counter_values[] = {__COUNTER__, __COUNTER__, __COUNTER__};

int main(void) {
    if (once_helper() != 1) {
        return 1;
    }
    if (defined_without_parens != 4 || defined_with_parens != 5 || object_like_condition != 1) {
        return 2;
    }
    if (pushed_value != 29 || popped_value != 11) {
        return 3;
    }
    if (alignof(struct Packed1) != 1 || sizeof(struct Packed1) != 5) {
        return 4;
    }
    if (alignof(struct Packed2) != 2 || sizeof(struct Packed2) != 6) {
        return 5;
    }
    if (alignof(struct ResetPack) != alignof(int) || sizeof(struct ResetPack) != 8) {
        return 6;
    }
    if (alignof(struct RestoredPack) != alignof(int) || sizeof(struct RestoredPack) != 8) {
        return 7;
    }
    if (counter_values[0] + 1 != counter_values[1] || counter_values[1] + 1 != counter_values[2]) {
        return 8;
    }
    return 0;
}
