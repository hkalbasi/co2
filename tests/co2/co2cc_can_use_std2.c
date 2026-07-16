//@ mode: c
//@ run-status: 0

use std::vec::Vec;

#include <assert.h>

int main() {
    auto x = Vec::<i32>::new();
    x.push(4);
    assert(x.len() == 1);

    return 0;
}
