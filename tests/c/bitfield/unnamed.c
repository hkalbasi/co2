//@ mode: c
//@ run-status: 0

typedef int foo;

struct S {
    /* ordinary unsigned bit-fields */
    unsigned a : 3;
    unsigned int b : 5;

    /* unnamed padding */
    unsigned : 2;

    /* signed bit-field */
    signed c : 4;

    /* zero-width bit-field */
    unsigned : 0;

    /* more bit-fields after a forced new allocation unit */
    unsigned d : 6;

    /* cv-qualified bit-fields */
    const unsigned e : 3;
    volatile unsigned f : 4;

    /* zero-width bit-field */
    foo : 0;
    foo g: 3;
    signed long int : 5;
    const int h: 4;
};

int main(void) {
    struct S s = {
        .a = 7,
        .b = 31,
        .c = -5,
        .d = 42,
        .e = 5,
        .f = 9,
        .g = -1,
        .h = 3,
    };

    /* Values should round-trip. */
    if (s.a != 7) return 1;
    if (s.b != 31) return 2;
    if (s.c != -5) return 3;
    if (s.d != 42) return 4;
    if (s.e != 5) return 5;
    if (s.f != 9) return 6;
    if (s.g != -1) return 7;
    if (s.h != 3) return 8;

    /* Assignment truncates to the field width. */
    s.a = 15;   /* 15 mod 8 == 7 */
    if (s.a != 7) return 9;

    s.b = 63;   /* 63 mod 32 == 31 */
    if (s.b != 31) return 10;

    s.d = 127;  /* 127 mod 64 == 63 */
    if (s.d != 63) return 11;

    /* Signed values within range remain unchanged. */
    s.c = 7;
    if (s.c != 7) return 12;

    s.c = -8;
    if (s.c != -8) return 13;

    /* Volatile bit-fields remain usable. */
    s.f = 3;
    if (s.f != 3) return 14;

    return 0;
}
