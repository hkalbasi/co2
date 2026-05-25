//@ mode: c
//@ run-status: 0

#include <stdbool.h>

int main(void) {
    bool has_year = true;
    bool has_mon = false;
    bool has_time = true;
    int num_index = 1;

    return (has_year + has_mon + has_time + num_index) == 3 ? 0 : 1;
}
