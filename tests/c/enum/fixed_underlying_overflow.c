//@ mode: c
//@ compile-fail

// Fixed underlying type enumerators must be representable (C23 6.7.2.2p2);
// gcc rejects out-of-range values.

enum A : unsigned char { Ok = 254, Ok2, OhNo };
//                                      ^^^^ error: overflow in enumeration values

enum B : unsigned char { Bad = 256 };
//                       ^^^^^^^^^ error: enumerator value outside the range of underlying type

enum C : signed char { Sok = 126, Sok2, SOhNo };
//                                      ^^^^^ error: overflow in enumeration values

enum D : signed char { SBad = 128 };
//                     ^^^^^^^^^^ error: enumerator value outside the range of underlying type

enum E : unsigned char { Neg = -1 };
//                       ^^^^^^^^ error: enumerator value outside the range of underlying type

enum : unsigned char { AnonBad = 300 };
//                     ^^^^^^^^^^^^^ error: enumerator value outside the range of underlying type

int main(void) {
    return 0;
}
