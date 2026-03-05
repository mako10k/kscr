# LSP / VSIX Next Implementation Plan

Last updated: 2026-03-05

## Baseline implemented (moved out of future plan)

The following items were previously listed as future phases and are now baseline.

- Diagnostics with source ranges are active in the LSP diagnostic pipeline.
- Unsaved-file typecheck is active via temp-file fallback in the target directory.
- Hover and go-to-definition are available.
- Completion, references, rename, and semantic tokens are available.
- VS Code extension can launch `kscr-lsp` via `kscr.lsp.serverPath` or `PATH`.

## Post-baseline scope (next)

Only items not implemented are kept here.

1. Workspace-wide symbol index for closed-file navigation and rename
- Current limitation: references/rename are VFS-scoped (open documents only), and definition relies on direct import/file reads.
- Track in backlog: `BG-012`.

2. VSIX server provisioning and preflight diagnostics
- Current limitation: users must manually provide `kscr-lsp` via PATH or `kscr.lsp.serverPath`.
- Track in backlog: `BG-013`.

3. Editor UX additions (snippets + formatter command wiring)
- Current limitation: no snippets contribution and no formatter command integration contract.
- Track in backlog: `BG-014`.

## Validation gate for post-baseline tasks

- Keep module resolution semantics unchanged (import base = importing file directory).
- Add focused tests in `crates/kscr_lsp/tests/` and `tests/lsp_*`.
- Keep release payload stable unless explicitly approved.

## Versioning note

- Baseline capability set already exceeds the old phase assumptions.
- Next version bump should be tied to delivery of `BG-012`/`BG-013`/`BG-014` slices, not to already-implemented baseline items.
