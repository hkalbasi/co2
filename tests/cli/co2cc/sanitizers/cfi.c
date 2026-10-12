// CFI check for -flto -fsanitize=cfi (see main.nu).
//
// Usage: must EXIT 0 without the flag (the mismatched call is plain UB
// there), must TRAP (SIGILL, nonzero exit, no output) with -flto
// -fsanitize=cfi.
typedef int (*binop)(int, int);
typedef int (*nulop)(void);

int target(int a, int b)
{
    return a + b;
}

int main(void)
{
    binop f = target;
    nulop g = (nulop)f;
    volatile int r = g();
    (void)r;
    return 0;
}
