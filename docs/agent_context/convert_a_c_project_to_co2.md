# Porting a C project to a co2cargo project

This guide provides instructions for an agent to port an existing C project to a `co2cargo` project.

## Overview
`co2cargo` is a wrapper around Rust's `cargo` that enables CO2 language features. It sets `RUSTC=co2rustc`, which can compile `.co2` files (backward-compatible with C) and integrate them with Rust crates and the standard library.

## Step-by-Step Porting Process

### 1. Initialize the co2cargo project
In the root directory of the C project, run:
```bash
co2cargo init
```
This will:
- Run `cargo init` to create a `Cargo.toml`.
- Add `#![co2::language]` to `src/main.rs` (or `src/lib.rs`).
- Create a template `src/main.co2` (or `src/lib.co2`).

### 2. Migration of C Source Files
- Rename your main C file (e.g., `main.c`) to `src/main.co2` (overwriting the template).
- Other C source files should also be renamed to have a `.co2` extension if they contain CO2 features or if you want them managed by the CO2 toolchain.
- Note: `co2rustc` currently detects the CO2 file by looking for a file with the same name as the Rust entry point but with a `.co2` extension (e.g., if `src/main.rs` has `#![co2::language]`, it looks for `src/main.co2`).

### 3. Handling Headers and Includes
- CO2 supports standard C `#include` directives.
- If you have local headers, ensure they are in the include path.
- `co2cc` uses `gcc -E` for preprocessing, so standard include paths should work.

### 4. Integration with Rust
- You can now use Rust features in your `.co2` files:
  - Use `use` statements at the beginning of the file to import Rust types (e.g., `use std::vec::Vec;`).
  - Use Rust primitives like `i32`, `u64`.
  - Define functions with Rust ABI using `fn` syntax.
  - Call Rust methods and use generic types.
- Add Rust dependencies using `co2cargo add <dep>`.

### 5. Building and Running
- Build the project:
  ```bash
  co2cargo build
  ```
- Run the project:
  ```bash
  co2cargo run
  ```
- Run tests:
  ```bash
  co2cargo test
  ```

## Important Constraints
- `use` statements must be at the very top of the `.co2` file due to current parser limitations.
- See `docs/known_incompatibilties_with_c.md` for specific C features that might not be supported yet.
- `co2cargo` forwards most commands to `cargo` with `RUSTC=co2rustc`. Tools like `cargo fmt` or `cargo clippy` are not yet supported for CO2.
