# AI Context Dump

We dump the context here after each task.

## Overview
- co2 is a new C-compatible frontend that lowers `co2_parser` → `co2_hir` → `co2_mir` → `co2_driver_lib` and emits `co2`/`co2c` binaries.
- The workspace intentionally keeps parser concerns in `co2_parser`, HIR construction inside `co2_hir`, MIR lowering in `co2_mir`, and orchestration + wiring in `co2_driver_lib`.
- `rustc_public_generative` provides the bridge into rustc (HirStructure, HirTy, etc.), and we try to keep downstream crates generic and not rustc-dependent beyond that API.

## Recent instructions and context
- Statics without explicit initializers must be filled via `mem::zeroed()` instead of skipping them, because runtime expects allocs to exist.
- Integer casts in expression lowering need support (e.g., returning a `char` from `int main`).
- `PtrDiff` currently misbehaves and there was a question around `offset_from`, but intrinsics were discouraged.
- Pointer-to-pointer implicit casts are required for C compatibility.
- Nested anonymous structs/unions, anonymous fields, and unions should be handled correctly; several tests live in `tests/compiletest/run/basic_c.c`.
- The code currently creates fake unions via struct-like helpers; we were asked to switch to real unions and extend `rustc_public_generative::HirAdtKind` before adding the generated unions.
- Debug points should move into a dedicated file.
- All structs in the `co2` binary (and related HIR helpers) were requested to implement `Copy`, implying they should only contain copyable data.
- Document generation should live in `docs/ai_dump.md`; this file captures the latest context for future calls.

## Tactical notes
- `cargo test -q` currently runs cleanly (see the latest run from this session).
- `basic_c.c` already contains the union/anonymous struct tests mentioned most recently, so rely on that file for coverage rather than adding new copies.
- `DetectCallbacks` in `co2/src/detect.rs` is stateful and not `Copy` because it stores a `PathBuf`, so the request to make "all structs in co2 Copy" needs clarification about what subset of structs we are targeting.

## Outstanding questions
1. Which structs exactly need to become `Copy`? Many existing structs own `String`, `Vec`, or `PathBuf`, so they are not trivially `Copy`.
2. Are there specific places where we still fake unions that should switch to the real `HirAdtKind::Union` path?

## Next time reminders
- Always use `apply_patch` for file edits.
- Keep track of `ai_dump.md` contents to avoid forgetting the instructions captured here.
