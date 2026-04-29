//@ mode: c

static int sum_pair(int a, int b) {
    int args[] = { a, b };
    return args[0] + args[1];
}

int main(void) {
    return sum_pair(1, 2) == 3 ? 0 : 1;
}
