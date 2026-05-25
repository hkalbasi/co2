//@ mode: c
//@ run-status: 0

#include <stdint.h>

constexpr int global = 42;
constexpr int64_t global2 = 0xbcdabcdabcdabcd;
constexpr char global3[] = "hello";
static int global_copy = global;
int global_arr[global];
constexpr int *global_null = 0;

int main(void) {
    constexpr int local = 4;
    int local_arr[local];
    static int local_copy = local;
    const int *local_addr = &local;

    local_arr[0] = 1;

    switch (46) {
    case local + global:
        break;
    default:
        return 1;
    }

    if (&global == 0)
        return 2;
    if (*local_addr != 4)
        return 3;
    if (local_copy != 4)
        return 4;
    if (global_copy != 42)
        return 5;
    if (sizeof(global_arr) / sizeof(global_arr[0]) != 42)
        return 6;
    if (global_null != 0)
        return 7;
    if (local_arr[0] != 1)
        return 8;
    if (global2 < 1e12) {
        return 9;
    }
    if (global3[0] != 'h' || global3[1] != 'e' || global3[4] != 'o' || global3[5] != '\0') {
        return 10;
    }
    return 0;
}
