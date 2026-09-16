//@ mode: c
//@ run-status: 0
//@ run-stderr-contains: : foo: bar

#include <error.h>

int main(void){
    error(0, 0, "%s: %s", "foo", "bar");
    return 0;
}
