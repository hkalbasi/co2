//@ mode: c
//@ run-status: 0

#include <stdalign.h>

#pragma pack(push, 1)
struct S1 {
    int x;
    char y;
};
#pragma pack(pop)

int main() {
    struct S1 s1;
    char* y = &s1.y;

    if (sizeof(struct S1) != 5 || alignof(struct S1) != 1) {
        return 1;
    }

    return 0;
}
