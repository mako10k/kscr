# Backlog (Draft v1)

Source: initial extraction from `Future` category items in `docs/DOC_INDEX.md`.

## Entry Template

- ID: `BG-XXX`
- Priority: `P0 | P1 | P2`
- Title: short actionable title
- Background: why this is needed now
- DoD: objective completion criteria

## Initial Items

### BG-001
- Priority: P1
- Title: Validate and stage Binary IR format proposal
- Background: `docs/BinaryIRFormat.md` is currently proposal-only with explicit TODO markers.
- DoD: Implementation decision record added; either (a) minimal supported subset merged with tests, or (b) proposal narrowed with explicit out-of-scope and timeline.

### BG-002
- Priority: P1
- Title: Implement diagnostics plan minimum slice (multi-span + clearer messages)
- Background: `docs/DiagnosticsPlan.md` defines user-visible improvements not fully represented as verified tasks.
- DoD: At least one CLI diagnostic and one LSP diagnostic flow improved and covered by regression tests.

### BG-003
- Priority: P2
- Title: Baseline caching strategy with benchmark harness
- Background: `docs/CACHING_STRATEGY.md` and `docs/CACHING_STRATEGY_ja.md` propose substantial speedups without a tracked implementation baseline.
- DoD: Benchmark script and baseline numbers committed; one safe cache layer (AST or type result) implemented behind clear invalidation rules.

### BG-004
- Priority: P0
- Title: Resolve qualified import false conflict issue
- Background: `docs/issues/001-qualified-import-name-conflict.md` includes a minimal reproduction of user-facing failure.
- DoD: Repro test passes, conflict behavior is corrected for qualified names, and no regression appears in import-related test suites.

### BG-005
- Priority: P2
- Title: Refresh VSIX/LSP roadmap into executable milestones
- Background: `docs/LSPDesign.md`, `docs/LSP_VSIX_NextPlan.md`, and `docs/VSCodeExtension.md` contain overlapping future directions.
- DoD: Consolidated milestone document with owners, scope boundaries, and release order (M1/M2/M3).

### BG-006
- Priority: P1
- Title: Typeclass method resolution modernization
- Background: `docs/TypeClassMethodResolutionPlan.md` identifies non-ordinary method handling that should be improved.
- DoD: A scoped implementation plan is accepted; first incremental change merged with targeted type inference regression tests.

### BG-007
- Priority: P1
- Title: KSIF dependency hashing and SCC solver rollout
- Background: `docs/plans/plan.md` and related planning set are marked ready but not tracked in a concise execution backlog.
- DoD: Phase 1 and Phase 2 checklist items are completed with passing tests and documented deltas.

### BG-008
- Priority: P2
- Title: Toolchain design to implementation traceability
- Background: `docs/ToolchainDesign.md` is conceptual and needs explicit mapping to actual crates and commands.
- DoD: Cross-reference table added (design component -> crate/file/command/test), with gaps captured as backlog tasks.

### BG-009
- Priority: P2
- Title: Design and implement a dedicated IR bytecode VM target
- Background: Current IR execution is expression-tree evaluation in `src/ir.rs`; `docs/IntermediateRepresentation.md` now treats explicit VM bytecode as future work.
- DoD: Define bytecode instruction set and execution model, implement encoder/decoder + runtime executor, and add roundtrip/execution regression tests.

### BG-010
- Priority: P1
- Title: Add LLVM CLI integration regression coverage
- Background: LLVM backend behavior is currently validated mainly by unit tests in `crates/kscr_llvm/src/lib.rs`; there is no repository-level CLI regression test that exercises `llvm-ir` output shape and `compile --llvm` invocation path with feature gating/toolchain assumptions.
- DoD: Add gated integration tests that verify `llvm-ir` success/error paths and `compile --llvm` contract (feature-gate diagnostics and stable invocation behavior), including clear skip behavior when `clang` is unavailable.

### BG-011
- Priority: P1
- Title: Add typeclass dictionary fallback traceability and regression coverage
- Background: `src/types.rs` currently has evidence-based early dictionary selection paths (inferred application type and enclosing binding return type), but `docs/TypeclassDictFallbackPolicy.md` still tracks missing explicit trace metadata and automated regressions.
- DoD: Add structured ambiguity metadata on deferred dictionary sites, add automated regression for `tests/repro_return_in_letrec_fail.ks`, and add an LSP completion test for incomplete-code default-mode behavior.

### BG-012
- Priority: P1
- Title: Add workspace-wide symbol index for LSP navigation and rename
- Background: Current `references`/`rename` in `crates/kscr_lsp/src/backend_references_rename.rs` are VFS-scoped and only cover open documents; definition currently resolves through immediate module reads, not a full project index.
- DoD: Add a project symbol index that includes closed files, make `references`/`rename` use indexed symbol resolution, and add regression tests covering cross-file edits when only one file is open in the editor.

### BG-013
- Priority: P1
- Title: Improve VSIX server provisioning and startup preflight diagnostics
- Background: `editors/vscode/extension.js` currently launches `kscr-lsp` from `kscr.lsp.serverPath` or `PATH` only; users must install/manage binaries manually and failure guidance is minimal.
- DoD: Add explicit preflight checks (existence/executable validation + actionable error details), document supported provisioning flows, and add smoke coverage for failure and success startup paths.

### BG-014
- Priority: P2
- Title: Add VS Code snippets and formatter integration contract
- Background: The extension currently contributes language grammar/config/LSP wiring but no snippets or formatter command integration contract.
- DoD: Contribute initial snippet set and define formatter command/settings contract (disabled by default if formatter is absent), with user-facing docs and smoke checks.
