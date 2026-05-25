//@ mode: c

#include <math.h>

int main(void) {
    if (!signbit(-1.0)) {
        return 1;
    }
    return signbit(1.0) ? 2 : 0;
}
