//@ mode: c
//@ compile-fail

// TODO: Our test harness requires ^^^^ as number of bytes, which is ugly, but if you actually run the co2cc it's correct.

/* Emoji */

void emoji_identifier(void) {
    int 😀 = 1;
//      ^^^^ error: invalid identifier
}

void emoji_continue(void) {
    int foo😀 = 1;
//         ^^^^ error: invalid identifier
}

/* Symbols */

void math_symbol(void) {
    int ∑ = 1;
//      ^^^ error: invalid identifier
}

void infinity_symbol(void) {
    int ∞ = 1;
//      ^^^ error: invalid identifier
}

void integral_symbol(void) {
    int ∫ = 1;
//      ^^^ error: invalid identifier
}

void arrow_symbol(void) {
    int ← = 1;
//      ^^^ error: invalid identifier
}

void dingbat_symbol(void) {
    int ✨ = 1;
//      ^^^ error: invalid identifier
}

void currency_symbol(void) {
    int € = 1;
//      ^^^ error: invalid identifier
}

/* Invalid starts */

void leading_combining_acute(void) {
    int ́x = 1;
//      ^^ error: identifier cannot begin with a combining character
}

void leading_combining_diaeresis(void) {
    int ̈x = 1;
//      ^^ error: identifier cannot begin with a combining character
}

void leading_zwj(void) {
    int ‍foo = 1;
//      ^^^ error: identifier cannot begin with U+200D ZERO WIDTH JOINER
}

void leading_zwnj(void) {
    int ‌foo = 1;
//      ^^^ error: identifier cannot begin with U+200C ZERO WIDTH NON-JOINER
}

/* Invalid continuations */

void embedded_space(void) {
    int foo bar = 1;
// `foo bar` lexes as two identifiers; a plain space is a valid separator
}

void embedded_nbsp(void) {
    int foo bar = 1;
//         ^^ error: U+00A0 NO-BREAK SPACE is not permitted in an identifier
}

void embedded_em_space(void) {
    int foo bar = 1;
//         ^^^ error: U+2003 EM SPACE is not permitted in an identifier
}

void embedded_bom(void) {
    int foo﻿bar = 1;
//         ^^^ error: U+FEFF ZERO WIDTH NO-BREAK SPACE is not permitted in an identifier
}

void embedded_lrm(void) {
    int foo‎bar = 1;
//         ^^^ error: U+200E LEFT-TO-RIGHT MARK is not permitted in an identifier
}

void embedded_rlm(void) {
    int foo‏bar = 1;
//         ^^^ error: U+200F RIGHT-TO-LEFT MARK is not permitted in an identifier
}

int main(void) {
    return 0;
}
