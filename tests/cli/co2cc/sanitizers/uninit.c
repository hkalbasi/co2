// MemorySanitizer check for -fsanitize=memory (see main.nu).
//
// Usage: branching on an uninitialized heap value must abort with a
// MemorySanitizer report (nonzero exit) with -fsanitize=memory.
#include <stdlib.h>
#include <stdio.h>

int main(void)
{
    int *p = (int *)malloc(sizeof(int));
    if (*p == 42)
        printf("42\n");
    else
        printf("other\n");
    free(p);
    return 0;
}
