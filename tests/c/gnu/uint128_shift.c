//@ mode: c
//@ run-status: 0

#include <assert.h>

int main(void) {
    unsigned __int128 hi = (unsigned __int128)1 << 100;
    assert((unsigned long long)(hi >> 100) == 1ull);
    assert((unsigned long long)hi == 0ull);
    assert((unsigned long long)(hi >> 64) == (1ull << 36));

    unsigned __int128 one = 1;
    assert((unsigned long long)((one << 100) >> 100) == 1ull);

    unsigned __int128 full = ~(unsigned __int128)0;
    assert((full >> 127) == 1);
    assert(((full << 64) >> 64) == (unsigned __int128)0xffffffffffffffffull);

    // Signed shift still sign-extends.
    __int128 neg = -1;
    assert((neg >> 120) == -1);

    return 0;
}
