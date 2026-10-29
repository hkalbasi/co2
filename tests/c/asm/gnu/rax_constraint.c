//@ mode: c
//@ run-status: 0

#include <assert.h>
#include <stdint.h>

// `"a"` (rax family) constraints, x86-64 specific.

// zstd's cpu.h reads CPUID leaf 0 with an output in the "a" register.
unsigned max_cpuid_leaf(void) {
    unsigned n;
    __asm__("cpuid" : "=a"(n) : "a"(0) : "ebx", "ecx", "edx");
    return n;
}

void test_output_widths(void) {
    uint64_t q;
    __asm__("movabsq $0x1122334455667788, %0" : "=a"(q));
    assert(q == 0x1122334455667788ULL);

    unsigned n;
    __asm__("movl $0xdeadbeef, %0" : "=a"(n));
    assert(n == 0xdeadbeefu);

    unsigned short s;
    __asm__("movw $0x1234, %0" : "=a"(s));
    assert(s == 0x1234);

    unsigned char c;
    __asm__("movb $0x5a, %0" : "=a"(c));
    assert(c == 0x5a);
}

void test_inout(void) {
    unsigned v = 5;
    __asm__("addl $3, %0" : "+a"(v));
    assert(v == 8);

    uint64_t w = 1;
    __asm__("addq $2, %0" : "+a"(w));
    assert(w == 3);
}

void test_input_pinned(void) {
    unsigned out;
    __asm__("movl %1, %0" : "=r"(out) : "a"(42u));
    assert(out == 42);

    uint64_t big = 0xfedcba9876543210ULL;
    uint64_t out64;
    __asm__("movq %1, %0" : "=r"(out64) : "a"(big));
    assert(out64 == big);

    unsigned char b = 7;
    unsigned char out8;
    __asm__("movb %1, %0" : "=r"(out8) : "a"(b));
    assert(out8 == 7);
}

int main(void) {
    assert(max_cpuid_leaf() > 0);
    test_output_widths();
    test_inout();
    test_input_pinned();
    return 0;
}
