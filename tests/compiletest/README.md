# co2 compiletest suites

Each test file is self-contained and uses inline directives prefixed with `//@`.

Supported suites:
- `tests/compiletest/ui`: compiler diagnostics checks.
- `tests/compiletest/run`: compile + run checks (status/stdout/stderr).
- `tests/compiletest/debuginfo`: debugger script checks (`gdb`/`lldb`).

Common directives:
- `//@ mode: c|co2|rust`
- `//@ compile-flags: ...` (repeatable, shell-split)
- `//@ skip: <reason>` (skip test unconditionally)

UI directives:
- `//@ ui-error: <substring>` (repeatable)
- `//@ ui-stderr-contains: <substring>` (repeatable)

Run directives:
- `//@ run-status: <int>` (default `0`)
- `//@ run-args: ...` (repeatable, shell-split)
- `//@ run-stdout: <exact text>` (`\\n` escapes are supported)
- `//@ run-stderr: <exact text>` (`\\n` escapes are supported)
- `//@ run-stdout-contains: <substring>` (repeatable)
- `//@ run-stderr-contains: <substring>` (repeatable)
- `//@ aux-lib: <crate_name> <relative_path>` (repeatable, Rust mode only)
  Compiles each listed `.rs` or `.co2` file as an `rlib` using `co2` and links it into the Rust test binary.
  Helper source files can use `.aux.rs` or `.aux.co2` suffix to avoid being collected as standalone tests.

Debuginfo directives:
- `//@ debugger: gdb|lldb` (default `gdb`)
- `//@ debug-command: <command>` (repeatable)
- `//@ debug-check: <substring>` (repeatable)
- `//@ debug-status: <int>` (default `0`)

Run harness:
- `cargo run -p co2_test_harness -- all`
- `cargo run -p co2_test_harness -- ui`
- `cargo run -p co2_test_harness -- run`
- `cargo run -p co2_test_harness -- debuginfo`
