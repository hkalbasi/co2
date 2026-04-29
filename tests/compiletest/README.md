# co2 compiletest suites

Each test file is self-contained and uses inline directives prefixed with `//@`.

Supported suites:
- `tests/compiletest/ui`: compiler diagnostics checks.
- `tests/compiletest/run`: compile + run checks (status/stdout/stderr).
- `tests/compiletest/debuginfo`: debugger script checks (`gdb`/`lldb`).

Common directives:
- `//@ mode: c|co2|rust`
- `//@ compile-flags: ...` (repeatable, shell-split)
- `//@ compile-warning: <exact warning text>` (repeatable, for warnings that do not map to source spans)
- `//@ skip: <reason>` (skip test unconditionally)
- Inline span annotations like `//^^^^ error: ...` or `//^^^^ warning: ...` can be used in any file-based suite.
- File-based suites fail on unexpected compiler warnings; annotate intentional warnings inline.

UI directives:
- `//@ compile-fail`
- Inline span annotations on the following line, for example `//^^^^ error: message`.
- UI checks use rustc JSON diagnostics and match inline annotations by byte span.
- UI diagnostic text is checked only from inline span annotations.

Run directives:
- `//@ run-status: <int>` (default `0`)
- `//@ run-args: ...` (repeatable, shell-split)
- `//@ run-stdout: <exact text>` (`\\n` escapes are supported)
- `//@ run-stderr: <exact text>` (`\\n` escapes are supported)
- `//@ run-stdout-contains: <substring>` (repeatable)
- `//@ run-stderr-contains: <substring>` (repeatable)

Directory run tests:
- A directory containing `main.nu` is treated as a run test.
- Directives in `main.nu` use `#@`, for example `#@ run-status: 0`.
- The harness copies the whole directory to a temp workspace and runs `nu main.nu` inside it.
- `PATH` is set so `co2rustc` and `co2cc` from `target/debug` are available.
- `CO2_WORKSPACE_ROOT`, `CO2_TEST_DIR`, and `CO2_BIN_DIR` are provided to the script.
- The Nushell script is responsible for checking correctness and exiting nonzero on failure.

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
