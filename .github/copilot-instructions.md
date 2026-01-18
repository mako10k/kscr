# Copilot Instructions (kscr)

Goal: improve and expand `stdlib/` while keeping the Rust execution engine (lexer/parser/types/IR/runtime) correct.

## Language / Writing (MANDATORY)

- The codebase is English-first: write code comments, docs, commit messages, identifiers, and this instruction file in English.
- Keep wording short and concrete; prefer examples over prose.

## Git Safety (MANDATORY)

- Do not run destructive or state-changing git commands without explicit user permission.
  - Examples: `git checkout`/`switch`, `reset --hard`, `clean -fd`, `merge`, `rebase`, `cherry-pick`, `revert`
- Prefer read-only commands for investigation: `git status`, `git diff`, `git log`, `git show`.

## Stdlib Policy (IMPORTANT)

- Do not “fix” engine bugs via ad-hoc stdlib workarounds.
- If changing stdlib appears to make a failing program/test pass, first assume a Rust-side bug.
  - Create a minimal `.ks` reproduction.
  - Fix the Rust subsystem (Lexer/Parser/Typechecker/IR/Runtime).
  - Add regression coverage.
- If behavior/spec is unclear or docs disagree with reality, open a GitHub Issue first.
  - Include: minimal repro, expected behavior, actual behavior, suspected subsystem.

## No Test-Only Special-Casing (MANDATORY)

- Do not add conditional branches or special cases whose only purpose is to make tests pass.
- If a change "makes tests green" for unclear reasons, assume an engine bug first.
  - Add a minimal `.ks` reproduction.
  - Fix the Rust subsystem (Lexer/Parser/Typechecker/IR/Runtime).
  - Add regression coverage.

## `lsp-cli` Usage (REQUIRED)

Use `lsp-cli` for reproducible diagnostics and reference searches.

### Rust (rust-analyzer)

```bash
npx -y @mako10k/lsp-cli --root . --format pretty ws-symbols "typecheck"
npx -y @mako10k/lsp-cli --root . --format pretty references src/types.rs 0 0
```

### kscr language server (`kscr-lsp`)

```bash
cd crates/kscr_lsp
cargo build --release

npx -y @mako10k/lsp-cli \
  --root . \
  --server-cmd "$PWD/target/release/kscr-lsp" \
  --format json batch <<'JSONL'
{"id":1,"cmd":"diagnostics","file":"./tests/example_hello.ks"}
JSONL

# Optional: pretty-print daemon events afterwards
# npx -y @mako10k/lsp-cli --root . --format pretty events --kind diagnostics --since 0 --limit 200
```

## Version bump policy

Keep versions aligned across Rust crates (and the npm package if used).

Bump **PATCH** when:
- Bugfixes that don’t change public surface behavior (incl. stdlib fixes).
- Internal refactors, docs, CI/editor config.

Bump **MINOR** when:
- Additions that are backwards compatible: new stdlib modules/functions/types, new CLI flags, new language features that don’t break existing code.
- New optional Cargo features / new builtins behind feature flags.

Bump **MAJOR** when:
- Breaking changes: syntax changes, type system changes affecting inference, stdlib API removals/renames, CLI breaking flags, changes to default semantics.

If a change affects both the Rust engine and stdlib semantics, treat it as at least **MINOR**.

## Quality Gates (before commit)

```bash
cargo test
cargo clippy -- -D warnings
```