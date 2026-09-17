//@ mode: c
//@ run-status: 0
// Test for GNU case ranges: case lo ... hi:
// This is a GNU extension, not standard C.
// Compile with -std=gnu* (default) or -std=c23 -pedantic will warn.

#include <stddef.h>
#include <assert.h>

// ------------------------------------------------------------
// 1. Basic ASCII character range
// ------------------------------------------------------------

int classify_char(int c) {
    switch (c) {
        case 'a' ... 'z':
            return 1;   // lowercase
        case 'A' ... 'Z':
            return 2;   // uppercase
        case '0' ... '9':
            return 3;   // digit
        default:
            return 0;
    }
}

// ------------------------------------------------------------
// 2. Integer ranges
// ------------------------------------------------------------

int classify_int(int n) {
    switch (n) {
        case 0 ... 9:
            return 1;   // single digit
        case 10 ... 99:
            return 2;   // two digits
        case 100 ... 999:
            return 3;   // three digits
        case -9 ... -1:
            return 4;   // negative single digit
        default:
            return 0;
    }
}

// ------------------------------------------------------------
// 3. Single-element range (lo == hi) behaves like normal case
// ------------------------------------------------------------

int single_range(int n) {
    switch (n) {
        case 5 ... 5:
            return 100;
        default:
            return 0;
    }
}

// ------------------------------------------------------------
// 4. Mixed range and normal cases
// ------------------------------------------------------------

int mixed(int n) {
    switch (n) {
        case 0:
            return 10;
        case 1 ... 3:
            return 20;
        case 4:
            return 30;
        case 5 ... 7:
            return 40;
        default:
            return 0;
    }
}

// ------------------------------------------------------------
// 6. Range with unsigned values
// ------------------------------------------------------------

unsigned classify_unsigned(unsigned u) {
    switch (u) {
        case 0u ... 10u:
            return 1;
        case 11u ... 100u:
            return 2;
        case 101u ... 1000u:
            return 3;
        default:
            return 0;
    }
}

// ------------------------------------------------------------
// 7. Range with long long values
// ------------------------------------------------------------

int classify_ll(long long v) {
    switch (v) {
        case -100LL ... -1LL:
            return -1;
        case 0LL ... 100LL:
            return 1;
        default:
            return 0;
    }
}

// ------------------------------------------------------------
// 8. Range in a function returning a value (with fallthrough)
// ------------------------------------------------------------

int with_fallthrough(int n) {
    int result = 0;
    switch (n) {
        case 0 ... 4:
            result += 1;
            // fallthrough (GNU case ranges allow falling out)
        case 5 ... 9:
            result += 10;
            break;
        case 10 ... 14:
            result += 100;
            [[fallthrough]];
        case 15 ... 19:
            result += 1000;
            break;
        default:
            result = -1;
            break;
    }
    return result;
}

// ------------------------------------------------------------
// 9. Empty switch body with only default
// ------------------------------------------------------------

int only_default(int n) {
    switch (n) {
        default:
            return n * 2;
    }
}

// ------------------------------------------------------------
// 10. Ranges using enum constants
// ------------------------------------------------------------

enum color { RED = 0, ORANGE = 1, YELLOW = 2, GREEN = 3, BLUE = 4 };

int color_group(enum color c) {
    switch (c) {
        case RED ... YELLOW:
            return 1;   // warm colors
        case GREEN ... BLUE:
            return 2;   // cool colors
        default:
            return 0;
    }
}

// ------------------------------------------------------------
// 11. Range with min/max of int (limits.h)
// ------------------------------------------------------------

#include <limits.h>

int full_int_range(int n) {
    switch (n) {
        case INT_MIN ... -1:
            return -1;
        case 0:
            return 0;
        case 1 ... INT_MAX:
            return 1;
        default:
            return 2;   // unreachable
    }
}

// ------------------------------------------------------------
// 12. Nested switch with case ranges
// ------------------------------------------------------------

int nested(int a, int b) {
    int r = 0;
    switch (a) {
        case 0 ... 9:
            switch (b) {
                case 0 ... 4:
                    r = 1;
                    break;
                case 5 ... 9:
                    r = 2;
                    break;
                default:
                    r = 3;
                    break;
            }
            break;
        case 10 ... 19:
            r = 4;
            break;
        default:
            r = 5;
            break;
    }
    return r;
}

// ------------------------------------------------------------
// Main
// ------------------------------------------------------------

int main(void) {
    // 1. char classification
    assert(classify_char('a') == 1);
    assert(classify_char('m') == 1);
    assert(classify_char('z') == 1);
    assert(classify_char('A') == 2);
    assert(classify_char('Z') == 2);
    assert(classify_char('0') == 3);
    assert(classify_char('9') == 3);
    assert(classify_char('!') == 0);
    assert(classify_char(' ') == 0);

    // 2. int classification
    assert(classify_int(0) == 1);
    assert(classify_int(9) == 1);
    assert(classify_int(10) == 2);
    assert(classify_int(99) == 2);
    assert(classify_int(100) == 3);
    assert(classify_int(999) == 3);
    assert(classify_int(-1) == 4);
    assert(classify_int(-9) == 4);
    assert(classify_int(1000) == 0);
    assert(classify_int(-10) == 0);

    // 3. single-element range
    assert(single_range(5) == 100);
    assert(single_range(4) == 0);
    assert(single_range(6) == 0);

    // 4. mixed range and normal cases
    assert(mixed(0) == 10);
    assert(mixed(1) == 20);
    assert(mixed(2) == 20);
    assert(mixed(3) == 20);
    assert(mixed(4) == 30);
    assert(mixed(5) == 40);
    assert(mixed(6) == 40);
    assert(mixed(7) == 40);
    assert(mixed(8) == 0);

    // 6. unsigned
    assert(classify_unsigned(0u) == 1);
    assert(classify_unsigned(10u) == 1);
    assert(classify_unsigned(11u) == 2);
    assert(classify_unsigned(100u) == 2);
    assert(classify_unsigned(101u) == 3);
    assert(classify_unsigned(1000u) == 3);
    assert(classify_unsigned(1001u) == 0);

    // 7. long long
    assert(classify_ll(-50LL) == -1);
    assert(classify_ll(-1LL) == -1);
    assert(classify_ll(0LL) == 1);
    assert(classify_ll(50LL) == 1);
    assert(classify_ll(100LL) == 1);
    assert(classify_ll(101LL) == 0);
    assert(classify_ll(-101LL) == 0);

    // 8. fallthrough
    assert(with_fallthrough(2) == 11);     // 0..4 -> +1 then fall to 5..9 -> +10 = 11
    assert(with_fallthrough(7) == 10);     // 5..9 -> +10
    assert(with_fallthrough(12) == 1100);  // 10..14 -> +100, fallthrough -> +1000
    assert(with_fallthrough(17) == 1000);  // 15..19 -> +1000
    assert(with_fallthrough(100) == -1);

    // 9. only default
    assert(only_default(21) == 42);

    // 10. enum ranges
    assert(color_group(RED) == 1);
    assert(color_group(ORANGE) == 1);
    assert(color_group(YELLOW) == 1);
    assert(color_group(GREEN) == 2);
    assert(color_group(BLUE) == 2);

    // 11. full int range
    assert(full_int_range(-12345) == -1);
    assert(full_int_range(-1) == -1);
    assert(full_int_range(0) == 0);
    assert(full_int_range(1) == 1);
    assert(full_int_range(12345) == 1);
    assert(full_int_range(INT_MIN) == -1);
    assert(full_int_range(INT_MAX) == 1);

    // 12. nested switch
    assert(nested(5, 3) == 1);
    assert(nested(5, 7) == 2);
    assert(nested(5, 100) == 3);
    assert(nested(15, 0) == 4);
    assert(nested(100, 0) == 5);

    return 0;
}
