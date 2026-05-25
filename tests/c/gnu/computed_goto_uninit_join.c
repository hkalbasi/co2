//@ mode: c

#include <stdlib.h>

static int dispatch(int opcode) {
    static const void *const table[] = {
        &&case_zero,
        &&case_one,
        &&case_default,
    };
    int value;

    goto *table[opcode];

case_zero:
    value = 10;
    goto done;
case_one:
    value = 11;
    goto done;
case_default:
    abort();

done:
    return value;
}

int main(void) {
    if (dispatch(0) != 10) {
        return 1;
    }
    if (dispatch(1) != 11) {
        return 2;
    }
    return 0;
}
