//@ mode: c
//@ run-status: 0

#include <stddef.h>

typedef unsigned char lu_byte;

typedef struct { void *p; } Value;

typedef union {
    struct {
        Value value_;
        lu_byte tt_;
        lu_byte key_tt;
        int next;
        Value key_val;
    } u;
    int i_val;
} Node;

static const Node dummy = {
    {{NULL}, 0x10, 0x0b, 0, {NULL}}
};

int main(void) {
    if (dummy.u.value_.p != NULL) return 1;
    if (dummy.u.tt_ != 0x10) return 2;
    if (dummy.u.key_tt != 0x0b) return 3;
    if (dummy.u.next != 0) return 4;
    if (dummy.u.key_val.p != NULL) return 5;
    return 0;
}
