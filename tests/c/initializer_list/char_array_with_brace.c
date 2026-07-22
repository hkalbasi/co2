//@ mode: c
//@ run-status: 0

#include <string.h>
#include <assert.h>

char foo[] = { "xxxx" };
char bar[3] = { "yy" };

int main() {
    assert(strlen(foo) == 4);
    assert(strlen(bar) == 2);

    return 0;
}
