# List of incompatibilities with C

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
* In C, primitives are `int`, `long int`, `long long int`, ... and `intN_t` is a type alias to those.
  In CO2, like Rust, `iN` is the primitive type and `core::ffi::{c_int, c_long, c_longlong}` are type aliases.
  So in C in x86_64, `long` and `long long` are distinct 64 bit integer types, but in CO2 both are the same type.
  This generally does not cause problems, since in C and CO2 there are permissive casts between primitive types,
  but it is observable with `_Generic`, `__builtin_types_compatible_p` and similar things.
* `inline int func()` has a very strange semantic which is not representable in Rust. CO2 treats it as `inline int func()`
  which can observably differs from what C99 semantic is for `inline int func()`. Use either `static inline int func()`
  or `extern inline int func()` to avoid inconsistency between C and CO2.

## Incompatibilities which seems doesn't worth the effort to fix

* non-UTF8 source files are not supported (not required by the C standard).
* `int main() {}` without `return 0;` is UB and is not guaranteed to return zero. Just add the `return 0;` yourself.

## Semantic difference between C and CO2

* CO2 uses Rust's [Exposed Provenance](https://doc.rust-lang.org/std/ptr/index.html#exposed-provenance) semantics for
  pointer to integer and integer to pointer casts. This is similar to PNVI-ae-udi, which is not part of the C standard,
  but all major optimizing C compilers have some assumptions similar to it. It is not known if this makes any difference
  for C codes in practice, please open an issue if you found an example impacted by this.
* Reading uninitialized memory in C gives you indeterminate value, but in CO2 it is UB.
* Writing uninitialized memory using bitfields is UB. This is unacceptable and I want to lift this restriction
  when Rust support is added.

## Incompatibilities due lack of interest / missing implementation

These incompatibilities have solution, but they are not implemented. Contact me if you hit them in a real world project.

* `alignas` is ignored: While Rust can't align struct fields, locals or statics, we could have a transparent type providing alignment,
  and implement coercion between that type and the inner type.
