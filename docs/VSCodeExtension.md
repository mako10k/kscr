# VS Code Extension (.vsix) Notes / Roadmap

Last updated: 2026-03-05

This document is implementation-first. Baseline features already shipped are listed separately from post-baseline work.

## Baseline implemented

### Language support

- Language id: `kscr`
- File extension association: `.ks`
- TextMate grammar and language configuration are packaged in `editors/vscode/`.

### LSP client wiring

- Extension entrypoint (`editors/vscode/extension.js`) starts a `LanguageClient` for `kscr-lsp`.
- Startup path resolution supports:
  - `kscr.lsp.serverPath` (explicit path)
  - fallback to `kscr-lsp` from `PATH`
- Command `kscr.lsp.restart` is contributed.

### LSP capability baseline (server side)

Current `kscr-lsp` advertises and serves:

- Diagnostics
- Hover
- Definition
- Document symbols
- Completion
- References
- Rename
- Semantic tokens (full/range/delta)

## Post-baseline roadmap (backlog-linked)

1. Workspace-wide index for closed-file navigation/rename
- Reason: current references/rename logic is VFS-scoped.
- Backlog: `BG-012`.

2. Server provisioning and startup preflight UX in extension
- Reason: current UX still assumes manual binary setup.
- Backlog: `BG-013`.

3. Snippets and formatter command integration contract
- Reason: no snippets contribution and no formatter pipeline contract yet.
- Backlog: `BG-014`.

## Packaging notes

- VSIX packaging remains Node-based with `@vscode/vsce`.
- Use `npx --yes @vscode/vsce package --dependencies` to include runtime dependencies.

## References

- `editors/vscode/package.json`
- `editors/vscode/extension.js`
- `crates/kscr_lsp/src/backend.rs`
- `crates/kscr_lsp/src/backend_diagnostics_hover.rs`
- `crates/kscr_lsp/src/backend_goto_completion.rs`
- `crates/kscr_lsp/src/backend_references_rename.rs`
- `crates/kscr_lsp/src/backend_semantic_tokens.rs`
- `tests/lsp_completion_docs_smoke.rs`
- `tests/lsp_semantic_tokens_smoke.rs`
