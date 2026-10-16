//@ mode: c
//@ run-status: 0
// Test for __attribute__((naked)) functions and their corner cases.
// GNU extension, x86-64 (System V ABI).
// Compile with: gcc -std=gnu11 -Wall -Wextra -O2 tests/c/gnu_naked.c

#include <stddef.h>
#include <assert.h>
#include <stdint.h>

// ------------------------------------------------------------------
// ABI reminders (x86-64 System V):
//   integer/pointer args: rdi, rsi, rdx, rcx, r8, r9
//   return value:         rax
//   callee-saved:         rbx, rbp, r12-r15
//   caller-saved:         rax, rcx, rdx, rsi, rdi, r8-r11
// The naked function body must be basic asm only — no C statements,
// no extended asm with operands, no local variables, no `return`.
// ------------------------------------------------------------------

// ------------------------------------------------------------------
// 1. Minimal naked function: return a constant
// ------------------------------------------------------------------

__attribute__((naked)) int n01_const(void) {
    __asm__("movl $42, %eax\n\t"
            "ret");
}

void test_const(void) {
    assert(n01_const() == 42);
}

// ------------------------------------------------------------------
// 2. Two integer arguments: return a + b
// ------------------------------------------------------------------

__attribute__((naked)) int n02_add(int a, int b) {
    __asm__("movl %edi, %eax\n\t"
            "addl %esi, %eax\n\t"
            "ret");
}

void test_add(void) {
    assert(n02_add(3, 4) == 7);
    assert(n02_add(-1, 1) == 0);
    assert(n02_add(0, 0) == 0);
}

// ------------------------------------------------------------------
// 3. Three arguments: return a + b + c
// ------------------------------------------------------------------

__attribute__((naked)) int n03_add3(int a, int b, int c) {
    __asm__("movl %edi, %eax\n\t"
            "addl %esi, %eax\n\t"
            "addl %edx, %eax\n\t"
            "ret");
}

void test_add3(void) {
    assert(n03_add3(1, 2, 3) == 6);
    assert(n03_add3(10, 20, 30) == 60);
}

// ------------------------------------------------------------------
// 4. Six integer arguments: return sum (uses all 6 arg registers)
// ------------------------------------------------------------------

__attribute__((naked)) long n04_add6(long a, long b, long c,
                                      long d, long e, long f) {
    __asm__("movq %rdi, %rax\n\t"
            "addq %rsi, %rax\n\t"
            "addq %rdx, %rax\n\t"
            "addq %rcx, %rax\n\t"
            "addq %r8,  %rax\n\t"
            "addq %r9,  %rax\n\t"
            "ret");
}

void test_add6(void) {
    assert(n04_add6(1, 2, 3, 4, 5, 6) == 21);
    assert(n04_add6(-1, -2, -3, -4, -5, -6) == -21);
}

// ------------------------------------------------------------------
// 5. Pointer argument: dereference and return
// ------------------------------------------------------------------

__attribute__((naked)) int n05_deref(int *p) {
    __asm__("movl (%rdi), %eax\n\t"
            "ret");
}

void test_deref(void) {
    int x = 99;
    assert(n05_deref(&x) == 99);
    x = -1;
    assert(n05_deref(&x) == -1);
}

// ------------------------------------------------------------------
// 6. 64-bit return: use 64-bit registers
// ------------------------------------------------------------------

__attribute__((naked)) uint64_t n06_id64(uint64_t v) {
    __asm__("movq %rdi, %rax\n\t"
            "ret");
}

void test_id64(void) {
    assert(n06_id64(0x123456789ABCDEF0ULL) == 0x123456789ABCDEF0ULL);
}

// ------------------------------------------------------------------
// 7. Naked with no arguments and no return (void)
// ------------------------------------------------------------------

// A void naked function must still `ret`. We expose the side effect
// through a global via basic asm.
long n07_side = 0;

__attribute__((naked)) void n07_set(long *p, long v) {
    __asm__("movq %rsi, (%rdi)\n\t"
            "ret");
}

void test_void_naked(void) {
    long x = 0;
    n07_set(&x, 123);
    assert(x == 123);
    n07_set(&x, -5);
    assert(x == -5);
}

// ------------------------------------------------------------------
// 8. Naked with __attribute__((used)) and noinline
// ------------------------------------------------------------------

__attribute__((naked, used, noinline))
int n08_used(int x) {
    __asm__("leal 1(%rdi), %eax\n\t"
            "ret");
}

void test_used(void) {
    assert(n08_used(10) == 11);
}

// ------------------------------------------------------------------
// 9. Naked function with callee-saved register preserved
//    (correct ABI: if we clobber rbx, we must save/restore it)
// ------------------------------------------------------------------

__attribute__((naked)) long n09_use_rbx(long a, long b) {
    __asm__("pushq %rbx\n\t"
            "movq %rdi, %rbx\n\t"
            "addq %rsi, %rbx\n\t"
            "movq %rbx, %rax\n\t"
            "popq %rbx\n\t"
            "ret");
}

void test_callee_saved(void) {
    // Simple correctness test.
    assert(n09_use_rbx(10, 20) == 30);

    // In a loop, to exercise the save/restore repeatedly.
    long s = 0;
    for (long i = 0; i < 1000; i++) {
        s += n09_use_rbx(i, 1);
    }
    assert(s == 1000 * 1 + (999 * 1000) / 2);
}

// ------------------------------------------------------------------
// 10. Naked function that calls another function
//     (must realign stack if it makes calls; here we keep it simple
//      and only use caller-saved registers so no alignment is needed
//      for a leaf-style call; but a genuine call requires stack
//      alignment of 16 bytes at the call site.)
// ------------------------------------------------------------------

int helper(int x) {
    return x * 3;
}

__attribute__((naked)) int n10_call(int x) {
    __asm__("pushq %rdi\n\t"        // keep arg across call
            "call helper\n\t"       // stack is 16-byte aligned at entry + 8 push
            "popq %rdi\n\t"         // (entry: rsp % 16 == 8 after call,
                                    //  push makes it 0 mod 16, aligned)
            "ret");
}

void test_call(void) {
    assert(n10_call(5) == 15);
    assert(n10_call(0) == 0);
    assert(n10_call(-2) == -6);
}

// ------------------------------------------------------------------
// 11. Naked function with a jump to a local label
// ------------------------------------------------------------------

__attribute__((naked)) int n11_abs(int x) {
    __asm__("movl %edi, %eax\n\t"
            "testl %eax, %eax\n\t"
            "jns 1f\n\t"
            "negl %eax\n\t"
            "1:\n\t"
            "ret");
}

void test_abs(void) {
    assert(n11_abs(5) == 5);
    assert(n11_abs(-5) == 5);
    assert(n11_abs(0) == 0);
}

// ------------------------------------------------------------------
// 12. Naked function with a loop (basic asm)
// ------------------------------------------------------------------

__attribute__((naked)) int n12_popcount(unsigned x) {
    __asm__("xorl %eax, %eax\n\t"
            "1:\n\t"
            "testl %edi, %edi\n\t"
            "jz 2f\n\t"
            "movl %edi, %edx\n\t"
            "andl $1, %edx\n\t"
            "addl %edx, %eax\n\t"
            "shrl $1, %edi\n\t"
            "jmp 1b\n\t"
            "2:\n\t"
            "ret");
}

void test_popcount(void) {
    assert(n12_popcount(0u) == 0);
    assert(n12_popcount(1u) == 1);
    assert(n12_popcount(7u) == 3);
    assert(n12_popcount(0xFFFFFFFFu) == 32);
}

// ------------------------------------------------------------------
// 13. Naked function with a top-level label (function-scope label)
//     used together with C labels elsewhere – labels are per-function.
// ------------------------------------------------------------------

__attribute__((naked)) int n13_label(void) {
    __asm__("movl $7, %eax\n\t"
            "jmp 1f\n\t"
            "movl $0, %eax\n\t"
            "1:\n\t"
            "ret");
}

void test_label(void) {
    assert(n13_label() == 7);
}

// ------------------------------------------------------------------
// 14. Naked function with `__attribute__((noreturn))`
//     (must not return via ret; we use a call to a real noreturn
//      function so the test is finite)
// ------------------------------------------------------------------

__attribute__((noreturn)) void my_abort(void) {
    __builtin_trap();
}

__attribute__((naked, noreturn)) void n14_noreturn(void) {
    __asm__("jmp my_abort");
}

// We do not call n14_noreturn() in the test because it aborts.

// ------------------------------------------------------------------
// 15. Naked function that forwards varargs? Not possible in C;
//     however we can forward a pointer to a va_list-like structure.
//     Here we just pass a fixed number of args and use them.
// ------------------------------------------------------------------

__attribute__((naked)) long n15_mul(long a, long b) {
    __asm__("movq %rdi, %rax\n\t"
            "imulq %rsi, %rax\n\t"
            "ret");
}

void test_mul(void) {
    assert(n15_mul(6, 7) == 42);
    assert(n15_mul(-3, 4) == -12);
}

// ------------------------------------------------------------------
// 16. Naked function that returns a floating-point value
//     (System V: float/double returned in xmm0)
// ------------------------------------------------------------------

__attribute__((naked)) double n16_ret_double(void) {
    // 1.5 = 0x3FF8000000000000; materialize the bits without a rodata
    // section (inline asm cannot switch sections under LLVM).
    __asm__("movabsq $0x3FF8000000000000, %rax\n\t"
            "movq %rax, %xmm0\n\t"
            "ret");
}

void test_ret_double(void) {
    assert(n16_ret_double() == 1.5);
}

// ------------------------------------------------------------------
// 17. Naked function taking a double argument
//     (first double arg is in xmm0)
// ------------------------------------------------------------------

__attribute__((naked)) double n17_double_arg(double x) {
    __asm__("addsd %xmm0, %xmm0\n\t"     // x + x
            "ret");
}

void test_double_arg(void) {
    assert(n17_double_arg(2.5) == 5.0);
    assert(n17_double_arg(-1.0) == -2.0);
}

// ------------------------------------------------------------------
// 18. Naked function with both integer and floating args
//     (int in rdi, double in xmm0)
// ------------------------------------------------------------------

__attribute__((naked)) double n18_mix(int n, double x) {
    __asm__("cvtsi2sdl %edi, %xmm1\n\t"
            "addsd %xmm1, %xmm0\n\t"
            "ret");
}

void test_mix(void) {
    assert(n18_mix(3, 4.5) == 7.5);
    assert(n18_mix(-1, 0.0) == -1.0);
}

// ------------------------------------------------------------------
// 19. Naked function with stack manipulation (small local buffer
//     on the stack, no C locals)
// ------------------------------------------------------------------

__attribute__((naked)) long n19_sum_bytes(const unsigned char *p, long n) {
    __asm__("xorq %rax, %rax\n\t"        // sum = 0
            "testq %rsi, %rsi\n\t"
            "jle 2f\n\t"
            "1:\n\t"
            "movzbl (%rdi), %edx\n\t"
            "addq %rdx, %rax\n\t"
            "incq %rdi\n\t"
            "decq %rsi\n\t"
            "jnz 1b\n\t"
            "2:\n\t"
            "ret");
}

void test_sum_bytes(void) {
    unsigned char buf[] = { 1, 2, 3, 4, 5 };
    assert(n19_sum_bytes(buf, 5) == 15);
    assert(n19_sum_bytes(buf, 0) == 0);
    assert(n19_sum_bytes(buf, 1) == 1);
}

// ------------------------------------------------------------------
// 20. Naked function that uses r10/r11 (caller-saved scratch)
// ------------------------------------------------------------------

__attribute__((naked)) long n20_scratch(long a, long b) {
    __asm__("movq %rdi, %r10\n\t"
            "movq %rsi, %r11\n\t"
            "leaq (%r10,%r11,2), %rax\n\t"   // a + 2*b
            "ret");
}

void test_scratch(void) {
    assert(n20_scratch(1, 2) == 5);
    assert(n20_scratch(10, 20) == 50);
}

// ------------------------------------------------------------------
// 21. Naked function with a conditional branch on the sign flag
// ------------------------------------------------------------------

__attribute__((naked)) int n21_sign(int x) {
    __asm__("xorl %eax, %eax\n\t"
            "testl %edi, %edi\n\t"
            "setg %al\n\t"           // al = (x > 0)
            "ret");
}

void test_sign(void) {
    assert(n21_sign(1) == 1);
    assert(n21_sign(0) == 0);
    assert(n21_sign(-1) == 0);
}

// ------------------------------------------------------------------
// 22. Naked function that copies memory with rep movsb
// ------------------------------------------------------------------

__attribute__((naked)) void n22_memcpy(void *dst, const void *src, size_t n) {
    __asm__("movq %rdi, %r8\n\t"     // save dst for return-less use
            "movq %rsi, %rsi\n\t"    // src -> rsi (already)
            "movq %rdx, %rcx\n\t"    // count -> rcx
            "rep movsb\n\t"
            "ret");
}

void test_memcpy(void) {
    unsigned char src[] = "hello, world";
    unsigned char dst[sizeof(src)];
    n22_memcpy(dst, src, sizeof(src));
    for (size_t i = 0; i < sizeof(src); i++) {
        assert(dst[i] == src[i]);
    }
}

// ------------------------------------------------------------------
// 23. Naked function with `__attribute__((section("...")))`
//     (must not affect correctness)
// ------------------------------------------------------------------

__attribute__((naked, section(".text.n23")))
int n23_section(void) {
    __asm__("movl $123, %eax\n\t"
            "ret");
}

void test_section(void) {
    assert(n23_section() == 123);
}

// ------------------------------------------------------------------
// 24. Naked function with `__attribute__((aligned(N)))`
// ------------------------------------------------------------------

__attribute__((naked, aligned(16)))
int n24_aligned(void) {
    __asm__("movl $7, %eax\n\t"
            "ret");
}

void test_aligned(void) {
    assert(n24_aligned() == 7);
}

// ------------------------------------------------------------------
// 25. Naked function calling another naked function
// ------------------------------------------------------------------

__attribute__((naked)) int n25_inner(void) {
    __asm__("movl $100, %eax\n\t"
            "ret");
}

__attribute__((naked)) int n25_outer(void) {
    // After `call`, rsp is 16-byte aligned (entry: rsp % 16 == 8,
    // call pushes 8 -> rsp % 16 == 0). No further push needed.
    __asm__("call n25_inner\n\t"
            "addl $1, %eax\n\t"
            "ret");
}

void test_naked_calls_naked(void) {
    assert(n25_outer() == 101);
}

// ------------------------------------------------------------------
// 26. Naked function with a global side effect through asm
// ------------------------------------------------------------------

int n26_global = 0;

__attribute__((naked)) void n26_bump(void) {
    __asm__("movl n26_global(%rip), %eax\n\t"
            "addl $1, %eax\n\t"
            "movl %eax, n26_global(%rip)\n\t"
            "ret");
}

void test_global_side_effect(void) {
    n26_global = 0;
    for (int i = 0; i < 10; i++) n26_bump();
    assert(n26_global == 10);
}

// ------------------------------------------------------------------
// 27. Naked function used as a function pointer target
// ------------------------------------------------------------------

__attribute__((naked)) int n27_fp_target(int x) {
    __asm__("leal 100(%rdi), %eax\n\t"
            "ret");
}

void test_function_pointer(void) {
    int (*fp)(int) = n27_fp_target;
    assert(fp(5) == 105);
    assert(fp(-5) == 95);
}

// ------------------------------------------------------------------
// 28. Naked with tail call (jmp to another function)
// ------------------------------------------------------------------

int n28_target(int x) { return x * x; }

__attribute__((naked)) int n28_tail(int x) {
    __asm__("jmp n28_target");
}

void test_tail_call(void) {
    assert(n28_tail(6) == 36);
    assert(n28_tail(-3) == 9);
}

// ------------------------------------------------------------------
// 29. Naked function returning a small struct (System V: in rax / rdx)
//     For a struct { int a; int b; } of size 8, it is returned in rax.
// ------------------------------------------------------------------

struct pair { int a; int b; };

__attribute__((naked)) struct pair n29_make_pair(void) {
    // rax = (a) | (b << 32) = 1 | (2 << 32)
    __asm__("movabsq $0x0000000200000001, %rax\n\t"
            "ret");
}

void test_small_struct_return(void) {
    struct pair p = n29_make_pair();
    assert(p.a == 1);
    assert(p.b == 2);
}

// ------------------------------------------------------------------
// 30. Naked function taking a struct argument
//     Small struct { int a; int b; } is passed in a single register (rdi).
// ------------------------------------------------------------------

__attribute__((naked)) int n30_sum_pair(struct pair p) {
    // p.a is low 32 bits of rdi, p.b is high 32 bits.
    __asm__("movq %rdi, %rax\n\t"
            "shrq $32, %rax\n\t"      // rax = p.b
            "addl %edi, %eax\n\t"     // rax = p.b + p.a (32-bit add)
            "ret");
}

void test_small_struct_arg(void) {
    struct pair p = { 3, 4 };
    assert(n30_sum_pair(p) == 7);
    struct pair q = { -10, 25 };
    assert(n30_sum_pair(q) == 15);
}

// ------------------------------------------------------------------
// main
// ------------------------------------------------------------------

int main(void) {
    test_const();
    test_add();
    test_add3();
    test_add6();
    test_deref();
    test_id64();
    test_void_naked();
    test_used();
    test_callee_saved();
    test_call();
    test_abs();
    test_popcount();
    test_label();
    test_mul();
    test_ret_double();
    test_double_arg();
    test_mix();
    test_sum_bytes();
    test_scratch();
    test_sign();
    test_memcpy();
    test_section();
    test_aligned();
    test_naked_calls_naked();
    test_global_side_effect();
    test_function_pointer();
    test_tail_call();
    test_small_struct_return();
    test_small_struct_arg();
    return 0;
}
