//@ mode: c
//@ run-status: 0

#include <stddef.h>

typedef size_t strlen_func(const char*);
typedef strlen_func strlen_alias;

strlen_alias strlen;

int main() {
    if (strlen("foo") != 3) {
        return 1;
    }
    return 0;
}
