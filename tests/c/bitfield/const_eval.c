//@ mode: c
//@ run-status: 0

enum {
    E = 3,
};

struct S {
    unsigned a : 2 * 4;
    unsigned b : (1 + 2) * (3 + 4);
    unsigned c : (4321 == 4321);
    unsigned d : (4321 != 1234) + 4;
    unsigned e : (3 < 4) * 5;
    unsigned f : (5 > 3) ? 7 : 1;
    unsigned g : (0 ? 1 : 9);
    unsigned h : (sizeof(int) == sizeof(int)) * 6;
    unsigned i : 8 * sizeof(unsigned);
    unsigned j : ((1 + 2) * (3 + 4)) / 3;
    unsigned k : (((5 & 3) | 8) ^ 1);
    unsigned l : (~0u != 0u) ? 11 : 1;
    unsigned m : E * 2;
    unsigned n : (1 && 1) * 13;
    unsigned o : (0 || 1) * 14;
    unsigned p : ((1 << 3) - 1);
};

int main(void) {
    struct S s = {0};

    s.a = (1u << (2 * 4)) - 1;
    s.c = 1;
    s.d = 5;
    s.e = 5;
    s.f = 7;
    s.g = 9;
    s.h = 6;
    s.m = 6;

    if (s.a != ((1u << (2 * 4)) - 1))
        return 1;
    if (s.c != 1)
        return 2;
    if (s.d != 5)
        return 3;
    if (s.e != 5)
        return 4;
    if (s.f != 7)
        return 5;
    if (s.g != 9)
        return 6;
    if (s.h != 6)
        return 7;
    if (s.m != 6)
        return 8;

    return 0;
}
