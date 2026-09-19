#include "myinline.h"

int helper(void);

int main(void) {
    if (myinlinefn(20) != 41) {
        return 1;
    }
    if (helper() != 3) {
        return 2;
    }
    return 0;
}
