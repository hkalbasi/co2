#include <stdio.h>

int add42(int x);

int main(void) {
    int value = add42(7);
    printf("%d\n", value);
    return value == 49 ? 0 : 1;
}
