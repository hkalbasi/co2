# CO2++

CO2 helps with integrating Rust and C projects, but the more interesting and important problem is the Rust and C++ integration.
Rust already can talk C, and C types are a subset of Rust types. But C++ and Rust need to use C as a linua franca,
losing features such as templates (generics), operator overloading, destructors, and ... which both languages have but C doesn't.

## CO2 would not be a subset of CO2++

Technically speaking, C is not a subset of C++, due to features like `restrict`, initializer lists and ABI differences,
you can't use a C++ compiler to compile C code. But most of the C features are available in the C++,
usually with the same syntax.

While CO2 tries hard to keep C as a subset and CO2++ should similarly try hard to keep C++ as a subset,
it is not feasible to make CO2 a subset of CO2++, without making CO2 worse. The reason is that CO2 has some Rust features,
which C++ has them too but with a completely different syntax, and CO2 doesn't want to use the suboptimal C++ syntax,
while CO2++ wants to reuse the C++ syntax since it should support it anyway.

## CO2++ can't expose some C++ items in the crate boundary

Rust can't express some things in the C++, like:
* Overloaded functions
* Unrestricted templates
* Protected members

And some things like C++ types with non trivial move constructors can exist in a crate boundary in a limited form.
I think CO2++ needs an additional syntax for defining Rust-like items, like CO2 `fn`.
Initially, CO2++ can start with exposing no C++ specific item in the crate boundary, and only expose Rust-like items.
Then it can detect and expose items exposable in the crate boundary, maybe by an annotation and emit error if the type is not exposable.

This may create problems for organizing C++ in multiple crates, since users may need to write wrapper for their C++ code.
But the `#include` option is always available, and CO2++ (or even CO2) can make cross crate includes possible with some build system tricks.

The Rust language can solve some of these problems, but I don't think this is a direction that Rust should take in general.
In some cases, the C++ feature may make sense, but generally we shouldn't add C++ legacy to the Rust.

## Want to help making CO2++ a reality?

Contact me.
