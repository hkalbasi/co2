//@ mode: c
//@ run-status: 0
//@ requires-header: stdcountof.h

#include <assert.h>
#include <stdcountof.h>

int arr[10];
int arr2[10][3];
int m = countof(arr);

typedef int arr4_t[4];

int sidefx_counter = 0;

int sidefx(void) {
    sidefx_counter++;
    return 1;
}

int main(void) {
    // ------------------------------------------------------------
    // basic expression form (macro + keyword)
    // ------------------------------------------------------------

    assert(countof(arr) == 10);
    assert(_Countof(arr) == 10);
    assert(_Countof arr == 10);
    assert(m == 10);

    // ------------------------------------------------------------
    // type-name form yields first dimension
    // ------------------------------------------------------------

    assert(_Countof(int[5]) == 5);
    assert(countof(int[7][3]) == 7);
    assert(_Countof(arr4_t) == 4);

    // ------------------------------------------------------------
    // multidimensional arrays
    // ------------------------------------------------------------

    int md[12][7] = {{0}};
    assert(_Countof(md) == 12);
    assert(_Countof(*md) == 7);
    assert(countof(md[0]) == 7);

    // ------------------------------------------------------------
    // typedef, const, and struct-member arrays
    // ------------------------------------------------------------

    arr4_t t = {0};
    assert(countof(t) == 4);

    const int carr[6] = {0};
    assert(_Countof(carr) == 6);

    struct Holder {
        int vals[3];
        long other;
    } h = {{0}, 0};
    assert(countof(h.vals) == 3);

    // ------------------------------------------------------------
    // string literals are arrays (include the NUL)
    // ------------------------------------------------------------

    assert(_Countof("abc") == 4);

    // ------------------------------------------------------------
    // constant-expression context
    // ------------------------------------------------------------

    _Static_assert(_Countof(arr) == 10, "");
    _Static_assert(countof(int[7][3]) == 7, "");
    _Static_assert(countof(arr4_t) == 4, "");
    int sized[countof(arr4_t)] = {0};
    assert(countof(sized) == 4);

    // ------------------------------------------------------------
    // unevaluated operand: side effects must not execute
    // ------------------------------------------------------------

    sidefx_counter = 0;
    int n = _Countof(arr2[sidefx()]);
    assert(n == 3);
    if (sidefx_counter != 0)
        return 1;

    return 0;
}
