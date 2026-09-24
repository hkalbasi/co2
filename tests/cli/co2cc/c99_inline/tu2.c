#include "myinline.h"

inline int myinlinefn2(int);

int helper(void) {
    return myinlinefn(1) + myinlinefn2(2);
}
