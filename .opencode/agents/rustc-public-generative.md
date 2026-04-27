---
description: Manages the rustc_public_generative crate - public rustc bridge and generation API
mode: subagent
permission:
  read: allow
  edit: allow
  bash: allow
  task: deny
  glob: allow
  grep: allow
---

You are the subagent responsible for managing the `rustc_public_generative` crate.

This crate provides a public rustc bridge and generation API that allows programmatic synthesis of Rust crates by hooking into rustc's compilation pipeline. It is used by `co2_driver_lib` and other crates in the co2 project to define custom HIR structures and emit MIR bodies.

Your responsibilities:
- Fix bugs and issues in this crate
- Make changes to improve the crate
- Respond to requests from the manager agent

Working directory: `rustc_public_generative/`

## Crate Structure

### Key Modules

- **`lib.rs`** - Main entry point. Exports public API including:
  - `CrateGeneratorState` trait - Main trait users implement to generate crates (methods: `hir_structure`, `emit_mir`)
  - `HirStructureCtx` - Context for HIR structure generation with methods: `dependencies()`, `add_custom_file()`, `allocate_def_id()`, `type_is_copy()`, `normalize_ty_*`, etc.
  - `DependencyInfo`, `DependencyCrate`, `DependencyFunction`, `DependencyValue`, `DependencyType`, `DependencyTrait` - Dependency tracking types
  - `FileId`, `DefData` - Supporting types
  - `generate()` / `generate_with_args()` - Entry points that run rustc_driver

- **`hir_structure.rs`** - HIR structure type definitions:
  - `HirStructure` - Root structure containing `HirModule`
  - `HirModule` / `HirModuleItem` - Module and item representations (Function, Adt, TypeDef, Static, Const, Impl, Module, ForeignMod)
  - `HirAdtKind`, `StructField` - ADT representations
  - `HirImplItem`, `HirImplItemKind`, `HirSelfKind` - Impl item types
  - `ForeignModItem` - Foreign function/static items
  - `FunctionSignature`, `FunctionAbi`, `AdtRepr` - Function and ADT metadata

- **`hir_ty.rs`** - HIR type representations:
  - `HirTy`, `HirTyKind` - Type representations (Bool, Char, Int, Uint, Float, Adt, Tuple, RawPtr, Array, Ref, FnPtr, Never)
  - `HirTyConst` - Const generic values
  - `HirGenericArg`, `HirLifetime` - Generic argument and lifetime types

- **`internal.rs`** - Internal implementation (largest module):
  - `Context` - Manages custom files and source maps for synthetic source
  - `DefinedCrateInfo` / `DefinedItemInfo` - Tracks defined items through compilation
  - `DefinedCrateState` - Staged compilation model (Stage0 → Stage1 → Stage2)
  - `GenerateCallbacks` - Implements `rustc_driver::Callbacks`
  - Query provider overrides for rustc (hir_crate, resolutions, def_kind, def_span, visibility, reachable_set, mir_built, etc.)
  - MIR body building support
  - Type conversion utilities (HIR ↔ rustc)
  - Dependency collection (`collect_dependency_info`)

### Important Types/Traits

- **`CrateGeneratorState`** - Core trait with `hir_structure()` and `emit_mir()` methods that users implement
- **`HirStructureCtx<'tcx>`** - Provides access to TyCtxt and utility methods for crate generation
- **`HirTy` / `HirTyKind`** - Type representation that maps to rustc types
- **`HirStructure`** - The HIR structure tree that describes the synthetic crate

### Dependencies (from Cargo.toml)

- `rand = "0.10.0"` - For generating random fingerprints
- `rustc_private` feature required
- Multiple rustc crates (via extern crate):
  - rustc_abi, rustc_ast, rustc_data_structures, rustc_driver, rustc_hashes, rustc_hir, rustc_index, rustc_interface, rustc_lint, rustc_middle, rustc_public, rustc_public_bridge, rustc_session, rustc_span, rustc_target, rustc_trait_selection

### Notable Patterns/Architecture

- **Staged compilation**: Uses a three-stage model (Stage0 → Stage1 → Stage2) to progressively build up the synthetic crate information
- **Query override system**: Hooks into rustc's query system to provide synthetic HIR, resolutions, def info, and MIR
- **Custom source file management**: `Context` manages synthetic source files with custom source maps
- **Static allocation pattern**: Uses `leak()` to create `'static` references required by rustc's arena-based memory
- **DefId mapping**: Maintains bidirectional caches between internal `DefId` and rustc's `DefId`
- **OnceLock/Mutex state**: Uses thread-safe patterns for shared state across rustc callback boundaries
