// ThreadSanitizer check for -fsanitize=thread (see main.nu).
//
// Usage: the two writes to `x` are unordered (no happens-before edge), so
// this must EXIT 0 without the flag and report `data race` (nonzero exit)
// with -fsanitize=thread. Both writes always execute, so the report is
// deterministic.
#include <pthread.h>

int x = 0;

void *worker(void *arg)
{
    (void)arg;
    x = 1;
    return 0;
}

int main(void)
{
    pthread_t t;
    pthread_create(&t, 0, worker, 0);
    x = 2;
    pthread_join(t, 0);
    return 0;
}
