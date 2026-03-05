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
| `docs/LSPDesign.md` | Needs-Update | `crates/kscr_lsp/src/backend.rs` capability set (completion/references/rename/semantic tokens); `editors/vscode/extension.js` serverPath-or-PATH launch path; `editors/vscode/package.json` current settings keys | Rebased roadmap sections to current implementation; moved auto-download and workspace-wide indexing to future scope. |
| `docs/LSP_VSIX_NextPlan.md` | Needs-Update | `crates/kscr_lsp/src/backend.rs` advertised capabilities; `crates/kscr_lsp/src/backend_diagnostics_hover.rs` unsaved-file temp path typecheck; `editors/vscode/extension.js` server path resolution and restart command | Removed already-implemented phases and compressed roadmap to post-baseline tasks only; linked remaining scope to `BG-012`/`BG-013`/`BG-014`. |
| `docs/VSCodeExtension.md` | Needs-Update | `editors/vscode/package.json` language + settings + command contribution; `editors/vscode/extension.js` LanguageClient startup and path fallback; `tests/lsp_completion_docs_smoke.rs` and `tests/lsp_semantic_tokens_smoke.rs` baseline coverage | Rewrote roadmap to implementation-first baseline and moved only unimplemented items to backlog-linked post-baseline section. |
| `docs/ImplementationPlan.md` | Needs-Update | `src/parser_impl.rs` class/instance parse paths; `src/lib_test/typeclass_phase3.rs` user class/instance regressions; `.github/workflows/ci.yml` phase3d job | Updated M3 status to implemented baseline and shifted execution order from implementation gap to hardening gap. |
| `docs/LanguageBNF.md` | Needs-Update | `src/lexer.rs` shebang/comments/layout + `src/parser_impl.rs` module/import/export/fixity/class/instance/expression forms + `src/parser_impl/pattern.rs` view/record-loose patterns + `src/lib_test.rs` list-range/view/record-loose tests | Updated grammar claims to current parser behavior (including sections, list ranges, open records, class/instance, and import specs). |
| `docs/TypeclassDictFallbackPolicy.md` | Aligned | `src/types.rs` structured fallback trace (`DictFallbackTraceEvent` / `DictFallbackDecision`) + failfast helpers/tests + regressions (`tests/typeclass_dict_fallback_regression.rs`, `tests/lsp_completion_docs_smoke.rs`, `tests/repro_return_in_letrec_fail.ks`) | BG-011 implemented; policy and implementation now aligned. |
| `docs/DOC_INDEX.md` | Aligned | Generated from current classification work | This file is the classification source doc itself. |
| `docs/ARCHIVE_NOTICE.md` | Aligned | Applied to all current `Past` docs | Template and applied banner are in sync. |

## Completed In This Pass

1. Added archive banners to all docs currently classified as `Past`.
2. Created classification ledger and initial consolidated backlog.
3. Reconciled one confirmed mismatch in `docs/PriorityChecklist.md`.
4. Reconciled `docs/TypeSystem.md` and audited `docs/TypeclassSemantics.md` against implementation evidence.
5. Reconciled `docs/LanguageBNF.md` against parser/lexer behavior and updated grammar claims to implementation-first wording.
6. Reconciled `docs/LSPDesign.md` against `kscr-lsp` capabilities and current VS Code extension launch/settings behavior.
7. Reconciled `docs/ImplementationPlan.md` milestone status against current parser/typeclass tests/CI.
8. Reconciled `docs/LSP_VSIX_NextPlan.md` to post-baseline scope and linked remaining work to backlog IDs.
9. Reconciled `docs/VSCodeExtension.md` to baseline-vs-future split and removed already-implemented LSP items from future roadmap.
10. Reconciled `docs/LSP_Quick_Start.md` startup/provisioning notes with current extension behavior (`serverPath`/`PATH` + restart flow).
11. Reconciled `docs/LSP_Usage.md` with current VFS-scoped references/rename limitation and linked workspace-index future direction.

## Next 3 Reconciliation Tasks

1. `docs/PriorityChecklist.md`: consider status normalization (`Needs-Update` -> `Aligned`) after one focused verification pass.
2. `docs/LanguageSemantics.md`: consider status normalization (`Needs-Update` -> `Aligned`) after one focused verification pass.
3. `docs/LSPDesign.md`: verify roadmap status after BG-012/BG-013 scoping updates.
