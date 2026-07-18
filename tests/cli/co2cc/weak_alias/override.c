//@ run-status: 0

// Provides a strong (non-weak) definition of `weak_only`, which must win over
// the weak definition in weak.c at link time.
int weak_only(void) {
    return 99;
}
