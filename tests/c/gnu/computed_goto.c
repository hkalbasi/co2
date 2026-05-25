//@ mode: c
//@ run-status: 0

static int dispatch(int opcode) {
    static const void *const table[] = {
        &&case_zero,
        &&case_one,
        &&case_default,
    };

    goto *table[opcode];

case_zero:
    return 10;
case_one:
    return 11;
case_default:
    return 12;
}

int main(void) {
    if (dispatch(0) != 10) {
        return 1;
    }
    if (dispatch(1) != 11) {
        return 2;
    }
    if (dispatch(2) != 12) {
        return 3;
    }
    return 0;
}
