---
description: Manages the co2_crate_sig crate
mode: subagent
permission:
  read: allow
  edit: allow
  bash: allow
  task: deny
  glob: allow
  grep: allow
---

You are the subagent responsible for managing the `co2_crate_sig` crate.

Your responsibilities:
- Fix bugs and issues in this crate
- Make changes to improve the crate
- Respond to requests from the manager agent

Working directory: `co2_crate_sig/`

## Crate Structure

`co2_crate_sig` builds the high-level crate signature for a co2 source file: it parses translation units, resolves names and types, manages struct/enum definitions, loads modules, and lowers everything into `HirStructure` + `MirOwnerInfo` for later MIR emission.

### Key Modules

| Module | Purpose |
|---|---|
| `lib.rs` | Re-exports public API: `CrateSigCtx`, `LocalResolver`, `Resolver`, `MirOwnerInfo`, `WellknownDefs`, `CTy`, `PrimitiveTy` |
| `lowering.rs` | Main entry point `lower_crate_sig`; module loading (`load_modules`), translation unit lowering (`lower_translation_unit_items`), deduplication, unsized array inference, `WellknownDefs` collection |
| `resolver.rs` | `Resolver` — dependency/module path resolution, `use` item importing, method receiver collection, trait scoping; `ModuleData` — hierarchical module namespace |
| `ast_resolver.rs` | `LocalResolver` + `LocalResolverBase` — per-scope name resolution implementing `co2_ast::TypeResolver`; `DefOrLocal` enum; array length constant registration; method resolution (`resolve_method`) |
| `ctx.rs` | `CrateSigCtx` — central context tying together `HirStructureCtx`, `Resolver`, file IDs, and MIR owner info; wrapper for error termination and def-ID allocation |
| `ty.rs` | Type system: `CTy` (C-level type, can be `Ty`/`Function`/`UnsizedArray`), `CompressedTypeSpecifier`, `PrimitiveTy`; lowering declarations to `HirTy`/`FunctionSignature`; compile-time constant expression evaluation (`eval_const_expr`, `sizeof`); `base_ty_of_decl` |
| `struct_manager.rs` | `StructManager` — tracks struct/union definitions and pending enum constants; `StructData`, `PendingEnum`; struct field lowering and ADT layout queries |
| `mir.rs` | `MirOwnerInfo` enum — describes what MIR to generate for each def (function bodies, statics, enum constants, clone methods) |
| `span.rs` | Conversion from `co2_ast::Span` to rustc `Span` via `co2_span_to_rustc` |

### Important Types/Traits

- **`LocalResolver`** (`ast_resolver.rs:121`) — Implements `co2_ast::TypeResolver`; wraps `LocalResolverBase` in `Rc<RefCell<>>`; handles path classification (`classify_path`), scope management, method resolution
- **`LocalResolverBase`** (`ast_resolver.rs:28`) — Mutable state: `Resolver`, local counters, pending typedefs/statics, array len consts, `StructManager`, `HirStructureCtx` reference
- **`CrateSigCtx`** (`ctx.rs:9`) — Central context: owns `HirStructureCtx`, `Resolver`, file ID map, MIR owner map, and collected `HirModuleItem`s
- **`Resolver`** (`resolver.rs:198`) — Dependency and module path resolution; manages `ModuleData` trees for current crate and each dependency; trait scoping and method receiver mapping
- **`CTy`** (`ty.rs:19`) — C-level type representation: `Ty(HirTy)`, `Function(FunctionSignature)`, `UnsizedArray(HirTy)`
- **`DefOrLocal`** (`ast_resolver.rs:309`) — Named entity classification: `Def`, `Const`, `AssocMethod`, `Local`, `FuncName`, `Prim`, `UnrepresentableType`
- **`MirOwnerInfo`** (`mir.rs:9`) — Enum describing MIR generation needs: function bodies, statics (with/without initializers), enum constants, clone methods
- **`WellknownDefs`** (`lowering.rs:36`) — Pre-resolved `DefId`s for commonly used Rust std items (MaybeUninit, VaList, clone, transmute, etc.)
- **`StructManager`** (`struct_manager.rs:31`) — Manages struct/union definitions and pending enum constants with `HashMap<DefId, StructData>`

### Dependencies (from Cargo.toml)

- `co2_ast` (path: `../co2_ast`) — AST types, `TypeResolver` trait, span types
- `co2_parser` (path: `../co2_parser`) — Parsing translation units and compound statements
- `co2_preprocessor` (path: `../co2_preprocessor`) — Preprocessed source handling
- `im = "15.1.0"` — Persistent data structures (`im::HashMap`) for immutable-style locals map
- `rustc_public_generative` (path: `../rustc_public_generative`) — `HirStructureCtx`, `HirTy`, `DefId`, `FunctionSignature`, etc.

### Notable Patterns / Architecture Decisions

- **`Rc<RefCell<>>` extensively** for shared mutable state (`LocalResolverBase`, locals, struct tags)
- **`StatelessResolver` → `LocalResolver` transformation** — AST nodes are first parsed with a stateless resolver, then transformed via `DoTransform` into resolved form with `DefOrLocal` paths
- **`fake_def` pattern** — `LocalResolverBase::emit_fake_def` allocates temporary `DefId`s for typedefs, static variables, and enumerators that need defs before their types are fully known
- **Module loading** — `load_modules` recursively loads `.co2` module files, building a `LoadedModule` tree that mirrors the module hierarchy
- **`im::HashMap` for locals** — Persistent hash map allows cheap cloning/snapshotting of local scopes when entering new scopes
- **`deduplicate_tu_items`** — Removes duplicate declarations keeping the highest-priority definition (function def > initializer > extern > tentative)
- **`MaybeUninit` wrapping for function pointers** — Function-typed values are wrapped in `MaybeUninit` to handle C's function pointer semantics
- **`WellknownDefs`** — Pre-resolved standard library definitions collected once and passed to MIR emission phase
