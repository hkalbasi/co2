//@ mode: c
//@ run-status: 0

#include <stddef.h>

size_t strlen_func(const char*) {
    return 5;
}

typeof(strlen_func) strlen;

int main() {
    if (strlen("foo") != 3) {
        return 1;
    }
    return 0;
}
