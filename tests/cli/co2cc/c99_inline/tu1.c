#include "myinline.h"

int helper(void);

int main(void) {
    if (myinlinefn(20) != 41) {
        return 1;
    }
    if (helper() != 10) {
        return 2;
    }
    if (myinlinefn2(20) != 61) {
        return 3;
    }
    return 0;
}
