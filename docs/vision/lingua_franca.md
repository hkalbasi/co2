# Make Rust the new lingua franca for system programming languages

Note: This is just some of my ideas, and it needs more work for details. CO2 and CO2++ can exist even without this.
If you are interested in pushing this forward in the Rust project, contact me.

## The current lingua franca is C

C has served as the universal interface language for systems programming.
Its simple, stable ABI allows libraries written in one language to be called from another with minimal friction.
If you want to use a library of language A in language B, you most probably will go through C since
almost all languages can speak C ABI and use/be used as a C library.

But C as a lingua franca is not without problem. C's ABI is simple because C's type system is simple. There is no generics, no ownership semantics, no sum types and no lifetimes. Higher-level information is stripped away at the boundary. A `Vec<T>` becomes a `T*` and a length.
A Result becomes an error code. The richness of modern type systems is lost in translation, and with it,
safety guarantees that compilers could otherwise enforce across language boundaries.

For example, C++ and Rust both have (although somehow incompatible) generics.
But since they are forced to talk to each other using C, they lose or severely limit their generic types
at the FFI boundary.

## The core language inside Rust

Inside of Rust, there is a [minimal, core language which is called minirust](https://github.com/minirust/minirust) 
and in this document I call it the "MIR language". MIR language consists of:
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

The MIR language has no specific syntax (but it is easy to imagine a minimal one).
Every language that is based on the MIR lang, can appear in the crate ecosystem use libraries of Rust
and other MIR languages. So the MIR lang has the potential of being the new lingua franca for
system programming language, with these extra features over C:
* More expressive type system
* Generic functions
* Crate and module system as a namespace for preventing conflicts between libraries/languages/versions.
* A unified package manager (Cargo)

## C as a MIR lang

CO2 demonstrates a typical MIR language. It remains backward compatible to the C syntax,
macro system (the preprocessor), and even ABI, while trying to be a good citizen in Rust crates.
CO2 levels up the Rust/C interop. For C to Rust direction, CO2 unlike rust-bindgen does not need code generation,
and enables using C headers using the same `#include`. For Rust to C direction, it completely changes the game.
It enables using Rust API involving generics or types with non trivial destructor,
without needing an unsafe and type erased wrapper.

CO2 is also important as a MIR lang because it can act as a bridge between MIR langs and the languages that
can talk C. 

## C++ as a MIR lang

C++ as a MIR language would be a more ambitious undertaking. While I definitely think a CO2++ language is feasible,
there are some challenges relative to CO2 since there was support for C features in Rust/MIR lang,
even obscure features like C-variadics, but it isn't the case for C++. Some notable challenges:
* Overloaded functions
* Templates, which are not compatible with Rust generics completely
* Inheritance

The CO2++ can easily get these working in a single crate (by doing what a normal C++ compiler does) but the problem
is that we need to somehow encode these things in the crate API, so that at least C++ libraries become able to
get split into multiple crates.

I wrote more [about CO2++ here](./co2pp.md).

## Empowering new system languages with crates ecosystem

Decades from now, there will probably be a new revolutionary language which solves a problem that Rust
can't solve in a backward compatible way, like how Rust itself solved the safe memory management without GC.
Having MIR lang as the new lingua franca, it can make that language able to seamlessly reuse Rust crates,
maybe reducing the need of Rewrite-it-in-that-language projects.

MIR langs can enable playing with new ideas without the ecosystem penalty that usually dooms experimental languages.
A researcher with a novel approach to memory management doesn't need to implement a standard library,
write HTTP and JSON crates from scratch, or build a package manager before anyone writes a real program in the language.
They write a frontend that targets MIR, plug it into Cargo, and immediately their users can import `serde`, `tokio`, and `clap`.
In this way, they can get some real world usage on the language and see its limitations and drawbacks in action.

Some of these ideas can find their way into the Rust itself. A MIR language can serve as a testbed.
If the experiment succeeds, the Rust project gains a working prototype and real-world usage data to inform language evolution.
That MIR language should already interact with Rust crates and use Rust type system as the base,
so it might be easy to introduce its progress in Rust itself.

## Official MIR langs

While MIR langs can become a great test bed for language experiments, they can also split the ecosystem.
If every library can have a language for itself, we may reach a point where people can't read or modify
other codes in the ecosystem since there are too many languages to learn.

To prevent this, we can split MIR langs into official and unofficial.
Every MIR lang start's its life as an unofficial language. For using an unofficial language,
users need to install the compiler manually, and they are discouraged to publish crates on crates.io.
(They can still use git dependencies, and use available crates on crates.io)
When an unofficial language gets major adoption, it can be proposed to become an official language.
Proposing a language to become official could use the current RFC system.
The language that is going to be official, should have some properties:
e.g. it should impose no additional semver requirements for Rust (and other official langs) crates.

When a language becomes official, its compiler will get released using the rustup,
and it become allowed to publish in crates.io. The bar of entry for new official languages should be very high,
and no language should be accepted as a new official language due purely aesthetic differences.

## Implementation

MIR lang compilers can emit something similar to a `.rlib` or `.rmeta` file, containing the items,
type definitions, documents, and polymorphic MIR of the functions. Then the backend, cargo doc, miri,
and similar tools can consume this file:

![alt text](<middle.svg>)
