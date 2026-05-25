//@ mode: c
//@ compile-fail
#include "missing_file.h"
       //^^^^^^^^^^^^^^^^ error: missing_file.h: No such file or directory

int main() {
    driven_by_error; // Don't report this, it is probably going to be fixed with the include.
}
