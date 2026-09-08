# Documentation Index

This file classifies `docs/` and root-level status records. It is a routing
index, not a second implementation-status ledger.

Current authority:

- observable behavior: implementation and focused tests;
- capability maturity: `docs/ImplementationPlan.md`;
- current execution order: `docs/PriorityChecklist.md`;
- live work and acceptance criteria: GitHub Issues.

## Classification Policy (Short)

- `Past` (`過去`): Historical snapshots, handoffs, or point-in-time execution summaries.
- `Current` (`現状`): Intended to describe present behavior or active operating policy.
- `Future` (`未来`): Proposals, plans, roadmaps, or open issue-driven work.
- `Needs-Triage` (`要判定`): Mixed documents or docs whose alignment with implementation cannot be confirmed yet.

Rule: when implementation alignment is uncertain, classify as `Needs-Triage` until reconciled with code/tests.

## Classification Table

| path | category | rationale | source-of-truth |
|---|---|---|---|
| `docs/BinaryIRFormat.md` | Future | Proposes a full separate-compilation format; an experimental KIR1/KSIF subset exists. | Proposal scope must be compared with `src/kir1.rs` and `src/ir_pack.rs`. |
| `docs/CACHING_STRATEGY.md` | Future | Optimization strategy and expected gains, not an implementation record. | Proposal baseline; implementation/tests decide final behavior. |
| `docs/CACHING_STRATEGY_ja.md` | Future | Japanese version of strategy document; same planning intent. | Same as `docs/CACHING_STRATEGY.md`. |
| `docs/ARCHIVE_NOTICE.md` | Current | Defines the canonical banner for historical evidence docs. | Documentation governance policy in this repo. |
| `docs/BACKLOG.md` | Past | Archived 2026-03 planning inventory with mixed completed and proposed items. | Historical reference; GitHub Issues are the live tracker. |
| `docs/DOC_INDEX.md` | Current | Canonical classification ledger for `Past/Current/Future/Needs-Triage`. | Documentation governance policy in this repo. |
| `docs/DiagnosticsPlan.md` | Future | Improvement plan for diagnostics UX. | Proposal; final behavior must follow code + tests. |
| `docs/DocComments.md` | Current | Describes active doc comment syntax and behavior. | Lexer/parser/tests are canonical. |
| `docs/FileIO_APIs.md` | Current | Documents currently exposed Prelude file/CLI APIs. | `stdlib/` + runtime/CLI implementation. |
| `docs/HANDOFF_2026-01-27.md` | Past | Date-stamped handoff and local branch status snapshot. | Historical reference only. |
| `docs/HANDOFF_2026-01-30.md` | Past | Date-stamped handoff with branch/WIP state. | Historical reference only. |
| `docs/HANDOFF_2026-02-03.md` | Past | Date-stamped WIP transfer note. | Historical reference only. |
| `docs/HANDOFF_2026-02-05.md` | Past | Date-stamped handoff containing local delta notes. | Historical reference only. |
| `docs/HANDOFF_2026-03-01.md` | Past | Date-stamped handoff summary of executed changes. | Historical reference only. |
| `docs/HANDOFF_2026-03-03.md` | Past | Date-stamped documentation reconciliation handoff. | Historical reference only. |
| `docs/IROptimization.md` | Needs-Triage | States framework principles; implementation depth unclear from doc alone. | Reconcile with optimizer passes and tests. |
| `docs/ImplementationPlan.md` | Current | Evidence-backed capability matrix with explicit maturity vocabulary and known Issue boundaries. | Code/tests/CI plus linked live Issues. |
| `docs/IntermediateRepresentation.md` | Current | Reference-style internal IR description intended as active spec. | `src/ir.rs`, `src/kir1.rs`, runtime/tests. |
| `docs/LLVMIRGeneration.md` | Current | Feature description for existing `kscr_llvm` pipeline. | `crates/kscr_llvm/` + related tests. |
| `docs/LSPDesign.md` | Future | Labeled design/roadmap for expanded LSP/VSIX architecture. | Design intent only. |
| `docs/LSP_Implementation_Summary.md` | Past | "What was implemented" snapshot summary. | Historical implementation report. |
| `docs/LSP_Quick_Start.md` | Current | Operational guide for building/using existing LSP binary. | Build commands + current crate layout. |
| `docs/LSP_Usage.md` | Current | Current editor integration usage instructions. | Actual `kscr-lsp` executable behavior. |
| `docs/LSP_VSIX_NextPlan.md` | Future | "Next Implementation Plan" by name/content. | Planning artifact. |
| `docs/LanguageBNF.md` | Current | Grammar/spec reference intended for active language syntax. | Parser + lexer + syntax tests. |
| `docs/LanguageSemantics.md` | Current | Current language semantics reference with explicit future notes. | Runtime/typechecker behavior + tests. |
| `docs/OPTIMIZATION_SUMMARY.md` | Past | Task completion summary for optimization documentation effort. | Historical summary only. |
| `docs/PriorityChecklist.md` | Current | Active priority ordering for present baseline work. | Current repo priorities and passing tests. |
| `docs/PreludeCodingStyle.md` | Current | Active style guide for keeping `stdlib/Prelude.ks` close to straightforward Haskell notation. | `stdlib/Prelude.ks` + parser/typechecker tests. |
| `docs/REPL_PERFORMANCE.md` | Needs-Triage | Contains dated status plus optimization narrative; freshness uncertain. | Benchmark re-run + current REPL timings required. |
| `docs/ToolchainDesign.md` | Future | High-level binary toolchain design document. | Directional design; not strict implementation record. |
| `docs/TypeClassMethodResolutionPlan.md` | Future | Method-resolution plan document by scope and content. | Plan pending implementation validation. |
| `docs/TypeClassesPlan.md` | Needs-Triage | Includes both "implemented" and planned sections; mixed temporal state. | Reconcile with current typeclass tests/behavior. |
| `docs/TypeSystem.md` | Current | Core type-system reference intended as active documentation. | `src/types.rs` + type tests. |
| `docs/TypeclassDictFallbackPolicy.md` | Current | States active compiler policy constraints. | Typechecker implementation and regressions. |
| `docs/TypeclassSemantics.md` | Current | Explicitly labeled current behavior and migration guidance. | Typeclass runtime/typechecker tests. |
| `docs/VSCodeExtension.md` | Future | Roadmap-oriented VSIX MVP-to-LSP plan. | Planning artifact for extension work. |
| `docs/artifact_formats_v0.3.7.md` | Past | Version-pinned status report for v0.3.7 at a past commit. | Historical versioned snapshot. |
| `docs/ksif-stage3-design.md` | Past | Stage 3 implementation summary (time-boxed). | Historical implementation summary. |
| `docs/plans/IMPLEMENTATION_CHECKLIST.md` | Future | Explicit execution checklist for planned work. | Planning tracker. |
| `docs/plans/PLANNING_INDEX.md` | Future | Index for pending KSIF plan set. | Planning index. |
| `docs/plans/PLAN_SUMMARY.md` | Future | "Ready for implementation" summary for planned work. | Planning summary. |
| `docs/plans/plan.md` | Future | Full implementation plan/specification. | Planning source doc. |
| `docs/traceability/current_vs_impl_matrix.md` | Past | Point-in-time record of the 2026-03 documentation reconciliation. | Historical audit evidence only. |
| `docs/issues/001-qualified-import-name-conflict.md` | Past | Resolved local issue record; its qualified-import regressions pass. | `tests/import_qualified_conflict_issue_001.rs`. |
| `FIX_SUMMARY.md` | Past | Point-in-time fix log with test status context. | Historical execution snapshot. |
| `IMPLEMENTATION_SUMMARY.md` | Past | Point-in-time implementation narrative and solved items. | Historical execution snapshot. |
| `ORCHESTRATION_SUMMARY.md` | Past | Process summary of delegated fix session. | Historical orchestration snapshot. |

## Needs-Triage Items

- `docs/IROptimization.md`
- `docs/REPL_PERFORMANCE.md`
- `docs/TypeClassesPlan.md`

Triage rule for all items above:
- Confirm current behavior via targeted tests/benchmarks first.
- If doc and implementation diverge, update doc to match implementation.

## Next Classification Tasks

1. Verify `docs/IROptimization.md` against current optimizer passes and tests.
2. Refresh `docs/REPL_PERFORMANCE.md` only with a current benchmark run.
3. Split or retire the mixed current/future content in `docs/TypeClassesPlan.md`.

## Count Summary

- Past: 16
- Current: 16
- Future: 13
- Needs-Triage: 3
- Total classified: 48
