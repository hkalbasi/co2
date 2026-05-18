#include <math.h>

static double common_function() {
    return 2.0;
}

double helper_magic(double value) {
    return floor(value * common_function());
}
