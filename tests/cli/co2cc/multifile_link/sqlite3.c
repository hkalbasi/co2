#include <math.h>

static double common_function() {
    return 2.0;
}

double sqlite3_magic(double value) {
    return floor(value * common_function());
}
