# Make Rust the new lingua franca for system programming languages

## The current lingua franca is C

C has served as the universal interface language for systems programming.
Its simple, stable ABI allows libraries written in one language to be called from another with minimal friction.
If you want to use a library of language A in language B, you most probably will go through C since
almost all languages can speak C ABI and use/be used as a C library.

But C as a lingua franca is not without problem. C's ABI is simple because C's type system is simple. There is no generics, no ownership semantics, no sum types and no lifetimes. Higher-level information is stripped away at the boundary. A `Vec<T>` becomes a `T*` and a length.
A Result becomes an error code. The richness of modern type systems is lost in translation, and with it,
safety guarantees that compilers could otherwise enforce across language boundaries.

For example, C++ and Rust both have (although somehow incompatible) generics.
But since they are forced to talk to each other using C, they lose or severly limit their generic types
at the FFI boundary.

## The core language inside Rust

Inside of Rust, there is a [minimal, core language which is called minirust](https://github.com/minirust/minirust) 
and in this document I call it the "Cargo language". Cargo language consists of:
* Type system of the Rust, with all details:
  * Traits
  * Lifetimes
  * Safety and calling convention of functions
* MIR definition and operation semantics
* "crate" as the compilation unit
  * Modules and items publicly available in a crate

Everything else belongs to the surface Rust:
* The exact Rust syntax and keywords
* Patterns: `match`, `if let`, `let pat`, ...
* The borrow checker
* Method resolution and auto deref logic
* Operator overloading
* Type inference

The Cargo language has no specific syntax (but it is easy to imagine a minimal one).
Every language that is based on the Cargo lang, can appear in the crate ecosystem use libraries of Rust
and other Cargo languages. So the Cargo lang has the potential of being the new lingua franca for
system programming language, with these extra features over C:
* More expressive type system
* Generic functions
* Crate and module system as a namespace for preventing conflicts between libraries/languages/versions.
* A unified package manager (Cargo)

## C as a Cargo lang

CO2 demonstrates a typical Cargo language. It remains backward compatible to the C syntax,
macro system (the preprocessor), and even ABI, while trying to be a good citizen in Rust crates.
CO2 levels up the Rust/C interop. For C to Rust direction, CO2 unlike rust-bindgen does not need code generation,
and enables using C headers using the same `#include`. For Rust to C direction, it completely changes the game.
It enables using Rust API involving generics or types with non trivial destructor,
without needing an unsafe and type erased wrapper.

CO2 is also important as a Cargo lang because it can act as a bridge between Cargo langs and the languages that
can talk C. 

## C++ as a Cargo lang

C++ as a Cargo language would be a more ambitious undertaking.

## Empowering new system languages with crates ecosystem

## Official cargo langs