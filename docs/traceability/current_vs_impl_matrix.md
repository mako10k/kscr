# Current Docs vs Implementation Matrix (Draft v1)

This matrix tracks reconciliation status for documents classified as `Current` in `docs/DOC_INDEX.md`.

Legend:
- `Aligned`: documentation matches implementation evidence.
- `Needs-Update`: documentation changed to follow implementation.
- `Needs-Evidence`: not yet verified against code/tests.
- `Impl-Bug-Suspected`: possible implementation bug; do not force docs to match before confirmation.

## Audit Table

| Document | Status | Evidence | Notes |
|---|---|---|---|
| `docs/PriorityChecklist.md` | Needs-Update | `.github/workflows/ci.yml` contains `phase3d_typeclass_regressions` job | Updated stale statement that Phase 3D CI job was pending. |
| `docs/LanguageSemantics.md` | Needs-Evidence | Pending targeted runtime/typechecker checks | Audit queued as top priority. |
| `docs/TypeSystem.md` | Needs-Evidence | Pending `src/types.rs` + type tests review | Audit queued as top priority. |
| `docs/TypeclassSemantics.md` | Needs-Evidence | Pending typeclass integration tests review | Audit queued as top priority. |
| `docs/IntermediateRepresentation.md` | Needs-Evidence | Pending `src/ir.rs` / `src/kir1.rs` review | Audit queued after type semantics docs. |
| `docs/DocComments.md` | Needs-Evidence | Pending lexer/parser + doc comment tests review | Scheduled. |
| `docs/FileIO_APIs.md` | Needs-Evidence | Pending stdlib/runtime/CLI surface review | Scheduled. |
| `docs/LLVMIRGeneration.md` | Needs-Evidence | Pending `crates/kscr_llvm` capability review | Scheduled. |
| `docs/LSP_Quick_Start.md` | Needs-Evidence | Pending build/run command verification | Scheduled. |
| `docs/LSP_Usage.md` | Needs-Evidence | Pending usage verification with current `kscr-lsp` | Scheduled. |
| `docs/LanguageBNF.md` | Needs-Evidence | Pending parser grammar coverage review | Scheduled. |
| `docs/TypeclassDictFallbackPolicy.md` | Needs-Evidence | Pending fallback path code + regressions review | Scheduled. |
| `docs/DOC_INDEX.md` | Aligned | Generated from current classification work | This file is the classification source doc itself. |
| `docs/ARCHIVE_NOTICE.md` | Aligned | Applied to all current `Past` docs | Template and applied banner are in sync. |

## Completed In This Pass

1. Added archive banners to all docs currently classified as `Past`.
2. Created classification ledger and initial consolidated backlog.
3. Reconciled one confirmed mismatch in `docs/PriorityChecklist.md`.

## Next 3 Reconciliation Tasks

1. `docs/LanguageSemantics.md`: verify major semantics claims with existing runtime/typechecker regressions.
2. `docs/TypeSystem.md` and `docs/TypeclassSemantics.md`: verify typeclass/coherence/resolution claims against `src/types.rs` and integration tests.
3. `docs/IntermediateRepresentation.md`: verify IR claims against `src/ir.rs`, `src/kir1.rs`, and current execution path.
