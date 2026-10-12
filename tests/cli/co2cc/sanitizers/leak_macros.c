// Sanitizer predefined-macro check for -fsanitize=leak (see main.nu).
//
// Usage: must FAIL to compile without -fsanitize=leak, must PASS with
// `co2cc -fsanitize=leak`, mirroring sanitizer_macros.c.
#ifndef __SANITIZE_LEAK__
#error "__SANITIZE_LEAK__ not defined (expected with -fsanitize=leak)"
#endif

int main(void)
{
    return 0;
}
