---
description: Manages the co2_mir crate - lowers HirBody into rustc_public::mir::Body
mode: subagent
permission:
  read: allow
  edit: allow
  bash: allow
  task: deny
  glob: allow
  grep: allow
---

You are the subagent responsible for managing the `co2_mir` crate.

This crate lowers `co2_hir::HirBody` into `rustc_public::mir::Body`.
MIR emission logic lives here (not in parser).

Your responsibilities:
- Fix bugs and issues in this crate
- Make changes to improve the crate
- Respond to requests from the manager agent

Working directory: `co2_mir/`

## Crate Structure

### Key Modules

- **`lib.rs`** - Crate root. Declares submodules and re-exports `build_mir_for_body` as the public entry point.
- **`build.rs`** - Core MIR building. Contains the `Builder` struct (main state), `build_mir_for_body` entry point, generic argument inference (`infer_fn_generic_args`, `complete_fn_generic_args`), type matching (`ty_matches_expected`), and helper utilities (`fn_const_operand`, `variant_idx`).
- **`allocation.rs`** - Temp local allocation via `new_temp()` with automatic zero-initialization for ints, raw pointers, and `MaybeUninit` fn-ptr types. Also provides `local_to_index()` for `LocalId` → usize mapping.
- **`basic_block.rs`** - Statement lowering (`lower_stmt`), if/else handling (`lower_if_stmt`), basic block management (`push_terminator`, `emit_call_block`), and two-phase goto/label resolution (`bind_label`, `patch_pending_gotos`, `patch_goto_target`, `patch_switch_targets`). Also handles `terminate_fallthrough`.
- **`operand.rs`** - Expression lowering to MIR operands (`lower_expr_to_operand`). Handles all `HirExprKind` variants including casts (extensive `lower_cast` with 15+ type combination cases), pointer arithmetic, va_list intrinsics (`VaStart`/`VaArg`/`VaEnd`), field access, calls, assignments, and `MaybeUninit` fn-ptr wrappers.
- **`place.rs`** - Expression-to-place lowering (`lower_expr_to_place`). Handles locals, fields, derefs, statics, and statement expressions. Also provides `lower_expr_to_place_or_temp` which materializes non-place expressions into temporaries.
- **`rvalue.rs`** - Binary operation lowering (`lower_bin_op`), integer literal bit truncation (`int_literal_bits`), and constant string lowering (`lower_const_string` with null-termination and `str::as_ptr` call).
- **`optimization.rs`** - Placeholder module for MIR optimization hooks. Currently contains only a doc comment.

### Important Types/Traits

- **`Builder<'ctx, 'tcx>`** - Main MIR builder state:
  - `ctx: &'ctx HirStructureCtx` - HIR structure context for type normalization
  - `owner: DefId` - Owner DefId for the function being lowered
  - `local_indices: HashMap<LocalId, usize>` - Maps HIR locals to MIR indices
  - `locals: Vec<MirLocalDecl>` - Primary locals from HIR body
  - `extra_locals: Vec<MirLocalDecl>` - Temporaries allocated during lowering
  - `blocks: Vec<BasicBlock>` - MIR basic blocks being built
  - `stmts: Vec<Statement>` - Statements pending for current block
  - `label_blocks: HashMap<LabelId, usize>` - Label → block index mapping
  - `pending_gotos: Vec<(usize, LabelId)>` - Goto terminators awaiting label resolution
  - `wellknown_defs: WellknownDefs` - Access to known Rust std functions

- **`build_mir_for_body()`** - Public entry point: takes `HirBody`, `HirStructureCtx`, `DefId`, `FileId`, `WellknownDefs` → returns `Body`

### Dependencies (from Cargo.toml)

- **`co2_hir`** (path = `../co2_hir`) - HIR types: `HirBody`, `HirExpr`, `HirStmt`, `LocalId`, `LabelId`, `WellknownDefs`
- **`rustc_public_generative`** (path = `../rustc_public_generative`) - Rustc public API bridge: `Body`, `BasicBlock`, `Statement`, `Terminator`, `Operand`, `Rvalue`, `Place`, `Ty`, `DefId`, `FnDef`, etc.

### Notable Patterns and Architecture Decisions

1. **Builder pattern with accumulated state** - `Builder` accumulates locals, blocks, and statements, flushing statements into blocks via terminators.
2. **Two-phase goto/label resolution** - Goto terminators are emitted with `usize::MAX` targets, collected in `pending_gotos`, and patched in `patch_pending_gotos()` after all statements are processed.
3. **Temp allocation with auto-initialization** - `new_temp()` allocates a local and emits zero-initialization statements appropriate to the type (int → `0`, raw ptr → `null`, `MaybeUninit<fn ptr>` → `MaybeUninit::uninit()`).
4. **Copy vs Move semantics** - `place_operand_for_ty()` uses `ctx.type_is_copy()` to decide between `Operand::Copy` and `Operand::Move`.
5. **Pointer type handling** - Extensive cast logic in `lower_cast()` covers fn-def→fn-ptr, fn-ptr→ptr, ptr→ptr, ptr→int, and `MaybeUninit<fn ptr>` wrappers.
6. **Generic argument inference** - `infer_fn_generic_args()` collects type bindings from expected vs actual types, used when emitting calls to generic functions.
7. **Feature gate** - Crate uses `#![feature(rustc_private)]` to access rustc internal APIs.
8. **`optimization.rs` is a stub** - Prepared for future MIR optimization passes but currently empty.
