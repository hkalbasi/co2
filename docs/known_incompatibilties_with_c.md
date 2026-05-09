# List of incompatibilites with C

CO2 tries to be a spec-compliant C compiler, but in some cases, it can't. This document tries to list the
cases when CO2 miscompiles or reject valid C code.

## Incompatibilities due Rust limitations

* `long double` in some system ABIs is defined to be a 80 bit thing, but in CO2 it is always 64 or 128 bit IEEE
  floating point. Linking with libraries built with other compilers and passing long double to them will break (e.g
  You can't use printf `%Lf`)
* Variable length arrays are not implemented (they are optional in the C standard).
  * `alloca` is also not implemented.

## Incompatibilities which seems doesn't worth the effort to fix

* non-UTF8 source files are not supported (not required by the C standard).

## Semantic difference between C and CO2

* Reading uninitialized memory in C gives you undereminate value, but in CO2 it is UB.
