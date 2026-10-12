// Sanitizer predefined-macro check for -fsanitize= (see main.nu).
//
// Usage: must FAIL to compile without -fsanitize=address, must PASS with
// `co2cc -fsanitize=address`, mirroring overflow.c. The #error names the
// missing macro so a wrong -fsanitize -> macro mapping shows up as a
// compile error, not silent miscompilation.
#ifndef __SANITIZE_ADDRESS__
#error "__SANITIZE_ADDRESS__ not defined (expected with -fsanitize=address)"
#endif

int main(void)
{
    return 0;
}
