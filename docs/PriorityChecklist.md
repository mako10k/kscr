# Priority Checklist (Agent Memory)

This file defines active priority IDs and the execution order from the **current implementation baseline**.

Rules:
- Do not use P numbers not listed in this file.
- If implementation and docs differ, treat implementation/tests/CI as source-of-truth and update docs.
- Potentially non-Haskell-compatible ideas are tracked as P3 and require explicit user instruction.

Last updated: 2026-02-27

---

## Current Snapshot (2026-02-27)

### Implemented / Operational
- P0: Import traversal E2E smoke coverage (multi-file, transitive imports, qualifiers, cycle diagnostics).
- P1: IO exceptions (`throw` / `catch` / `try`).
- P2: Braces/semicolons surface syntax (`do`/`let`/`where` forms).
- P4: Numeric/doc consistency for checked behavior.
- P5: Backend numeric boundary checks (MVP checked casts).
- P6: Minimal FFI boundary behavior (`ffiAddI32` / `ffiAddF32`).
- P7: Unsafe boundary isolation and runtime tracing switches.
- P8: Optional BigInt backend wiring.
- P9: Real C ABI MVP under feature flag (`ffiPuts`) with unsafe isolation.
- P10: Unsafe feature gate policy + CI enforcement (required jobs in workflow).
- P11: `unsafe_bigint` isolation into subcrate.
- P12: Function clauses/guards parser desugar.
- P13A/B/C/D: Import/export behavior and diagnostics alignment.
- P14/P15: REPL MVP + optional readline + `:load`/`:modules`.
- P16 Phase 1/2: `deriving Show` / `deriving Eq` + dictionary passing.
- P17: KSIF test determinism hardening for clean checkouts (stdlib `.ksif` assumption removed from flaky path).

### In Progress / Not Yet Implemented
- P16 Phase 3: User-defined `class` / `instance` (non-ground instance support, coherence, ambiguity handling).

### Corrected from older docs
- CI integration for unsafe feature policy is already active and green on `main`; treat P10 CI integration as done.
- Recent `ksif_hash_rebuild` CI flake was fixed by deterministic test setup; no longer depends on local untracked stdlib `.ksif` artifacts.

---

## Active Priorities (Execution Order)

## P16 — Typeclasses Phase 3 (Primary)
Purpose: Complete user-defined typeclasses/instances with coherent dictionary passing.

### Phase 3A — Surface and environment foundations
- [ ] Parse and validate user `class` / `instance` declarations in all supported module/import paths.
- [ ] Keep MVP restriction: reject user `class Show` / `class Eq` redefinition.
- [ ] Build explicit class/instance environment snapshots per module for deterministic import merge.

### Phase 3B — Resolution and coherence
- [ ] Support constrained non-ground instances (`instance (C a) => C (Maybe a)`).
- [ ] Enforce coherence: no overlap, no duplicates, deterministic tie-breaking diagnostics.
- [ ] Improve ambiguity diagnostics with concrete candidate traces and import origin notes.

### Phase 3C — IR/runtime completion
- [ ] Complete dictionary construction/passing for user class methods as values across module boundaries.
- [ ] Add regression coverage for method forwarding, qualified/unqualified imports, and transitive instance visibility.
- [ ] Validate against current no-flattening architecture and keep changes minimal.

### Phase 3D — Hardening and gate quality
- [ ] Add focused CI tests for class/instance regressions and known edge cases.
- [ ] Add docs for class/instance semantics and migration from deriving-only assumptions.

Status:
- [ ] Not started as a complete milestone.
- [ ] Partial groundwork exists from deriving and built-in dictionary passing.

## P18 — Diagnostics and Tooling Usability (Secondary)
Purpose: Improve developer UX after P16 Phase 3 is stable.
- [ ] Strengthen span/source mapping consistency in complex imported errors.
- [ ] Add/update lint/formatter scope (stable subset first).
- [ ] Add targeted CLI debug affordances for import/class-instance resolution tracing.

## P19 — LLVM/JIT Expansion (Optional, after semantic stability)
Purpose: Expand optional backend only after interpreter semantics for class/instance are stable.
- [ ] Increase lowering coverage beyond current optional LLVM text generation path.
- [ ] Add semantics parity checks interpreter vs LLVM path.

## P3 — Spec-Divergent Ideas (Skippable)
Purpose: Track ideas that may deviate from Haskell or repo goals.
- [ ] Skip by default unless explicitly requested.

---

## Historical Priority IDs (Completed)
Use these IDs for reference only; avoid reopening without explicit request.
- P0, P1, P2, P4, P5, P6, P7, P8, P9, P10, P11, P12, P13, P14, P15, P17.

---

## Notes for Future Updates
- Keep this file concise and execution-oriented.
- When a milestone status changes, update this file first before implementation starts.
- Keep commit references in PR/issue history; this file tracks state and order, not exhaustive changelog.
