//@ mode: c
//@ run-status: 0

int first(int buf[static 4]) {
    return buf[0];
}

int main(void) {
    int values[4] = {1, 2, 3, 4};
    return first(values) - 1;
}
