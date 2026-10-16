//@ mode: c
//@ run-status: 0

#include <stddef.h>
#include <assert.h>
#include <stdint.h>

// ------------------------------------------------------------------
// 1. Basic asm – no operands, implicitly volatile
// ------------------------------------------------------------------

void test_basic_asm(void) {
    // Empty basic asm is allowed.
    __asm__("");

    // Nop (x86-64 specific).
    __asm__("nop");

    // Basic asm with volatile qualifier (has no effect; basic asm is
    // always implicitly volatile).
    __asm__ __volatile__("nop");
}

// ------------------------------------------------------------------
// 2. Basic asm at file scope (top-level, unused symbol)
// ------------------------------------------------------------------

// Emit a global symbol with basic asm (x86-64). Never referenced;
// co2 ignores file-scope asm.
__asm__(".globl my_asm_global\n"
        "my_asm_global:\n"
        ".long 0x12345678\n");

// ------------------------------------------------------------------
// 3. Extended asm – no outputs, no inputs, no clobbers
// ------------------------------------------------------------------

void test_extended_empty(void) {
    // Empty extended asm with all three sections empty.
    __asm__ __volatile__("" : : : );

    // A nop with no operands.
    __asm__ __volatile__("nop" : : : );
}

// ------------------------------------------------------------------
// 4. Extended asm – input only (nop, inputs have no side effects)
// ------------------------------------------------------------------

void test_input_only(void) {
    int x = 10;

    // Input in register; no outputs.
    __asm__ __volatile__("nop"
                         : /* no outputs */
                         : "r"(x));

    // Input in memory.
    __asm__ __volatile__("nop"
                         : /* no outputs */
                         : "m"(x));

    // Multiple inputs.
    int a = 1, b = 2;
    __asm__ __volatile__("nop"
                         : /* no outputs */
                         : "r"(a), "r"(b));

    // Immediate input.
    __asm__ __volatile__("nop"
                         : /* no outputs */
                         : "i"(42));
}

// ------------------------------------------------------------------
// 5. Extended asm – clobbers with nop (no outputs)
// ------------------------------------------------------------------

void test_clobbers_nop(void) {
    // "memory" clobber – asm reads/writes memory unpredictably.
    int y = 10;
    __asm__ __volatile__("nop"
                         : /* no outputs */
                         : "r"(y)
                         : "memory");

    // Specific register clobber (x86-64 example).
    __asm__ __volatile__("nop"
                         : /* no outputs */
                         : /* no inputs */
                         : "rax");

    // Multiple clobbers.
    __asm__ __volatile__("nop"
                         : /* no outputs */
                         : /* no inputs */
                         : "rax", "rbx", "cc", "memory");
}

// ------------------------------------------------------------------
// 6. Volatile asm with no outputs is never optimized away.
// ------------------------------------------------------------------

void test_volatile_asm(void) {
    __asm__ __volatile__("nop");
}

// ------------------------------------------------------------------
// 7. Top-level asm defining an unused function (file scope)
// ------------------------------------------------------------------

__asm__(".globl top_level_asm_func\n"
        "top_level_asm_func:\n"
        "    movl $99, %eax\n"
        "    ret\n");

// ------------------------------------------------------------------
// 8. asm with no output and no input, only clobbers
// ------------------------------------------------------------------

void test_clobber_only(void) {
    __asm__ __volatile__("nop"
                         : /* no outputs */
                         : /* no inputs */
                         : "cc", "memory");
}

// ------------------------------------------------------------------
// 9. Clobbering a specific register and verifying it is free
// ------------------------------------------------------------------

void test_register_clobber(void) {
    int x = 1;

    // Clobber a specific register. The compiler will save/restore
    // as needed.
    __asm__ __volatile__("movl $0, %%eax\n\t"
                         "nop"
                         : /* no outputs */
                         : /* no inputs */
                         : "eax");

    // x should be unaffected.
    assert(x == 1);
}

// ------------------------------------------------------------------
// 10. Output only
// ------------------------------------------------------------------

void test_output_only(void) {
    int out = 0;
    __asm__("movl $42, %0"
            : "=r"(out));
    assert(out == 42);
}

// ------------------------------------------------------------------
// 11. Input + output
// ------------------------------------------------------------------

void test_input_output(void) {
    int x = 40, z = 0;
    __asm__("movl %1, %0\n\t"
            "addl $2, %0"
            : "=r"(z)
            : "r"(x));
    assert(z == 42);
}

// ------------------------------------------------------------------
// 12. Early clobber – output written before inputs are consumed
// ------------------------------------------------------------------

void test_early_clobber(void) {
    int x = 41, z = 0;
    __asm__("movl %1, %0\n\t"
            "incl %0"
            : "=&r"(z)
            : "r"(x));
    assert(z == 42);
}

// ------------------------------------------------------------------
// 13. Symbolic operand names
// ------------------------------------------------------------------

void test_symbolic_names(void) {
    int x = 40, z = 0;
    __asm__("movl %[a], %[out]\n\t"
            "addl $2, %[out]"
            : [out] "=r"(z)
            : [a] "r"(x));
    assert(z == 42);
}

// ------------------------------------------------------------------
// 14. Multiple instructions sharing operands
// ------------------------------------------------------------------

void test_multiple_instructions(void) {
    int x = 1, y = 2, z = 0;
    __asm__("movl %1, %0\n\t"
            "addl %2, %0"
            : "=&r"(z)
            : "r"(x), "r"(y));
    assert(z == 3);
}

// ------------------------------------------------------------------
// 15. Read-write (`+`) operand
// ------------------------------------------------------------------

void test_read_write(void) {
    int z = 40;
    __asm__("addl $2, %0"
            : "+r"(z));
    assert(z == 42);
}

// ------------------------------------------------------------------
// 16. Memory operands
// ------------------------------------------------------------------

void test_memory(void) {
    int x = 10, out = 0;
    __asm__("movl %1, %0"
            : "=r"(out)
            : "m"(x));
    assert(out == 10);

    int y = 0;
    __asm__("movl $7, %0"
            : "=m"(y));
    assert(y == 7);
}

// ------------------------------------------------------------------
// 17. Immediate (`i`) operand used by value
// ------------------------------------------------------------------

void test_immediate(void) {
    int out = 0;
    __asm__("movl %1, %0"
            : "=r"(out)
            : "i"(42));
    assert(out == 42);
}

// ------------------------------------------------------------------
// 18. Operand size modifiers (`b`/`w`/`k`/`q`) and sized default
// ------------------------------------------------------------------

void test_modifiers(void) {
    uint8_t b = 0xAB, bout = 0;
    __asm__("movb %1, %0"
            : "=r"(bout)
            : "r"(b));
    assert(bout == 0xAB);

    uint16_t w = 0x1234, wout = 0;
    __asm__("movw %1, %w0"
            : "=r"(wout)
            : "r"(w));
    assert(wout == 0x1234);

    uint32_t k = 0x12345678, kout = 0;
    __asm__("movl %1, %k0"
            : "=r"(kout)
            : "r"(k));
    assert(kout == 0x12345678);

    uint64_t q = 0x123456789ABCDEF0ULL, qout = 0;
    __asm__("movq %1, %q0"
            : "=r"(qout)
            : "r"(q));
    assert(qout == 0x123456789ABCDEF0ULL);

    // Unadorned `%N` picks the operand size automatically.
    uint32_t u = 0, usrc = 99;
    __asm__("movl %1, %0"
            : "=r"(u)
            : "r"(usrc));
    assert(u == 99);
}

// ------------------------------------------------------------------
// 19. Local register variables (the register binding is a hint;
//     co2 treats them as plain locals)
// ------------------------------------------------------------------

void test_local_register_vars(void) {
    register int rv __asm__("r12") = 5;
    int out = 0;
    __asm__("movl %1, %0\n\t"
            "addl $37, %0"
            : "=r"(out)
            : "r"(rv));
    assert(out == 42);
}

// ------------------------------------------------------------------
// 20. Matching constraint (`"0"` reuses the output register)
// ------------------------------------------------------------------

void test_matching(void) {
    int out = 0, in = 42;
    __asm__("movl %1, %0"
            : "=r"(out)
            : "0"(in));
    assert(out == 42);
}

// ------------------------------------------------------------------
// 21. Basic asm with raw register references (no operands: the
//     template is literal assembler text, so `%rip`/`%eax` name
//     registers instead of starting an operand placeholder)
// ------------------------------------------------------------------

int basic_asm_counter = 0;

void test_basic_asm_registers(void) {
    basic_asm_counter = 0;
    __asm__ __volatile__("incl basic_asm_counter(%rip)");
    __asm__ __volatile__("addl $41, basic_asm_counter(%rip)");
    assert(basic_asm_counter == 42);
}

// ------------------------------------------------------------------
// Main
// ------------------------------------------------------------------

int main(void) {
    test_basic_asm();
    test_extended_empty();
    test_input_only();
    test_clobbers_nop();
    test_volatile_asm();
    test_clobber_only();
    test_register_clobber();
    test_output_only();
    test_input_output();
    test_early_clobber();
    test_symbolic_names();
    test_multiple_instructions();
    test_read_write();
    test_memory();
    test_immediate();
    test_modifiers();
    test_local_register_vars();
    test_matching();
    test_basic_asm_registers();

    return 0;
}
