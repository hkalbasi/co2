# Project Summary (for future agent calls)

## Goal
This is implementation of a new language called co2 which aims to be backward compatible with C but be
able to use Rust crates and standard library.

This workspace implements a small C-like frontend (`co2c`) and Rust entry tool (`co2`) on top of `rustc_public_generative`.

Core pipeline:
`co2_parser` -> `co2_hir` -> `co2_mir` -> `co2_driver_lib` -> binaries (`co2`, `co2c`).

## Workspace map
- `rustc_public_generative`: public rustc bridge and generation API.
- `co2_parser`: lexer/parser only. No rustc dependency; intended to stay stable-friendly.
- `co2_hir`: lowers one function body to a typed andresolved AST called HIR.
- `co2_mir`: lowers `co2_hir::HirBody` into `rustc_public::mir::Body`.
- `co2_driver_lib`: orchestration, item collection, HIR structure emission, MIR emission wiring.
- `co2`: Rust mode CLI.
- `co2c`: C mode CLI (uses `gcc -E`, filters includes, then compiles).

## Current architecture contracts
- Keep parser concerns in `co2_parser` only.
- `co2_hir` is body-focused:
  - Input: tokens of a single function body + global resolver.
  - Output: `HirBody` (resolved/type-checked AST-like body form).
  - Uses `la_arena` IDs for locals.
  - Does **not** expose module-level HIR concepts.
- MIR emission logic lives in `co2_mir` (not in parser).
- Avoid depending on rustc crates directly in project crates; use `rustc_public_generative`.

## Important implementation notes
- `rustc_public::Ty` is safe during body/MIR phases, but can be problematic in early defining phases.
- For early HIR-structure construction in driver, keep using driver-local `HirTy` handling where needed.
- Never do any special casing and overfitting. Design for long term goals and do things as general as possible.
- Always ask human when in doubt. The human knows everything better than you about Rust and co2.

## Useful commands
- Check everything:
  - `cargo check -q`
- Run Rust mode:
  - `cargo run -q -r -p co2 -- playground/fake_hello_world.rs`
- Run C mode:
  - `cargo run -q -r -p co2c -- playground/hello.c`
  - `cargo run -q -r -p co2c -- playground/point_length.c`

## Known expectations
- `co2c` should preserve C-style return-code behavior for `main`.
- Some generated examples can emit `dead_code` warnings; warnings are currently tolerated if compilation/runtime are correct.
