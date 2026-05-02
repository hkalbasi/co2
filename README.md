# CO2

CO2 (oxidized C) is a programming language which is backward compatible with
C (See [incompatibilities](./docs/known_incompatibilties_with_c.md)) but with
direct access to the Rust ecosystem. CO2 and Rust can use each other crates seamlessly,
with no FFI boundaries or extra tooling required. 

## Getting started

Install the CO2:

```
curl http://foo | bash
```

You can use CO2 in two ways:

* As a standalone C compiler: `co2cc` is a drop-in replacement for gcc, compatible in flags and CLI interface. You can use it directly or plug it into build systems like CMake. It compiles standard C, but also enables some CO2-specific extensions and use of the Rust standard library.

* As a Cargo-based project: `co2cargo` wraps Rust's cargo, letting you create binary crates that mix .co2 and .rs files. Run co2cargo init to scaffold a project, `co2cargo add <dep>` to pull in Rust crates, and co2cargo run to build and execute. The two languages share a common compiler backend, so calls between them carry zero overhead.

For a step-by-step walkthrough covering installation, language features, and project setup, see the [Quick Tour](./docs/quick_tour.md).

## FAQ

### Is CO2 a memory safe language?

No. It's a safer language than C, but is not a safe language like Rust.

It is safer than C because:
* CO2 code has access to fat pointers, smart pointers and Rust containers,
  which enables bound checking and other safety guarantees if used.
* Some kind of borrow checker is still running, which kicks in specially when using Rust code wrongly.
* It enables using safe Rust dependencies, with its original API, not an unsafe FFI wrapper around it.
* It enables rewriting safety critical parts of code to Rust, which is memory safe.

But it is not a safe language like Rust:
* There is no `unsafe` block (You can declare your functions as unsafe but it is only for your
  Rust dependents).
* You can freely call every C function, e.g. all libc functions, which most of them are super unsafe.

### Is C++ syntax supported?

No. We can imagine a CO2++ language which is backward compatible with C++. But CO2 is just compatible with C.
For more details on CO2++, see [this page](./docs/vision/lingua_franca.md). 

## Rewrite a C project in Rust

CO2 enables incrementally migrating projects between C and Rust, by using CO2 as a midpoint.
For converting a C project to Rust, you can:

1. Convert it to a co2 project
2. Use Rust dependencies instead of C
  1. Use Rust std instead of libc
  2. Use crates.io instead of hand coded things
  3. Use Rust wrappers of C dependencies instead of themself.
3. Split your project into multiple crates
  1. Make the public API of each crate minimal and idiomatic in Rust
4. Rewrite each crate into Rust, one by one.
  1. Use tests to keep the behavior stable in each rewrite.
