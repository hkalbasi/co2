//@ mode: c
//@ run-status: 0

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
int main(void) {
    char a[2][16] = { "hello", " " };
    for (int i = 0; i < 2; i++) {
        char *p = 0;
        if (a[i][0] != ' ') p = strdup(a[i]);
        if (p) free(p);
    }
    return 0;
}
