//@ mode: c

#include <signal.h>

int main(void) {
    return ((sighandler_t)0) == (sighandler_t)0 ? 0 : 1;
}
