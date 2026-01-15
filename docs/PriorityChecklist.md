# Priority Checklist (Agent Memory)

This file serves as a **priority definition (P0 and beyond)** and progress tracker for this repository, ensuring that AI does not misinterpret or mix up priority numbers in future conversations/sessions.

- Rule: Do not use "P numbers" that are not listed here.
- Rule: Items that may differ from Haskell specifications should be categorized as **P3 (skippable)** and not implemented unless explicitly instructed by the user.

Last updated: 2026-01-09

---

## P0 — End-to-end smoke tests (import traversal)
Purpose: Ensure that the execution system works end-to-end for "multi-file", "import traversal", "export/import boundaries", and "qualified references" as the highest priority.

Status: In progress (continuous additions allowed, but avoid re-implementing completed items listed below).

### Done
- [x] run: Successfully execute across import traversal (A→B→Main) (commit: `34c439d`)
- [x] run: Transitive import + qualified reference (`import A as OM; OM.x`) works (commit: `34c439d`)
- [x] run: `import A` allows both unqualified references (`x`) and module qualifiers (`A.x`) (commit: `4904ba6`)
- [x] typecheck: Export/import boundaries are enforced, and non-exported items are inaccessible (commit: `4904ba6`)
- [x] run: Resolve name conflicts using qualified imports (`import A as A1` / `import B as B1`) (commit: `f276e02`)
- [x] typecheck: Detect cyclic imports, and the error message includes `cyclic imports` (commit: `f276e02`)

### Next
- [x] import traversal: Add more realistic smoke tests spanning data/constructors + case + do (tests/P0/Main.ks)

## P1 — Exceptions via IO (throw/catch/try)
Purpose: Implement `throw/catch/try` in the IO layer, ensuring propagation, catching, and try (conversion to Either) through smoke tests.
- [x] Implementation and testing complete (commit: `4d0c477`)

## P2 — Braces / Semicolons surface syntax
Purpose: Accept minimal brace/semicolon syntax in addition to existing indentation blocks.
Targets:
- `do { ...; ... }`
- `let a = ...; b = ... in ...`
- `where { a = ...; b = ... }`
- [x] Implementation and testing complete (commit: `4d0c477` + additional tests/fixes)

## P12 — Haskell-style function clauses/guards (parser desugar)
Purpose: Introduce Haskell-style syntax (multiple clauses, guards, clauses in let/where) by desugaring in the parser without extending the AST, minimizing changes to type inference/IR/runtime.

Status: Complete (MVP)
- [x] top-level: Aggregate function clauses with the same name into a single binding (lambda + case) (commit: `71c33a7`)
- [x] Accept guarded function clauses (`f x | guard = body`) and map them to `CaseArm.guard` (commit: `5bf35e1`)
- [x] Similarly aggregate clauses in let/where and desugar (commit: `5bf35e1`)
- [x] Resolve ambiguity of `|`: Or-patterns in function arguments require parentheses (e.g., `f (0 | 1) = ...`) (commit: `5bf35e1`)

## P14 — Interactive REPL (MVP)
Purpose: Enable interactive evaluation of expressions/definitions via `kscr repl`, speeding up language experimentation.

Scope (MVP):
- Single-line input (expression or `name = expr`)
- Commands: `:type <expr>`, `:quit`
- Reuse the existing pipeline (parse→typecheck→IR→run), even if full typechecking is performed each time.

Status: Done
- [x] Implementation + minimal tests (REPL core refactored for unit testing) (commit: `acd80da`)

## P15 — REPL: Readline + module loading
Purpose: Improve interactivity (line editing/history) and enable module loading in the REPL following existing import rules.

Scope (MVP):
- Readline: Line editing with history (via `rustyline`)
  - NOTE: `rustyline` includes unsafe dependencies, so it is only enabled with `--features readline` (default is stdio REPL).
- `:load <path>`: Resolve imports relative to the specified file's directory and evaluate (generate Main overlay).
- `:modules`: Display currently loaded modules.

Status: Done
- [x] Implementation + tests + gate passing (commit: `29cb45f`)

## P16 — Typeclasses roadmap (Deriving → Class/Instance)
Purpose: Gradually introduce Haskell-style `class` / `instance` by first implementing "derivable classes" to establish a foundation for the language/implementation.

### Phase 0 (Current)
- Constraints are internally fixed (`Show` / `ShowRow` / `Lacks`), and user-defined classes/instances cannot be written.
- `Show` is resolved structurally (requires `Show` for fields in data declarations), and runtime display depends on built-ins.

### Phase 1 (MVP): Explicit `deriving Show`
Purpose: Only data types explicitly marked with `deriving Show` satisfy the `Show` constraint (aligning with Haskell).
- Syntax: `data T a = ... deriving (Show)` / `deriving Show`
- Implementation: Parser retains deriving lists, and typechecking resolves `Show` based on `deriving Show`.
- Prerequisites:
  - Data declarations are consistently collected during import traversal (ensured by P0/P13D).
  - Default builds do not introduce unsafe dependencies (gate maintained).

### Phase 2: `deriving Eq` (no class/instance yet)
Purpose: Introduce `Eq` for data types in a "structural" manner, extending `(==)` to data types.
- Initially limited to data types with deriving.
- Ensure consistency in constraints/inference/runtime (Eq dictionaries are not yet introduced; built-ins suffice).

### Phase 3: `class`/`instance` (Final Goal)
Purpose: Implement user-defined classes/instances and constraint-based polymorphism (including dictionary passing).
- Syntax: `class C a where ...` / `instance C T where ...`
- Typechecking: Instance environment + resolution (candidate search, ambiguity/duplication, scoping).
- IR/Runtime: Dictionary representation (records) and invocation transformation.
- Compatibility: Deriving from Phases 1/2 should eventually desugar into forms like `instance Show (T a)`.

MVP scope (Phase 3):
- Allow **non-ground instances** with constraints, e.g. `instance (C a) => C (Maybe a) where ...`.
- Coherence: **no overlap / no duplicates** (if two instances can apply to the same wanted constraint, it is an error).
- Defaulting: do not require call sites to be ground; keep dictionary passing as the primary mechanism.
- Restriction (MVP): user-defined `class Show` / `class Eq` declarations are **forbidden** to avoid clashing with built-in deriving support.

Status:
- [x] Phase 1: `deriving Show` implementation + tests + gate passing (commit: `e72ce94`)
- [x] Phase 2: `deriving Eq` implementation + tests (commit: `86066b8`)
  - ✅ `deriving Eq` and `deriving (Eq, Show)` syntax support
  - ✅ Eq constraint resolution and dictionary passing
  - ✅ `(==)` and `(/=)` implementation
  - ✅ Structural Eq (primitive, lists, tuples, records, data types)
- [ ] Phase 3+: `class` / `instance` syntax (future)

**Current status**: Phase 2 complete. Both Show and Eq implemented with dictionary passing.

## P13 — Align imports/exports with Haskell (recommended order)
Purpose: Gradually improve the "feel", clarity of name resolution, and specification of imports/exports to align with Haskell.

Recommended order:
1) P13C (diagnostic improvements) → 2) P13D (resolution/specification) → 3) P13A (surface syntax) → 4) P13B (export granularity)

- [x] **P13C: Diagnostic improvements for import name resolution** (e.g., explain conflicts, suggest qualifiers, show allowed qualifiers for unknown qualifiers).
  - Done: Better conflict/qualifier errors + tests (commit: `bac0b11`)
- [x] **P13D: Fix import resolution rules/specification** (e.g., resolution order, module name ↔ path rules + smoke tests).
  - Rule: Try `<importer_dir>/<Module>.ks` then `<repo>/stdlib/<Module>.ks`; on miss, error shows tried paths.
  - Rule: Imported module must declare `module <Module> where` (mismatch is an error).
  - Tests: Local-over-stdlib shadowing, tried-paths in error, module name mismatch (commit: `7985565`)
- [x] **P13A: Align import syntax with Haskell** (e.g., `import qualified A as OM`).
  - Behavior: `import qualified` is qualified-only; `import A as OM` is unqualified + OM qualifier.
  - Tests: Updated existing smokes + added unqualified + qualifier smoke (commit: `2699f9e`)
- [x] **P13B: Strengthen export granularity** (e.g., `export T(..)` / `export T(C1, C2)`).
  - Done: Export spec parsing + constructor subset + qualified import cannot bypass (commit: `c11239d`)

## P3 — Specifications that may differ from Haskell (skippable)
Purpose: Even if documented, items that may differ from Haskell or have ambiguous designs should not be implemented without explicit instructions.
Candidates (examples):
- Other syntax/semantics that may differ from Haskell.
- [ ] Skip by default (mistakenly implemented items have been reverted: `4996536`).

## P4 — Numeric/Doc consistency (MVP)
Purpose: Eliminate discrepancies between current implementation (runtime/stdlib) and documentation, ensuring safe behavior (e.g., overflow handling) as an MVP.
Contents:
- Make Integer operations checked to raise runtime errors on overflow.
- Align String/Integer MVP specifications in docs with the current implementation.
- [x] Complete (commit: `94a57f5`)

## P5 — Backend numeric types + checked casts at boundaries (next target)
Purpose: Implement "LLVM-aligned backend numeric types" and "checked casts at boundaries" as described in docs (ImplementationPlan/TypeSystem/IR) within an MVP scope.
Scope (proposal):
- Introduce `i32/i64` and `f32/f64` equivalents (internal use) in type representation (minimal surface syntax is acceptable).
- Represent checked casts at literal/FFI boundaries (FFI itself can be deferred) in IR/runtime, raising runtime errors on failure.
- Tests: Smoke tests for successful/failed casts (overflow/invalid).

Status: Complete (MVP)
- [x] Runtime values: Store `Integer`/`Float64` as `i64`/`f64` (parse at literal boundaries, raise runtime errors on failure).
- [x] Annotation boundaries: Represent `(:: i32/i64/f32/f64)` as checked casts in IR, raising runtime errors on failure.
- [ ] Additional boundaries (e.g., FFI) deferred.

## P6 — Minimal FFI boundary (unsafe-free scaffold)
Purpose: Since real C ABI calls may require `unsafe`, first implement **FFI boundary behavior (checked casts for arguments/returns)** in a form that can be smoked with built-ins.
Scope:
- Add built-ins like `ffiAddI32 :: i32 -> i32 -> i32` requiring backend numeric types.
- Check range/overflow at call boundaries, raising runtime errors on failure.
- Tests: Normal cases, out-of-range arguments, overflow.

Status: Complete (MVP)
- [x] Added `ffiAddI32`/`ffiAddF32` built-ins.
- [x] Checked range/overflow at call boundaries.
- [x] Added smoke tests.

---

## P7 — Unsafe boundary isolation + tracing
Purpose: Isolate the minimal `unsafe` (FFI/special optimizations/BigInt, etc.) under **feature flags** and enable tracing to observe when `unsafe` is used during debugging.
- Implementation: Enable with `--features unsafe_ffi/unsafe_bigint`, etc. (default build is off).
- Observation: When executed with `KSCR_DEBUG_UNSAFE=1`, output `unsafe` boundary crossings to stderr.

Status: Complete (MVP)
- [x] Added feature flags (`unsafe_ffi` / `unsafe_bigint`).
- [x] Output tags once per execution with `KSCR_DEBUG_UNSAFE=1`.

## P8 — Optional BigInt Integer backend
Purpose: Ensure `Integer` semantics are always arbitrary precision (custom safe backend) while optionally enabling `num-bigint` under **feature flags** for performance/testing purposes.
Scope:
- Default: Custom variable-length Integer backend (no unsafe).
- `--features unsafe_bigint`: `num-bigint` backend (optional dependency / isolated crate).
- Ensure range checks work at existing boundaries (`:: i32/i64`, `ffiAddI32`, etc.).

Status: Complete (MVP)

## P9 — Real C ABI FFI (unsafe isolated)
Purpose: Implement real C ABI calls under **feature flags with unsafe isolation**, verifying boundary behavior (e.g., String → C string, return type/range) as an MVP.

Scope (MVP):
- Add built-ins only enabled with `--features unsafe_ffi`.
  - Example: `ffiPuts :: String -> IO i32` (calls C standard library `puts`).
- String boundary: Interior NUL is an error.
- Observe `unsafe used: ffiPuts` with `KSCR_DEBUG_UNSAFE=1`.
- Tests: Smoke tests with the feature enabled.

Notes:
- Since `cfg(feature = "unsafe_ffi")` alone causes `cargo geiger` to detect unsafe code, **isolate unsafe into a separate crate (optional dependency)**.
- This ensures that default builds pass mandatory gates (`cargo geiger`) while enabling unsafe only with `--features unsafe_ffi`.

Status: Complete (MVP)
- [x] Added `ffiPuts` as a built-in enabled only with `--features unsafe_ffi`.
- [x] Isolated unsafe into a separate crate (optional dependency) `kscr_unsafe_ffi`.
- [x] Added smoke tests with the feature enabled.

Operation (gates):
- Default mandatory: `cargo test && cargo clippy -- -D warnings && cargo geiger && cargo +nightly udeps`.
- Optional (with unsafe_ffi enabled): `cargo test --features unsafe_ffi` / `cargo geiger --features unsafe_ffi`.

## P10 — Unsafe features gate policy
Purpose: Establish CI/operational rules for verifying **unsafe features** like `unsafe_ffi` / `unsafe_bigint`, ensuring no breakage.

Policy (MVP):
- Default build (no features):
  - `kscr` main crate is marked with `#![forbid(unsafe_code)]`, prohibiting unsafe code entirely.
  - Mandatory gates: `cargo test && cargo clippy -- -D warnings && cargo geiger && cargo +nightly udeps`.
- Unsafe features (e.g., `unsafe_ffi` / `unsafe_bigint`):
  - Isolate unsafe into separate crates (optional dependencies).
  - Run separate jobs for `cargo test --features ...`.
  - Monitor `cargo geiger --features ...` to ensure no new unsafe code or dependencies are introduced.

Status: Complete (MVP)
- [x] Policy established (as above).
- [ ] CI integration (requires separate task if CI needs to be introduced/updated for this repository).

## P11 — Isolate BigInt backend into subcrate
Purpose: Isolate `unsafe_bigint` (arbitrary precision Integer) into a separate crate (optional dependency), removing `num-bigint` dependency from the `kscr` main crate to stabilize default gates.

Scope (MVP):
- Add `crates/kscr_unsafe_bigint` (encapsulate `num-bigint` dependency here).
- `--features unsafe_bigint` enables `dep:kscr_unsafe_bigint`.
- Remove `num_bigint::...` references from `src/ir.rs`, using the subcrate API instead.

Status: Complete (MVP)
- [x] Added subcrate.
- [x] Feature wiring.
- [x] Existing tests pass with `--features unsafe_bigint`.

## Notes
- From now on, if asked to "implement P5", refer to **P5 in this file**.
- If a new priority is needed, update this file before starting work.
