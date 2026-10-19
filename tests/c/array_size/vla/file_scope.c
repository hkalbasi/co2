//@ mode: c
//@ compile-fail
//
// A variably modified type cannot appear at file scope: there is no runtime
// scope in which its bound could be evaluated, and the object would need
// static storage duration (C11 6.7.6.2p2).
//
// Kept separate from `invalid.c` because this fails during crate-signature
// lowering: while it is red, it prevents every body-level check in the same
// file from being reached.

int n;
int file_vla[n];
//  ^^^^^^^^ error: variable length array cannot appear at file scope

typedef int file_vla_t[n];
//          ^^^^^^^^^^ error: variable length array cannot appear at file scope
