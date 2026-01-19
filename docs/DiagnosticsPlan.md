# Diagnostics Improvement Plan

Goal: improve error/diagnostic UX in CLI + LSP.

This plan is user-facing behavior first.
Implementation will avoid stdlib workarounds and keep engine correctness.

## Current baseline
- CLI prints `error: <file:line:col>: <msg> (span a..b)` when `Error::span()` is present.
- LSP publishes diagnostics using only the primary span (`err.span()`).
- Multiple spans are available internally via `Error::spans()`.
- Goto-definition:
  - Unqualified identifiers can jump to toplevel bindings (local or via unqualified imports).
  - `import <Module>` module name now jumps to the module source.

## Non-goals (for now)
- Perfect cross-file related locations for all errors.
- Full multi-error recovery in typechecker.
- Rich pretty-printing of type terms (leave for later).

## Priority order
1) Show multiple relevant locations (failure point vs definition point)
2) Make diagnostics stable to search/filter (error codes)
3) Increase signal in type mismatch messages (expected/actual)
4) Improve module/import failure messages
5) Multi-error collection (design first, then minimal implementation)

## Step-by-step work

### Step 1: CLI: `note:` lines for secondary spans
**User-visible**
- Keep `error:` anchored at the primary span.
- Print additional spans as `note:` lines.

**Implementation**
- File: `src/cli_impl.rs`
- Use `e.spans()`.
- For each secondary span, print `note: <file:line:col>: related location (span a..b)`.
- Deduplicate identical spans.

**Validation**
- Repro file: `/tmp/Diag_B.ks`
- Run:
  - `cargo run --bin kscr -- typecheck /tmp/Diag_B.ks`

### Step 2: LSP: `relatedInformation` for secondary spans
**User-visible**
- VS Code shows extra clickable locations under the same diagnostic.

**Implementation**
- File: `crates/kscr_lsp/src/backend_helpers.rs`
- Extend `create_diagnostic` to fill `related_information` when `err.spans()` has > 1 spans.
- Map each extra span to `Location` in the same document.

**Validation**
- Build LSP: `cargo build --release -p kscr_lsp`
- Trigger diagnostics:
  - `lsp-cli did-open ...`
  - `lsp-cli did-save ...`
  - `lsp-cli events --kind diagnostics`

### Step 3: Error codes / kinds
**User-visible**
- Messages become searchable and stable:
  - Example: `E1001 cannot unify`

**Implementation**
- Introduce a small `enum DiagnosticCode` (or numeric codes) in Rust.
- Thread it through `Error` (either via new variants or structured payload).
- LSP: set `Diagnostic.code`.
- CLI: print `[E1001]` prefix.

**Validation**
- Snapshot-like tests for formatted errors.

### Step 4: Type mismatch delta (expected vs actual)
**User-visible**
- `cannot unify` becomes something like:
  - `cannot unify: expected <T>, got <U>`

**Implementation**
- In `infer_expr_apply` and other unify call sites, format both sides.
- Keep output short; add truncation if needed.

### Step 5: Import resolution failure notes
**User-visible**
- When `import X` fails:
  - show searched paths (local + stdlib)

**Implementation**
- Share the path search logic with the LSP resolver.
- Attach paths as secondary `note:` lines (CLI) and as `relatedInformation` (LSP) or as `message` suffix.

### Step 6: Multi-error collection (design → minimal impl)
**Design**
- Define where recovery is safe.
- Set an upper bound (e.g. 50 diagnostics).
- Fatal errors still abort.

**Implementation (minimal)**
- Start from parser (if feasible), then consider constraint solving.

## Quality gate
- `cargo test`
- `cargo clippy -- -D warnings`
