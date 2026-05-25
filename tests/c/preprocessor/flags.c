//@ mode: c
//@ run-status: 0

#ifdef __GNUC__
    #error "co2c is not gcc"
#endif
#ifdef __clang__
    #error "co2c is not clang"
#endif
#ifndef __CO2__
    #error "co2c is co2"
#endif

int main(void) {
    return 0;
}