//@ mode: c

enum { N = 3 };

int main(void) {
    int array[N + 1];
    return sizeof(array) == sizeof(int) * 4 ? 0 : 1;
}
