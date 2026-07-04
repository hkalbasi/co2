//@ mode: c
//@ compile-fail
//@ compile-flags: -DFOO=66 -DBAR

#ifndef FOO
    #error "FOO is not present"
#endif
#ifndef BAR
    #error "BAR is not present"
#endif
#ifndef BAZ
    #error "BAZ is not present"
//   ^^^^^ error: #error "BAZ is not present"
#endif

int main(void) {}
