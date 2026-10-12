// LeakSanitizer check for -fsanitize=leak (see main.nu).
//
// Usage: the 32-byte allocation is never freed, so this must report
// `LeakSanitizer` (nonzero exit) with -fsanitize=leak.
#include <stdlib.h>

int main(void)
{
    char *p = malloc(32);
    p[0] = 'x';
    (void)p;
    return 0;
}
