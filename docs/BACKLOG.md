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
