# List of incompatibilites with C

CO2 tries to be a spec-compliant C compiler, but in some cases, it can't. This document tries to list the
cases when CO2 miscompiles or reject valid C code.

## Incompatibilities due Rust limitations

* `long double` in some system ABIs is defined to be a 80 bit thing, but in CO2 it is always 64 or 128 bit IEEE
  floating point. Linking with libraries built with other compilers and passing long double to them will break (e.g
  You can't use printf `%Lf`)
