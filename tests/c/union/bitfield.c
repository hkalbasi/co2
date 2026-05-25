//@ mode: c
//@ run-status: 0

#include <stdint.h>

/*
 * Union where multiple bitfields use the same underlying storage type.
 * All start at bit 0 of the union, so they all alias the same bits.
 */
union Flags {
    uint32_t all : 8; /* bits [0,8) */
    uint32_t lo3 : 3; /* bits [0,3) */
    uint32_t hi5 : 5; /* bits [0,5) */
    uint32_t raw;     /* full 32-bit word */
};

static int check_flags(void) {
    union Flags f;
    f.raw = 0;

    /* 0b10110101 = 0xB5 = 181 */
    f.all = 0xB5;

    /* lo3 = bits[0:3) of 0xB5 = 0b101 = 5 */
    if (f.lo3 != 5) return 1;

    /* hi5 = bits[0:5) of 0xB5 = 0b10101 = 21 */
    if (f.hi5 != 21) return 2;

    /* raw low byte reflects what was stored via 'all' */
    if ((f.raw & 0xFF) != 0xB5) return 3;

    /* write through lo3: 0b111 = 7 */
    f.lo3 = 7;
    /* raw & 0xFF = (0xB5 & ~0x7) | 0x7 = 0xB0 | 0x7 = 0xB7 */
    if ((f.raw & 0xFF) != 0xB7) return 4;

    /* hi5 = bits[0:5) of 0xB7 = 0b10111 = 23 */
    if (f.hi5 != 23) return 5;

    return 0;
}

/*
 * Union mixing bitfields with different underlying storage types.
 */
union Mixed {
    uint8_t  nibble : 4;  /* bits [0,4) of u8 overlay  */
    uint16_t byte_lo : 8; /* bits [0,8) of u16 overlay */
    uint32_t word;
};

static int check_mixed(void) {
    union Mixed m;
    m.word = 0;

    m.word = 0xABCD;
    /* nibble = low 4 bits of the first byte = 0xD */
    if (m.nibble != 0xD) return 6;
    /* byte_lo = low 8 bits of the first u16 = 0xCD */
    if (m.byte_lo != 0xCD) return 7;

    /* write through nibble: clear bits[0:4) and set to 0xA */
    m.nibble = 0xA;
    /* first byte is now (0xCD & ~0xF) | 0xA = 0xCA */
    if (m.byte_lo != 0xCA) return 8;

    return 0;
}

int main(void) {
    int rc;
    rc = check_flags();
    if (rc) return rc;
    rc = check_mixed();
    if (rc) return rc;
    return 0;
}
