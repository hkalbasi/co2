//@ mode: c
//@ run-status: 0

#include <assert.h>
#include <math.h>

int main() {
    assert(1);
    assert(2 == 2);
    assert(INFINITY > 5.2);
    assert(INFINITY == INFINITY);
    assert(NAN != NAN);
    return 0;
}
