// Declares but never defines `myinlinefn`: this TU can only link if some
// other TU provides the real external definition (the `extern inline`
// instantiation in tu_ext.c). Per-TU local copies are not enough.
int myinlinefn(int x);
int myinlinefn2(int x);

int main(void) {
    if (myinlinefn(20) != 41) {
        return 1;
    }
    if (myinlinefn2(20) != 61) {
        return 1;
    }
    return 0;
}
