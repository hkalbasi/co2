//@ mode: c
//@ run-status: 0

int first(int buf[static 4]) {
    return buf[0];
}

int last_qualified(int buf[static const restrict 4]) {
    return buf[3];
}

typedef int Row[4];

int matrix_middle(Row rows[static 1]) {
    return rows[0][2];
}

int main(void) {
    int values[4] = {1, 2, 3, 4};
    Row rows[1] = {{7, 8, 9, 10}};
    if (first(values) != 1) {
        return 1;
    }
    if (last_qualified(values) != 4) {
        return 2;
    }
    if (matrix_middle(rows) != 9) {
        return 3;
    }
    return 0;
}
