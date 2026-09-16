//@ mode: c
//@ run-status: 0

int memchr(int x) {
    return x + 1;
}

int main(void) {
    return memchr(41) - 42;
}
