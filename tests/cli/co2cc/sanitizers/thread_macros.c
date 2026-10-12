// Sanitizer predefined-macro check for -fsanitize=thread (see main.nu).
//
// Usage: must FAIL to compile without -fsanitize=thread, must PASS with
// `co2cc -fsanitize=thread`, mirroring sanitizer_macros.c.
#ifndef __SANITIZE_THREAD__
#error "__SANITIZE_THREAD__ not defined (expected with -fsanitize=thread)"
#endif

int main(void)
{
    return 0;
}
