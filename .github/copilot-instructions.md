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

## Quality Gates (before commit)

```bash
cargo test
cargo clippy -- -D warnings
```