//@ mode: c
//@ run-status: 0

#include <stddef.h>

typedef struct {
    char f0;
    int f1;
    void* f2;
    int f3;
} BitField;

int main() {
    if (offsetof(BitField, f0) < offsetof(BitField, f1)
        && offsetof(BitField, f1) < offsetof(BitField, f2)
        && offsetof(BitField, f2) < offsetof(BitField, f3)) {
        return 0;
    }
    return 1;
}
