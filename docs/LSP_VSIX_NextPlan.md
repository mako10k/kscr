# LSP / VSIX Next Implementation Plan

Last updated: 2026-01-15

## Current versions (as of this plan)

- LSP crate: `crates/kscr_lsp/Cargo.toml` → `version = "0.1.0"` (bin: `kscr-lsp`)
- VS Code extension (VSIX): `editors/vscode/package.json` → `version = "0.0.2"`

## Goals

- Improve editor UX quickly with minimal churn.
- Keep behavior aligned with kscr’s module resolution rules (import base is the importing file’s directory).
- Preserve existing tests; add only targeted tests for new behavior.

---

## Phase 1 — Diagnostics: correct ranges (Span/Ranges)

### Problem
- Current diagnostics often report at (0,0) because we do not have structured position info available end-to-end.

### Implementation approach
1. **Introduce structured spans in errors** (minimal, additive)
   - Add a `Span` type (suggest: `{ start: usize, end: usize }` byte offsets + optional file path).
   - Extend the centralized error type (`src/error.rs`) to optionally carry a span.
   - Keep existing `Display` messages stable as much as possible.

2. **Plumb spans from lexer/parser/types**
   - Lexer: produce span on tokenization failures.
   - Parser: attach span on parse failures.
   - Typechecker/import errors: attach span when possible; otherwise keep span `None`.

3. **Convert Span → LSP Range**
   - In `crates/kscr_lsp/src/vfs.rs`, ensure we have a helper to map byte offsets to `Position` (UTF-16).
   - In `crates/kscr_lsp/src/backend.rs` `create_diagnostic(...)`, use the span if present; fall back to (0,0) when absent.

### Acceptance criteria
- A syntax error underlines the correct range (at least start position is correct; ideally start/end).
- Type errors underline a reasonable range when the error originates from a specific identifier/expression.

### Tests
- Unit tests in `kscr`:
  - One lexer error span test
  - One parser error span test
  - One type error span test
- Unit test in `kscr_lsp`:
  - Span→Range mapping test on a string containing multi-byte chars

---

## Phase 2 — Typecheck unsaved documents (VFS → temp file)

### Problem
- `kscr_lsp` currently skips typechecking for unsaved documents (`path.exists() == false`).

### Implementation approach (MVP)
1. In `crates/kscr_lsp/src/backend.rs`, if the URI is a file URI but the path does not exist:
   - Create a temp file *in the same directory as the intended file* (important for relative import resolution).
   - Write current VFS text to that temp file.
   - Call `kscr::types::typecheck_file(&temp_path)`.
   - Delete the temp file after analysis.

2. Reuse the same filename pattern (pid + random suffix) to avoid collisions.

### Acceptance criteria
- While editing an unsaved `.ks` file, type errors appear and update on change.
- Import resolution remains consistent with the file’s directory.

### Tests
- `crates/kscr_lsp/tests/` integration test that:
  - creates a temp workspace dir
  - writes dependent modules to disk
  - runs analysis on an unsaved main buffer via temp-file path

---

## Phase 3 — Hover + Go-to-definition (MVP: same-file only)

### Hover (MVP)
- For identifier under cursor:
  - If it is a top-level binding/type/class in the same file, show inferred type or declared signature.
  - If not found, return `None`.

### Go-to-definition (MVP)
- For identifier under cursor:
  - Jump to same-file definition location.
  - Cross-file resolution is deferred.

### Acceptance criteria
- Hover shows a type for simple local symbols.
- Go-to-definition works within the same file.

### Tests
- Unit tests for:
  - extracting identifier under cursor
  - mapping identifier → definition location

---

# Version bump plan

## Principles

- **LSP (Rust crate)**: SemVer.
  - Patch: internal changes / bug fixes only.
  - Minor: new LSP capabilities (hover/definition) or behavior changes users notice.
- **VSIX (extension)**: keep `0.y.z` while rapidly iterating.
  - Patch: fixes, packaging, small behavior changes.
  - Minor: new user-visible feature or setting.

## Proposed bumps per phase

### After Phase 1 (diagnostic ranges)
- `kscr_lsp`: `0.1.0` → `0.1.1`
- VSIX: `0.0.2` → `0.0.3`

### After Phase 2 (unsaved typecheck)
- `kscr_lsp`: `0.1.1` → `0.1.2`
- VSIX: `0.0.3` → `0.0.4`

### After Phase 3 (hover + definition)
- `kscr_lsp`: `0.1.2` → `0.2.0` (new user-visible LSP capability set)
- VSIX: `0.0.4` → `0.1.0` (LSP client becomes meaningfully IDE-like)

## Release checklist (each bump)

1. `cargo test`
2. `cargo clippy -- -D warnings`
3. `cargo geiger`
4. LSP manual smoke:
   - `cd crates/kscr_lsp && cargo test && bash test_lsp.sh`
5. VSIX packaging smoke:
   - `cd editors/vscode && npm ci && npx --yes @vscode/vsce package --dependencies`

## Notes

- VSIX packaging currently needs `--dependencies` to include `vscode-languageclient` in the VSIX.
- Consider bundling later to reduce file count; not required for correctness.
