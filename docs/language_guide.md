# CO2 Language Guide

CO2 is a C-compatible language that can use Rust crates and the Rust standard library. It aims to be backward compatible with C while adding seamless Rust interop.

This guide assumes your are familiar with C and Rust, and makes references to those languages without explanation.

## C as the base language

CO2 strives to be a spec-compliant C23 compiler. Most valid C code compiles as-is,
and [minor incompatibilities are documented](./known_incompatibilities_with_c.md).
Incompatibilities beyond that list are considered a bug.

In addition to the C standard, CO2 supports some popular GNU extensions:
* GNU statement expressions
* Computed goto
* Range designator (`[0 ... 4] = 1`)
* Transparent union

## Module system and use Statements

A CO2 compilation unit (a single .c/.co2 file compiled with co2cc or a Cargo crate) is considered a crate.
Each crate contains a tree of modules, with items (functions, type definitions and global variables with static storage)
as leaf of that tree. The module system map into the file system similar to Rust modules. Each file is a module,
and can define its submodules using the `mod foo;` declarations. For modules that are crate roots or are `mod.co2`,
CO2 expects a `foo.co2` or `foo/mod.co2` file containing that module content, and for modules that are called `some_name.co2`,
Co2 expects the submodule to be at `some_name/foo.co2` or `some_name/foo/mod.co2`.

Each module can access items in other modules using `use` statement:
```rust
use foo::baz; // Import item baz from module foo which is submodule of this module
use super::foo::baz; // Import item baz from module foo which is submodule of the parent module
use crate::foo::baz; // Import item baz from module foo which is submodule of the crate root
```

Use statements accept multiple items and glob imports:
```rust
use foo::*; // Import all Items from the module Foo
use crate::{foo::item1, bar::{item2, item3}}; // Use item1 from crate::foo and item2 and item3 from crate::bar.
```

Note: Unlike Rust, the `mod foo;` and `use foo;` statements should appear at top of the file.

## Depending on crates

Projects using Cargo can depend on both Rust and CO2 crates, and import their items using the `use` statements:
```rs
use dep::foo::bar; // Import item bar from module foo of the crate dep.
```
Unlike local modules which should be imported based on their relative path in the module tree,
crates are available in all modules, like Rust.

All crates are automatically dependent on the Rust stdlib (`std`, `core` and `alloc` crates) and the `libc` crate.
Even single c or co2 files compiled using `co2cc` can use items from stdlib and libc crates.

## Rust paths as C ident

Every place which C accepts an identifier, CO2 accepts a Rust path. Here are some examples:

Declaration specifiers:
```
int f1(some::rust::Path1 *p) {} // p becomes a pointer to the `Path1` type.

typedef some::rust::Path2 new_type[10]; // new_type becomes a `[some::rust::Path2; 10]`
```

Expressions:
```
int main() {
    some_crate::SomeType v1 = some_crate::SomeType::new(); // The new function is in the expression position which accepts Rust paths
}
```

## Generic functions and types

Rust paths in the CO2 are more powerful than paths in the `use` statements, they can have generic arguments:

```
use std::vec::Vec;

int main() {
    Vec::<i32> v = Vec::<i32>::new(); // This one is not just an example, it actually works
}
```

CO2 does almost no type inference about generics, unlike Rust. So explicit types are needed when using generic types.
CO2 also does not allow you to define generic things, you can just use generic things defined in Rust.
The turbofish syntax `::<` is mandatory anywhere Rust paths appear as C identifier.

## Rust type syntax

Types in the generic arguments use the type syntax of Rust, because it is not tied to a declarator.
So for example for a vector of pointers you should write `Vec::<*mut T>`.
The exact details of the Rust type grammar accepted by CO2 is defined in the reference, but we leave some examples here:

* Pointer: `T *decl` == `*mut T`
* Array: `T decl[10]` == `[T; 10]`
* Tuples: `(T1, T2)` which are unrepresentable as C declarators.
* C type specifiers like `int` and `unsigned int` are not accepted. You need to use `i32` or `std::ffi::c_int`.

Paths do not need `::<` in a Rust type (but it is accepted so you can use turbofish everywhere). So this is valid:
```
typedef Vec::<Vec<i32>> VectorOfVector;
         //^^^ This is mandatory since the first type is in the declaration specifier position.
```

## Rust-Style items

Since CO2 items are exported to Rust crates, sometimes you need more control on the properties of the exported item.
For example you may want to control the ABI of the functions, or control which traits the structs have.
This control needs a syntax. CO2 reuses the Rust syntax in these cases instead of inventing its own.

### Functions

CO2 supports defining functions with Rust ABI using `fn` syntax:

```co2
fn rust_style_function(a: i32, b: i32) -> i32 {
    // The body still is a C block.
    goto label;
    label:
        return c_style_function2(a, b);
}
```

The body still uses C syntax (goto, etc.) but the signature uses Rust types.
Using this syntax allow you to control over publicness, safety and ABI of the function:

```
fn safe_private() {}
unsafe fn unsafe_private() {}
pub unsafe fn unsafe_public() {}
pub unsafe extern "C" fn unsafe_public_with_c_abi(arg: *mut c_int) {}
void unsafe_public_with_c_abi_equivalent(int *arg) {} // C style functions are always public and unsafe and extern "C"
```

Arguments and return types use the Rust syntax.

### Typedefs

Rust styles typedefs give you control over the publicness of typedefs:
```
type PrivateTypeDef = *mut i32;
pub type PublicTypeDef = *mut i32;
typedef int *AnotherPublicTypeDef; // C style typedefs are always public
```
