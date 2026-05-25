//@ mode: c

int main(void) {
    _Atomic unsigned int value = 1;
    return __atomic_fetch_add(&value, 2, 5) == 1 ? 0 : 1;
}
