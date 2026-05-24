# List of incompatibilites with C

CO2 tries to be a spec-compliant C compiler, but in some cases, it can't. This document tries to list the
cases when CO2 miscompiles or reject valid C code.

## Incompatibilities due Rust limitations

* `long double` in some system ABIs is defined to be a 80 bit thing, but in CO2 it is always 64 or 128 bit IEEE
  floating point. Linking with libraries built with other compilers and passing long double to them will break (e.g
  You can't use printf `%Lf`)
* Variable length arrays are not implemented (they are optional in the C standard).
  * `alloca` (gnu extension) is also not implemented.
* Casting pointers to integers in compile time expression contexts is not accepted:
```
static int value = (int)"foo"; // Valid in C, compile error in CO2!

int array_decl[(intptr_t)some_pointer]; // Valid (although not useful) in C, compile error in CO2
``` 

## Incompatibilities which seems doesn't worth the effort to fix

* non-UTF8 source files are not supported (not required by the C standard).

## Semantic difference between C and CO2

* CO2 uses Rust's [Exposed Provenance](https://doc.rust-lang.org/std/ptr/index.html#exposed-provenance) semantics for
  pointer to integer and integer to pointer casts. This is similar to PNVI-ae-udi, which is not part of the C standard,
  but all major optimizing C compilers have some assumptions similar to it. It is not known if this makes any difference
  for C codes in practice, please open an issue if you found an example impacted by this.
* Reading uninitialized memory in C gives you undereminate value, but in CO2 it is UB.
