//@ mode: c
//@ run-status: 0

#include <stddef.h>

typedef size_t strlen_func(const char*);

strlen_func strlen;

int main() {
    if (strlen("foo") != 3) {
        return 1;
    }
    return 0;
}
