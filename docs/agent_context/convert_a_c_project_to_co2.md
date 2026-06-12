# Porting a C project to a co2 project

This guide provides instructions for an agent to port an existing C project to a `co2cargo` project and explains the CO2 language features.

## Overview
`co2cargo` is a wrapper around Rust's `cargo` that enables CO2 language features. It sets `RUSTC=co2rustc`, which can compile `.co2` files (backward-compatible with C) and integrate them with Rust crates and the standard library.

## The CO2 Language

CO2 is designed to be a "C with Rust powers." It maintains high compatibility with C while allowing direct access to the Rust ecosystem.

### Key Similarities to C
- **Syntax**: Standard C control flow (`if`, `for`, `while`, `switch`), expressions, and operators.
- **Preprocessor**: Full support for `#include`, `#define`, and conditional compilation.
- **Types**: Supports `int`, `char`, `float`, `struct`, `union`, `enum`, etc.
- **Pointers**: C-style pointer arithmetic and memory management are fully supported.

### Key Differences and Extensions
- **Rust Primitives**: You can use Rust types like `i32`, `u64`, `usize`, `bool` directly in declarations.
- **Rust Functions**: Use the `fn` keyword to define functions with Rust ABI. The body remains C-style.
  ```c
  fn add_rust(a: i32, b: i32) -> i32 {
      return a + b;
  }
  ```
- **Optional Forward Declarations**: Unlike C, functions do not strictly require a forward declaration before use.
- **Method Calls**: You can call methods on Rust objects using the `.` operator.
- **Generic Types**: Access generic Rust types like `Vec<T>` or `Option<T>`.
  ```c
  use std::vec::Vec;
  // ...
  Vec<i32> v = Vec::<i32>::new();
  v.push(10);
  ```

## Module Structure

CO2 integrates into the Rust module system. 

### .rs and .co2 Relationship
A Rust file (e.g., `main.rs`) can "host" a CO2 file by using the `#![language(co2)]` attribute at the crate root or a module level.
When `co2rustc` sees this, it looks for a corresponding `.co2` file (e.g., `main.co2`) in the same directory.

### Submodules
You can define submodules in CO2 using the `mod` keyword, similar to Rust:
```c
// main.co2
mod foo;
mod bar;

fn main() {
    foo::f();
}
```
This expects `foo.co2` or `foo/mod.co2` to exist.

## The `use` Syntax

CO2 supports a powerful `use` syntax for importing Rust items. **Note: `use` statements must appear at the very beginning of the file.**

### Nested Groups
Import multiple items from the same path efficiently:
```c
use std::collections::{HashMap, HashSet};
use foo::{bar::Item1, baz::{Item2, Item3}};
```

### Aliasing with `as`
Rename imports to avoid name collisions or for brevity:
```c
use std::vec::Vec as MyVec;
use some_crate::LongFunctionName as short_fn;
```

### Wildcards
Import all public items from a module:
```c
use my_module::*;
```

## Step-by-Step Porting Process

### 1. Initialize the co2cargo project
In the root directory of the C project, run:
```bash
co2cargo init
```
This will:
- Run `cargo init` to create a `Cargo.toml`.
- Add `#![language(co2)]` to `src/main.rs`.
- Create a template `src/main.co2`.

### 2. Migration of C Source Files
- Rename your main C file (e.g., `main.c`) to `src/main.co2`.
- For multi-file projects, you can use `mod` statements in `src/main.co2` to include other `.co2` files.
- Alternatively, keep some files as C and compile them via traditional means, but using `.co2` allows full Rust integration.

### 3. Replace `#include` with `use`
- Remove all `.h` files.
- Add all `.c` files to the module tree by `mod foo;` statements.
- Replace all `#include` directives with `use`, either `use file::{needed, items};` or `use file::*;`

### 4. Building and Running
- Build the project: `co2cargo build`
- Run the project: `co2cargo run`

## FAQ

* Q: Can I use `build.rs` and invoke gcc for compiling some of the C sources?
* A: No, that is considered cheating. CO2 should compile everything.
  `build.rs` is only allowed for linking external dependencies, but prefer using crates even then.
* Q: Can I reuse `#define` between modules?
* A: No, and that's by design. You have some alternatives:
  * For `#define CONST value`, use C23 `constexpr type CONST = value;` instead.
  * You can use `#include` for sharing complex macros (but prefer modules for anything else).
* Q: I'm hitting `warning: "extern function" redeclared with a different signature`
* A: You are defining a type in multiple modules (maybe by `#include`ing it). Define type in one module
  and `use` it in others to fix this warning.
* Q: In replacing stdlib includes with `use libc::{item1, item2}`, what to do with macros?
* A: Use Rust std counterparts, e.g. `std::ptr::null_mut::<()>()` instead of `NULL`.