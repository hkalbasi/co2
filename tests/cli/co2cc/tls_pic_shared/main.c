#include <stdio.h>

int get_tls(void);
void set_tls(int v);

int main(void) {
    if (get_tls() != 42) {
        return 1;
    }
    set_tls(7);
    if (get_tls() != 7) {
        return 2;
    }
    printf("tls-ok\n");
    return 0;
}
