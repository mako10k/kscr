# Implementation Plan

This document tracks the implementation plan for **kscr** from the current repository state.

Policy:
- If implementation/tests/CI and docs disagree, implementation is the source-of-truth.
- This file is not a changelog; it defines baseline, gaps, and execution order.

Last updated: 2026-03-05

---

## Baseline Status (As Implemented)

The codebase is end-to-end runnable (lexer -> parser -> typechecker -> IR -> runtime) and currently includes:
- Multi-file module resolution with import/export boundaries and qualified imports.
- Typeclass support including user-defined `class`/`instance`, deriving-based `Show`/`Eq`, and dictionary passing.
- REPL (`kscr repl`) with stdio mode and optional readline (`--features readline`), plus `:load` and `:modules`.
- Optional unsafe-isolated features via subcrates (`unsafe_ffi`, `unsafe_bigint`).
- Optional LLVM text-generation path (`--features llvm`).
- CI gates active on main (test/fmt/clippy + required unsafe crates checks + phase3d typeclass regression job).

Reference docs:
- `docs/PriorityChecklist.md` (execution-first priorities)
- `docs/TypeSystem.md`
- `docs/LanguageSemantics.md`
- `docs/IntermediateRepresentation.md`

---

## Milestone Status Matrix

## M0-M2 Foundations, Syntax, HM Core
Status: Done (operational baseline).
- Lexer/parser/type inference pipeline is active and tested.
- Module syntax and import traversal are production paths.

## M3 Typeclasses
Status: Done for current scope.
- Done:
  - User-defined `class` / `instance` parsing and typechecking paths are implemented.
  - Constraint representation and dictionary passing for built-in and user-defined class paths.
  - `deriving Show`, `deriving Eq`, and `deriving (Eq, Show)`.
  - Reserved-class guardrails (`Show`/`Eq` redefinition rejection).
- Remaining future hardening:
  - Better ambiguity/fallback traceability and additional ergonomics (tracked in backlog).

## M4 IR Elaboration
Status: Done for current interpreter path.
- Typed AST to IR lowering is active for current language features.
- Import/no-flattening behavior is already integrated into current architecture.

## M5 Runtime + Interpreter
Status: Done for current language scope.
- Thunks, lazy evaluation, IO sequencing, and exception support are operational.

## M6 FFI Boundaries
Status: MVP complete; broad expansion pending.
- Done:
  - Checked boundary builtins (`ffiAddI32`, `ffiAddF32`).
  - Unsafe-isolated real C ABI MVP (`ffiPuts`) under `--features unsafe_ffi`.
- Pending (future extension):
  - Wider C ABI surface and richer ownership/encoding policies.

## M7 LLVM Backend
Status: Partial/optional.
- Optional LLVM-related path exists.
- Full lowering/JIT parity with interpreter is not yet a completed project milestone.

## M8 Tooling and UX
Status: Partial.
- Implemented: `kscr-lsp` + VS Code client baseline with diagnostics/hover/definition/documentSymbol/completion/references/rename/semantic tokens.
- Pending: formatter/linter scope and wider IDE UX polish.

---

## Corrections Applied vs Older Plan Text

1. Typeclasses were previously labeled as partially complete due to missing user `class` / `instance`.
- Corrected: user `class` / `instance` baseline is implemented; remaining work is hardening/ergonomics.

2. FFI section previously implied mostly future work.
- Corrected: MVP checked-boundary and unsafe-isolated real C ABI path are already implemented.

3. CI/unsafe gate posture was partially described as pending.
- Corrected: required CI checks are active on `main`, including phase3d typeclass regression coverage.

4. Tooling section understated current LSP capability surface.
- Corrected: baseline LSP features are implemented; roadmap now focuses on scale/UX hardening.

---

## Rebased Execution Order (From Current State)

## Stage A - Typeclass Hardening and Traceability (Highest Priority)
Goal: improve diagnosability and long-term maintainability of the already-implemented typeclass baseline.

A1. Fallback/ambiguity visibility
- Add structured metadata and user-facing diagnostics for dictionary fallback choices.

A2. Resolution reliability
- Expand regressions for transitive imports, method-as-value, and alias-heavy module graphs.

A3. Documentation sync
- Keep typeclass policy docs and implementation evidence aligned per pass.

## Stage B - Diagnostics and Developer UX
Goal: reduce debugging cost and improve error explainability.

B1. Improve deep import/class-instance diagnostic traces.
B2. Add stable, incremental lints.
B3. Define formatter/linter MVP scope and implementation order.

## Stage C - LLVM/JIT Expansion (Optional, after Stage A stability)
Goal: broaden optional backend only after semantic parity strategy is clear.

C1. Expand lowering coverage beyond current subset.
C2. Add parity checks against interpreter behavior.
C3. Define acceptance gates for LLVM/JIT reliability.

## Stage D - Extended FFI Surface (Optional)
Goal: broaden safe-enough boundary API while keeping unsafe isolated.

D1. Prioritize a minimal additional C ABI set based on user demand.
D2. Keep boundary checks and ownership rules explicit and tested.
D3. Preserve default safe build posture and required CI gates.

---

## Practical Rule for Contributors

Before starting new implementation work:
1. Update `docs/PriorityChecklist.md` first if priority ordering changes.
2. Ensure this plan reflects actual code/CI state.
3. Keep diffs proportional; avoid mixing broad redesign with single failing-test fixes.
