# Task

Create a cargo workspace, which contains a main library crate named `rustc_public_generative`. It should
expose a function `pub fn generate(impl FnOnce(Context, DependencyInfo) -> CurrentCrateInfo)`. This function
should act as a rustc_driver invokation, but instead of compiling the input files, it should generate
a fake crate based on what the callback returns.

The `DependencyInfo` is
a type that contains the information about dependencies loaded by the rustc_driver and our caller will
use it to create `CurrentCrateInfo` which contains module structure, items, functions, and MIR of those
functions. The context is used for allocating new IDs for spans and such things, if needed. The `generate` function
then need to intercept rustc_driver callbacks and hook queries to
emit binary of the fake crate instead of the original one.

Then also write a binary crate `fake_hello_world` which is a rustc like compiler but always generate a
hello world printing program independent of the input. It should use the `rustc_public_generative` library.

## Conditions

* `rustc_public_generative` can use all rustc_internals crates, but it should not expose any type from them in
  its public API except the ones defined in `rustc_public`.
* We should prefer to reuse `rustc_public` types as much as we can, but it is incomplete and in some cases we need
  to define our types.
* User code should be able to load custom files and declare span for them in items and MIR.
* Use the installed rustc nightly. Don't change the nightly version.
* Don't do hacky things like generating a fake AST or fake source files. Hook queries to inject items directly in the
  HIR and MIR of the compiler session.

## Notes
* I use JJ for development. You are not allowed to do Git/JJ commits, ask me and I will do that if I feel
  the code is in a good state. Don't run any JJ commands so that you don't accidentally break old approved
  code.
* Ask me whether you were in doubt. I know everything about this project and Rust in general better than you.
* Don't try to generate the perfect code in first try. Leave things with `todo!()` for future and just make
  the `fake_hello_world` working.

## FAQ

* Q: Where should rustc_public come from? Is it a local path dependency, a git dependency, or a crate you already have elsewhere?
* A: `rustc_public` is a rustc_private crate and you can use it with feature `#![feature(rustc_private)]`.
* Q: How do you want to handle rustc_private dependencies? Should I add explicit rustc_* dependencies
     in Cargo.toml (and if so, with what paths), or do you already have a preferred setup for building
     with rustc-dev components?
* A: Use `#![feature(rustc_private)]` and `cargo +nightly build`. Add rustc_* crates using `extern crate` statements
     at the beginning of rustc.
* Q: For the query overrides and HIR/MIR injection, is there a reference implementation or preferred
     approach you want me to follow? I can implement via `Config::override_queries`, but I want to
     align with your expectations and any existing patterns.
* A: Yes use `override_queries`. Anything that works in all cases and without any hacks is fine to me, I don't
     have preference on the design of this part. Don't use hacks, specially the hacks related to emiting fake
     AST which can not work in all cases.
* Q: For custom files and spans, do you already have a rustc_public type to
     represent Span/FileName equivalents, or should I define local types and a translation layer?
* A: Refer to the docs of rustc_private crates. I provided a copy for you at `temp_ai/read_only/rustc-private-docs`
* Q: Is there any example for generating fake crate items from `rustc_public` apis?
* A: No, `rustc_public` is designed mostly for analyzing Rust code, not generating it. This is a novel thing
     that I want to propose to the stable-mir team, and this code serves as a proof of concept.