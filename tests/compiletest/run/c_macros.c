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
    assert(isinf(INFINITY));
    assert(isinf(1./0.));
    assert(isinf(-1./0.));
    assert(!isinf(NAN));
    assert(!isinf(2.3));
    assert(isnan(NAN));
    assert(!isnan(INFINITY));
    assert(!isnan(2.3));
    return 0;
}
