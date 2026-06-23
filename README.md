# CO2

CO2 (oxidized C) is a programming language which is backward compatible with
C (See [incompatibilities](./docs/known_incompatibilities_with_c.md)) but with
direct access to the Rust ecosystem. CO2 and Rust can use each other crates seamlessly,
with no FFI boundaries or extra tooling required. 

## Why another language?

C has served systems programming for decades, but its lack of memory safety, minimal type system,
and absence of a standard package manager are increasingly making people look for alternatives.
Rust has emerged as one of the leading candidate to fill this role,
offering memory safety without a garbage collector, a rich type system, and a modern build system / package manager.

But Rust has a not great story in integrating with C:
* C libraries are second class citizens
  * The API becomes an unsafe and unidiomatic Rust, which probably needs a wrapper
  * They are not distributed by Cargo, and often distributed in binary form, limiting things like LTO or cross compilation
* Rust libraries are not usable in C, needing an special C wrapper
  * Losing Rust types and falling back to things like `void*`
  * Some things (like `HashMap`) are impossible to use in C without performance loss
  * A C wrapper is time consuming to develop

These issues lead to a trend of RIIR (Rewrite It In Rust) which is taking an existing C or C++ project and rewriting it entirely
in Rust to gain safety and modern tooling. This may work for small projects or organizations with massive resources,
but there are billions of lines of C code which is not going to be rewritten, any time soon (unless AI advances massively).

CO2 fixes these issues. Instead of rewriting your project in Rust, you can rewrite it in CO2.
A CO2 rewrite is an easy and feasible task, since CO2 is backward compatible with C,
so it is not like a full rewrite, its more like changing a build system. See [co2-quickjs](https://github.com/HKalbasi/co2-quickjs/)
as an example of a non trivial project ported to CO2.

After that, your CO2 code is a crate like any other Rust crates.  You can import Rust crates without any change,
use all of their API (even generic types and functions), split your code into multiple CO2 crates,
and rewriting some of them or incrementally all of them to Rust for safety gains.

## Getting started

Install the CO2:

```
curl --proto '=https' --tlsv1.2 -sSf https://hkalbasi.github.io/co2/install.sh | sh
```

You can use CO2 in two ways:

* As a standalone C compiler: `co2cc` is a drop-in replacement for gcc, compatible in flags and CLI interface. You can use it directly or plug it into build systems like CMake. It compiles standard C, but also enables some CO2-specific extensions and use of the Rust standard library.

* As a Cargo-based project: `co2cargo` wraps Rust's cargo, letting you create co2 binary and library crates dependent on Rust crates, and vise versa. Run `co2cargo init` to scaffold a project, `co2cargo add <dep>` to pull in Rust crates, and `co2cargo run` to build and execute (See [all available commands](./docs/cargo_third_party.md)). The two languages share a common compiler backend, so calls between them carry zero overhead, and the syntax added to C by CO2 provides a seamless experience in using Rust crates with their original API.

For a step-by-step walkthrough covering installation, language features, and project setup, see the [Quick Tour](./docs/quick_tour.md).

## FAQ

### Is CO2 a memory safe language?

No. It's a safer language than C, but is not a safe language like Rust.

It is safer than C because:
* CO2 code has access to fat pointers, smart pointers and Rust containers,
  which enables bound checking and other safety guarantees if used.
* Some kind of borrow checker is still running, which kicks in specially when using Rust code wrongly.
  The borrow checker only emits warning, and the programmer is responsible for everything,
  but it can still be useful in detecting memory bugs.
* It enables using safe Rust dependencies, with its original API, not an unsafe FFI wrapper around it.
* It enables rewriting safety critical parts of code to Rust, which is memory safe.

But it is not a safe language like Rust:
* There is no `unsafe` block (You can declare your functions as unsafe but it is only for your
  Rust dependents). CO2 allows you to freely dereferencing raw pointers coming from C, calling unsafe Rust functions,
  using unions, and many other things which can easily cause UB.
* You can freely call every C function, e.g. all libc functions, which most of them are super unsafe,
  and have non-trivial preconditions that the caller should abide for memory safety.

### Is C++ syntax supported?

No. We can imagine a CO2++ language which is backward compatible with C++. But CO2 is just compatible with C.
For more details on CO2++, see [this page](./docs/vision/co2pp.md). 

### Why the compiler binary is so huge?

The currently packaged binary is a self extracting archive of co2 + rustc + rust std + llvm + miri.
It would be much smaller if you install the rustc separately and [build CO2 from source](./docs/installation.md).

### How to rewrite a C project in Rust using CO2?

CO2 enables incrementally migrating projects between C and Rust, by using CO2 as a midpoint.
For converting a C project to Rust, you can:

1. Convert it to a co2 project
2. Use Rust dependencies instead of C
  1. Use Rust std instead of libc
  2. Use crates.io instead of hand coded things
  3. Use Rust wrappers of C dependencies instead of themselves.
3. Split your project into multiple crates
  1. Make the public API of each crate minimal and idiomatic in Rust
4. Rewrite each crate into Rust, one by one.
  1. Use tests to keep the behavior stable in each rewrite.
