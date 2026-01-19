# Binary IR Format (KIR1) and Separate Compilation Artifacts

This document proposes a binary format for kscr IR and module-level artifacts suitable for separate compilation and fast loading.

Status: design proposal (not yet implemented).

This document is intentionally written as a toolchain-change proposal.
Sections marked **ToDo** are actionable items not yet implemented in the Rust engine / CLI.

## Scope and ToDo markers

- **Implemented today**: `src/ir_pack.rs` packs/unpacks whole-program `IrModule` (no header, no sections).
- **Proposed here**: `KIR1` container, intern tables, `TYPES/INTERFACE` for separate compilation.
- **ToDo**: everything in this document unless explicitly marked otherwise.

## Goals

- Fast load: minimal parsing, predictable layout.
- Forward/backward compatibility: explicit versioning, feature flags, and section table.
- Dedup & sharing: string/type/symbol interning.
- Separate compilation: module artifacts contain both interface (types/exports) and implementation (IR body).
- Content-addressed caching: stable hashes for dependency tracking.

## Non-goals

- Human readability.
- Preserving exact source spans (can be an optional debug section later).
- Cross-version ABI stability for runtime values.

---

## Terminology

- **IR**: `kscr_ir::ir::{IrModule, IrItem, IrExpr, IrPattern, ...}`.
- **Interface**: exported names + their type schemes + data/type definitions needed by importers.
- **Implementation**: IR bodies for bindings, plus any compiler metadata needed for codegen/interpretation.
- **Artifact**: a file containing either a whole-program IR bundle or a per-module compilation product.

---

## File container: `KIR1`

### Byte order and primitives

- Endianness: **little-endian**.
- Integers: `u8`, `u32`, `u64`.
- Variable-length unsigned integer: **ULEB128** (`varu32`, `varu64`) for compact tables.
- Length-prefixed bytes: `varu32 len` + `len` bytes.
- Length-prefixed UTF-8 string: `varu32 byte_len` + UTF-8 bytes (no NUL terminator).

### Alignment

- Sections are byte-aligned; optional sections may specify internal 4- or 8-byte alignment.
- Readers must not assume alignment beyond what the section declares.

### Header

```
struct FileHeader {
  u8  magic[4];        // "KIR1"
  u16 version_major;   // start at 0
  u16 version_minor;   // start at 1
  u32 flags;           // reserved; must be 0 for now
  u64 file_len_bytes;  // total file size
  u64 section_table_off;
  u32 section_count;
  u32 header_crc32;    // CRC32 of bytes [0..header_crc32)
}
```

- `version_major` bumps on breaking changes.
- `version_minor` bumps on backwards-compatible additions.

### Section table

```
struct SectionEntry {
  u32 section_id;
  u32 section_flags;     // e.g. compressed, encrypted(never), required
  u64 offset;
  u64 length;
  u64 uncompressed_len;  // 0 if not compressed
  u32 crc32;             // of (compressed) payload
  u32 reserved;
}
```

Recommended `section_id` values:

- `1  STRINGS` string table
- `2  SYMBOLS` symbol table (qualified names)
- `3  TYPES` type graph + type schemes + constraints
- `4  INTERFACE` exports/import requirements
- `5  IR` IR module bodies
- `6  DEPGRAPH` dependency hashes / cache keys
- `7  BUILDINFO` compiler version, feature flags
- `100+` optional/debug/experimental

### Compression

- `section_flags` may include `COMPRESSED_ZSTD`.
- If compressed, payload is zstd frame bytes and `uncompressed_len` must be set.
- Rationale: string/type tables can compress well while keeping random access via section boundaries.

---

## Toolchain proposal (separate compilation) (ToDo)

This section describes how `.ksif`/`.kscm` integrates into the build pipeline.

### Artifact types

- `.ksif` (interface-only): contains `STRINGS + SYMBOLS + TYPES + INTERFACE (+ DEPGRAPH, BUILDINFO)`.
- `.kscm` (module implementation): contains everything in `.ksif` plus `IR` (and optionally debug sections).
- (Optional) `.kir` (whole-program bundle): a single `KIR1` with all modules merged; convenient for distribution.

### CLI changes (ToDo)

To keep change surface small, prefer additive flags/subcommands at first.

- **ToDo: `kscr compile` outputs**
  - `kscr compile --emit ksif <file.ks>` writes `./target/kscr/<Module>.ksif`.
  - `kscr compile --emit kscm <file.ks>` writes `./target/kscr/<Module>.kscm`.
  - `kscr compile --emit kir <entry.ks>` writes a whole-program `./target/kscr/app.kir`.

- **ToDo: search path for imported artifacts**
  - `--artifact-path <dir>` (repeatable) + default `./target/kscr`.
  - Resolution order: local workspace build products first, then user paths.

- **ToDo: incremental build / cache key**
  - Compute interface hash for each module from normalized `TYPES+INTERFACE`.
  - Rebuild module if any imported interface hash changes.

### Build pipeline (ToDo)

1. Parse current module; resolve imports to module names.
2. For each import `M`, load `M.ksif` and extend the type environment with exported schemes + ADT shapes.
3. Typecheck current module; produce its interface tables.
4. Lower to IR; emit `.kscm` (or `.kir`).
5. Runtime/link step loads `.kscm` bodies for all reachable modules.

Rationale: steps 2-3 allow typechecking without importing others' IR.

---

## Shared tables

To avoid repeated strings and to enable stable references across sections, references are by index into tables.

### `STRINGS` section

Payload:

```
varu32 string_count
for i in 0..string_count:
  varu32 byte_len
  u8[byte_len] utf8
```

- String IDs are `StringId = varu32` index into this table.

### `SYMBOLS` section

A "symbol" is a *qualified* name in the compiler sense (e.g. `Prelude.map`, `Data.List.foldl`).

Payload:

```
varu32 symbol_count
for each symbol:
  varu32 module_name_sid   // StringId
  varu32 local_name_sid    // StringId
  u8 kind                 // 0=value,1=type,2=ctor,3=method,4=field
```

- Symbol IDs are `SymbolId = varu32`.
- A canonical string form may be reconstructed as `"{module}.{name}"`.

Rationale: separating module/name avoids recomputing splits and enables cheap prefix filtering.

---

## Type information: `TYPES` section

This section is designed to support separate compilation (importers need type schemes) and typeclass dictionary passing.

### Type node table

Types are stored as a DAG of nodes referenced by `TypeId`.

```
varu32 type_node_count
for each node:
  u8 tag
  ...tag payload...
```

Tags (initial set):

- `0 VAR`   : `varu32 var_id` (de Bruijn-ish within a scheme; see below)
- `1 CON`   : `varu32 name_sid` (StringId)
- `2 LIST`  : `varu32 elem_type` (TypeId)
- `3 TUPLE` : `varu32 n` + `TypeId[n]`
- `4 RECORD`: `varu32 n` + repeated `{ field_name_sid, field_type }`
- `5 RECORD_OPEN`: `varu32 n` + required fields + `varu32 rest_type`
- `6 APP`   : `varu32 head_type` + `varu32 n` + `TypeId[n]`
- `7 FUNC`  : `varu32 a` + `varu32 b`

This mirrors `src/types.rs::Ty` so decoding is direct.

### Constraints

A constraint represents a requirement like `Show a` or `EqRow r`.

```
varu32 constraint_count
for each constraint:
  varu32 class_symbol   // SymbolId, e.g. Prelude.Show
  varu32 n_args
  TypeId[n_args]
```

### Schemes

A scheme is `forall vars. constraints => ty`.

```
varu32 scheme_count
for each scheme:
  varu32 forall_count
  // vars are implicitly [0..forall_count)
  varu32 constraint_count
  ConstraintId[constraint_count]
  varu32 body_type
```

Note: `Ty::Var(u32)` in Rust is currently a global inference variable id. For serialization we pin schemes to *local* var ids.

- During emit: alpha-renumber vars inside each scheme to 0..n.
- During load: keep scheme-local ids; only expand to inference vars when needed.

---

## Stable IDs and hashing (ToDo)

Separate compilation needs IDs that are stable across builds.

### Symbol identity

Current proposal uses `SymbolId = index into SYMBOLS section`, which is only stable inside a single file.

- **ToDo: define stable symbol key** used for cross-file linking:
  - Option A (name-based): `SymbolKey = (module_name, local_name, kind)`; resolve by lookup in `SYMBOLS`.
  - Option B (hash-based): `SymbolKey = blake3("kind:module.name")[:16]`.

Recommended: start with Option A (simpler, deterministic, debuggable), add Option B later if needed.

### What gets hashed

- **ToDo: interface hash** = BLAKE3 of a normalized encoding of `TYPES+INTERFACE`.
  - Must be independent of table ordering; sort exports/import requirements by `SymbolKey`.
  - Exclude `BUILDINFO` and section offsets/CRC.

### Data/type definitions (optional initial subset)

To typecheck pattern matches across modules, importers need ADT shapes.

```
varu32 data_type_count
for each data type:
  varu32 type_symbol      // SymbolId, kind=type
  varu32 n_params
  // parameter vars are 0..n_params
  varu32 ctor_count
  for each ctor:
    varu32 ctor_symbol    // SymbolId, kind=ctor
    varu32 field_count
    TypeId[field_count]   // ctor args
```

---

## Interface section: `INTERFACE`

The interface is the minimal information another module needs to typecheck and link.

Payload:

```
varu32 export_value_count
for each exported value:
  varu32 value_symbol   // SymbolId
  varu32 scheme_id      // SchemeId

varu32 export_type_count
for each exported type:
  varu32 type_symbol    // SymbolId
  // For now, types are exported via the data-type table in TYPES.

varu32 reexport_count
for each reexport:
  varu32 symbol_id

varu32 import_requirement_count
for each requirement:
  varu32 symbol_id
  u8 required_kind      // value/type/ctor/method/field
```

Rationale:
- Exported schemes allow importers to infer types without loading implementation IR.
- Import requirements allow sanity-checking that all referenced external symbols are provided.

---

## IR section: `IR`

This stores the IR bodies. It should be decodable without consulting higher-level compiler internals.

### Encoding strategy

The existing `src/ir_pack.rs` is a good starting point but lacks:

- header/versioning
- string/symbol interning
- sectioning
- stable references (today it stores many `String`)

We propose to store IR using interned ids:

- Variables and binding names: `SymbolId` (toplevel) or `LocalId` (lambda params/let-bound names) depending on scope.
- Record field names: `StringId`.
- Constructor names: `SymbolId`.

### Locals

Within an IR body, local names are best stored as indices (like SSA-ish) rather than strings.

```
varu32 local_count
for i in 0..local_count:
  varu32 local_name_sid   // optional; 0xFFFFFFFF if omitted
```

IR nodes then reference locals by `varu32 local_id`.

**ToDo: local naming policy**

- Interpreter/codegen does not need local names; they are for debugging only.
- Emit `local_name_sid = 0xFFFFFFFF` by default; optionally include names under a debug flag.

### IR item table

```
varu32 item_count
for each item:
  u8 item_tag
  item payload
```

For initial kscr IR, only:

- `0 BINDING`:
  - `varu32 symbol_id`  (toplevel binding)
  - `varu32 expr_id`    (root expression)

### Expression table

We store expressions as a node table for sharing and fast random access.

```
varu32 expr_count
for each expr:
  u8 tag
  ... payload ...
```

Tags reflect `kscr_ir::ir::IrExpr`:

- `0 UNIT`
- `1 INTEGER` : `StringId` (kept as string for now, matches current IR)
- `2 FLOAT64` : `StringId`
- `3 BOOL`    : `u8`
- `4 STRING`  : `StringId`
- `5 CHAR`    : `u32`
- `6 VAR_LOCAL`: `varu32 local_id`
- `7 VAR_TOP`  : `varu32 symbol_id`
- `8 LAMBDA`  : `varu32 param_count` + `LocalId[param_count]` + `ExprId body`
- `9 APPLY`   : `ExprId func` + `varu32 n_args` + `ExprId[n_args]`
- `10 IF`     : `ExprId cond` + `ExprId then` + `ExprId else`
- `11 LET`    : `varu32 n_bind` + repeated `{ LocalId, ExprId }` + `ExprId body`
- `12 CASE`   : `ExprId scrut` + `varu32 n_arms` + `CaseArmId[n_arms]`
- `13 IO_BIND`: `ExprId action` + `LocalId param` + `ExprId body`
- `14 IO_THEN`: `ExprId first` + `ExprId then`
- `15 CONS`   : `ExprId head` + `ExprId tail`
- `16 LIST`   : `varu32 n` + `ExprId[n]`
- `17 TUPLE`  : `varu32 n` + `ExprId[n]`
- `18 RECORD` : `varu32 n` + repeated `{ field_name_sid, ExprId }`
- `19 CHECKED_CAST`: `ExprId expr` + `u8 cast_target`

### Patterns and case arms

Case arms reference a pattern table similarly:

```
varu32 pat_count
for each pattern:
  u8 tag
  ...

varu32 case_arm_count
for each arm:
  varu32 pat_id
  u8 has_guard
  (ExprId guard)?
  ExprId body
```

Pattern tags mirror `IrPattern`, using StringId/SymbolId/LocalId where applicable.

---

## Dependency and caching: `DEPGRAPH`

This section supports incremental builds.

Payload:

```
varu32 dependency_count
for each dependency module:
  varu32 module_name_sid
  u8 hash_kind            // 0=blake3
  u8 hash_len
  u8[hash_len] digest

u8 hash_kind
u8 hash_len
u8[hash_len] this_artifact_digest
```

Recommended hash: BLAKE3 of the *normalized interface* (and optionally implementation for full rebuild checks).

---

## Separate compilation artifact: `.kscm` (KSC Module)

Proposed per-module file extension: `.kscm` (kscr compiled module).

A `.kscm` file is a `KIR1` container that **must** contain:

- `STRINGS`
- `SYMBOLS`
- `TYPES`
- `INTERFACE`

and **may** contain:

- `IR` (when distributing implementation)
- `DEPGRAPH`
- `BUILDINFO`

### Split outputs

To support "interface-only" builds (faster importing), allow two outputs:

- `Foo.ksif` interface file: `KIR1` with `TYPES+INTERFACE` only.
- `Foo.kscm` implementation file: includes `IR`.

The compiler can choose whether importers require only `.ksif` (for typechecking) and the linker/runtime loads `.kscm` for execution.

---

## Import pipeline (proposed)

1. Parse and resolve module headers/imports.
2. For each imported module `M`:
   - Load `M.ksif` (or `M.kscm`) and read `TYPES+INTERFACE`.
   - Add exported schemes and data definitions into the type environment.
3. Typecheck current module; produce its own interface.
4. Lower to IR and emit `IR` section.

This keeps typechecking independent from loading all dependencies' IR.

---

## Compatibility rules

- Readers must reject unknown `version_major`.
- Readers may ignore unknown section IDs unless marked `required`.
- Adding a new IR/Type tag requires bumping `version_minor` and defining decoding behavior.
- Removing/changing existing tags requires bumping `version_major`.

---

## Relationship to current code

- Current packed IR: `src/ir_pack.rs` encodes `IrModule` as a flat stream with u8 tags + strings.
- Proposed `KIR1`:
  - wraps packed data with header + section table,
  - interns strings/symbols,
  - adds `TYPES` and `INTERFACE` for separate compilation.

Implementation can proceed incrementally:

1. Introduce `KIR1` container and `STRINGS` + `IR` using interned ids.
2. Add `TYPES` + `INTERFACE` emission for exported values.
3. Teach compiler pipeline to load imported interfaces from `.ksif`.

**ToDo: staging notes**

- Stage 1 should be usable as a drop-in replacement for `ir_pack` in `kscr compile` (whole-program mode).
- Stage 2 should include enough `TYPES` to typecheck imports (schemes + ADT shapes).
- Stage 3 requires changes in module resolution/typechecker to consult `.ksif`.

