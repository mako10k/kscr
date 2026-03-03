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
| `docs/LanguageSemantics.md` | Needs-Update | `src/ir.rs` thunk/IO execution + checked cast paths; `src/types.rs` main type gate; `src/lib_test.rs` list comprehension/laziness/cast/main tests | Updated numeric and entrypoint claims to match implementation; no new impl-bug suspicion in this pass. |
| `docs/TypeSystem.md` | Needs-Update | `src/types.rs` policy comments + `src/safe_bigint.rs` Integer implementation + `src/types.rs` string literal typing/open-row constraints; typeclass regressions in `src/types.rs` and `tests/*.rs` | Updated stale primitive/string/overloading wording to match implementation. |
| `docs/TypeclassSemantics.md` | Aligned | `src/types.rs` dictionary rewrite/failfast/overlap checks + transitive method-value typecheck tests; runtime CLI regression in `tests/cli_run_typeclass_transitive_reexport.rs` | Claims match current user-class/instance and transitive forwarding behavior. |
| `docs/IntermediateRepresentation.md` | Needs-Update | `crates/kscr_ir/src/ir.rs` IR node set; `src/ir.rs` lowering+runtime+IO exceptions; `src/kir1.rs` KIR1/KSIF sections+roundtrip tests; `src/ir_pack.rs` packed IR roundtrip test | Rewrote doc to implementation-first description; moved VM-bytecode-only claims to backlog (`BG-009`). |
| `docs/DocComments.md` | Needs-Update | `src/lexer.rs` doc tokens + `src/parser_impl.rs` attachment flow + `tests/doc_comments_lex_smoke.rs`/`tests/doc_comments_attach_smoke.rs` + `crates/kscr_lsp/src/backend_diagnostics_hover.rs`/`crates/kscr_lsp/tests/completion_docs_smoke.rs` | Updated attachment wording (blank-line behavior) and added constructor-attachment coverage. |
| `docs/FileIO_APIs.md` | Aligned | `stdlib/Prelude.ks` exports + `src/types.rs` builtin typing + `src/ir.rs` IoAction/runtime + `tests/io_apis.rs`/`tests/test_*` fixtures | Updated getArgs example to executable-path-inclusive output and verified file IO/exit semantics. |
| `docs/LLVMIRGeneration.md` | Needs-Update | `crates/kscr_llvm/src/lib.rs` MVP subset + `src/cli/cli_compile.rs` `compile --llvm` path + `src/cli/cli_llvm_ir.rs` command wiring | Updated over-claims (JIT/optimization/full lowering) to MVP-scoped behavior. |
| `docs/LSP_Quick_Start.md` | Needs-Update | `crates/kscr_lsp/test_lsp.sh` build+startup smoke path; `crates/kscr_lsp/src/backend.rs` advertised capabilities and change/save diagnostics; `crates/kscr_lsp/src/backend_diagnostics_hover.rs` file-backed typecheck behavior | Updated feature status and unsaved/typecheck wording to match implementation details. |
| `docs/LSP_Usage.md` | Needs-Update | `crates/kscr_lsp/src/backend.rs` capability set; `editors/vscode/package.json` language id/settings keys; `crates/kscr_lsp/src/backend_references_rename.rs` VFS-scoped references/rename | Corrected binary path wording and moved implemented navigation features out of "Coming Soon". |
| `docs/LanguageBNF.md` | Needs-Update | `src/lexer.rs` shebang/comments/layout + `src/parser_impl.rs` module/import/export/fixity/class/instance/expression forms + `src/parser_impl/pattern.rs` view/record-loose patterns + `src/lib_test.rs` list-range/view/record-loose tests | Updated grammar claims to current parser behavior (including sections, list ranges, open records, class/instance, and import specs). |
| `docs/TypeclassDictFallbackPolicy.md` | Needs-Update | `src/types.rs` resolve path (`resolve_method_dict_expr`) + failfast helpers/tests + CLI check for `tests/repro_return_in_letrec_fail.ks`; remaining gaps tracked in `docs/BACKLOG.md` (`BG-011`) | Updated policy wording to match evidence-based fallback currently implemented. |
| `docs/DOC_INDEX.md` | Aligned | Generated from current classification work | This file is the classification source doc itself. |
| `docs/ARCHIVE_NOTICE.md` | Aligned | Applied to all current `Past` docs | Template and applied banner are in sync. |

## Completed In This Pass

1. Added archive banners to all docs currently classified as `Past`.
2. Created classification ledger and initial consolidated backlog.
3. Reconciled one confirmed mismatch in `docs/PriorityChecklist.md`.
4. Reconciled `docs/TypeSystem.md` and audited `docs/TypeclassSemantics.md` against implementation evidence.
5. Reconciled `docs/LanguageBNF.md` against parser/lexer behavior and updated grammar claims to implementation-first wording.

## Next 3 Reconciliation Tasks

1. `docs/LSPDesign.md`: reconcile roadmap statements with currently advertised `kscr-lsp` capabilities and limits.
2. `docs/ImplementationPlan.md`: reconcile plan status against currently shipped implementation milestones.
3. `docs/TypeclassDictFallbackPolicy.md`: implement `BG-011` (fallback ambiguity metadata + regression coverage).
