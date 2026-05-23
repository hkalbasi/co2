# co2 compiletest

Each test file is self-contained and uses inline directives prefixed with `//@`.

The harness scans `tests/compiletest` recursively. Directory names are only
organization and filter conveniences; test behavior is inferred from directives.

Common directives:
- `//@ mode: c|co2|rust`
- `//@ compile-flags: ...` (repeatable, shell-split)
- `//@ compile-warning: <exact warning text>` (repeatable, for warnings that do not map to source spans)
- `//@ skip: <reason>` (skip test unconditionally)
- Inline span annotations like `//^^^^ error: ...` or `//^^^^ warning: ...` can be used in any file-based suite.
- File tests fail on unexpected compiler warnings; annotate intentional warnings inline.

Compile-fail tests:
- `//@ compile-fail`
- Inline span annotations on the following line, for example `//^^^^ error: message`.
- Compile-fail checks use rustc JSON diagnostics and match inline annotations by byte span.
- Diagnostic text is checked only from inline span annotations.

Run tests:
- Any file test without `//@ compile-fail` is compiled and run.
- `//@ run-status: <int>` (default `0`)
- `//@ run-args: ...` (repeatable, shell-split)
- `//@ run-stdout: <exact text>` (`\\n` escapes are supported)
- `//@ run-stderr: <exact text>` (`\\n` escapes are supported)
- `//@ run-stdout-contains: <substring>` (repeatable)
- `//@ run-stderr-contains: <substring>` (repeatable)
- `//@ run-miri` runs `mode: co2` tests through `co2cargo miri run` after the normal run and checks the same run status/output expectations. The test is skipped if `cargo-miri` is unavailable.
- `//@ miri-error` runs `mode: co2` tests through Miri after the normal run and expects Miri to fail with inline `//^^^^ error: ...` annotations.

Directory run tests:
- A directory containing `main.nu` is treated as a run test.
- Directives in `main.nu` use `#@`, for example `#@ run-status: 0`.
- The harness copies the whole directory to a temp workspace and runs `nu main.nu` inside it.
- `PATH` is set so `co2rustc` and `co2cc` from `target/debug` are available.
- `CO2_WORKSPACE_ROOT`, `CO2_TEST_DIR`, and `CO2_BIN_DIR` are provided to the script.
- The Nushell script is responsible for checking correctness and exiting nonzero on failure.

Run harness:
- `cargo run -p co2_test_harness`
- `cargo run -p co2_test_harness -- 'tests/compiletest/**/*.c'`
