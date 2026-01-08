# Copilot Instructions for kscr

## Overview
- This is a Rust-based toolchain for a lazy functional scripting language.
- Language specification and design philosophy are documented in the `docs/` directory.

## Key Directories & Files
- `src/`: Main Rust source code. Each file has a distinct role (e.g., `lexer.rs`, `parser.rs`, `ir.rs`).
- `docs/`: Markdown documents for language specification, design, type system, etc.
- `README.md`: Setup, build, test, and CLI command examples.

## Build, Test, Run
- Use standard Rust `cargo` commands:
  - Test: `cargo test`
  - Run: `cargo run -- help`
  - Parse: `cargo run -- parse path/to/file.ks`

## Commit Gate: Quality Checks Required
Before every commit, the following quality checks MUST pass:

- **Lint**: Run `cargo clippy -- -D warnings` and ensure no warnings.
- **Unsafe code**: Run `cargo geiger` and confirm no unsafe code is present.
- **Unused/duplicate dependencies**: Run `cargo udeps` (requires nightly) and confirm no unused dependencies.

If any check fails, DO NOT commit. Fix issues first.

Example workflow:
1. Run all tests: `cargo test`
2. Run lint: `cargo clippy -- -D warnings`
3. Run unsafe check: `cargo geiger`
4. (Optional, nightly only) Run dependency check: `cargo +nightly udeps`
5. If all pass, commit: `git commit -a -m 'your message'`

These checks are mandatory for all AI agent-driven changes.

## Coding & Design Patterns
- Components (AST, Lexer, Parser, IR, Type System) are separated by file.
- Error handling is centralized in `src/error.rs`.
- CLI interface logic is in `src/cli.rs`; entry point is `src/main.rs`.
- Type system and intermediate representation are managed in `src/types.rs` and `src/ir.rs`.

## Essential Knowledge & Workflow
- Refer to `docs/` for language specification and design intent (e.g., `LanguageBNF.md`, `TypeSystem.md`).

### Current project policy (execution-first)
- Prefer making the system runnable end-to-end over adding more surface syntax.
- Avoid further Source⇒AST churn unless a failing end-to-end test proves it is required.
- First add/expand **run smoke tests** that exercise:
  - `kscr run` over multiple files (`import` traversal)
  - qualified refs (`OM.x`), export/import boundaries
  - `data` + constructors, `case`, `do`
- Only after smoke tests are in place, incrementally fill gaps in typecheck/IR/runtime that those tests expose.
- If design choices are ambiguous, lean toward **Haskell-like semantics** (especially for import/qualified name resolution).

- To add new language features or syntax (only when required by a failing test):
  1. Extend `lexer.rs`
  2. Update `parser.rs`
  3. Modify `ast.rs`
  4. Adjust `ir.rs`
- For new CLI commands:
  - Add functions to `cli.rs` and update the entry logic in `main.rs`.
- Tests are typically managed in `src/lib_test.rs`.

## External Dependencies & Integration
- Relies mainly on Rust standard libraries; dependencies managed via Cargo.toml.
- No current integration with external services or tools.

## Examples
- Adding a new syntax feature: Implement across `lexer.rs`, `parser.rs`, `ast.rs`, and `ir.rs`.
- Adding a CLI command: Update `cli.rs` and `main.rs`.

---

For further details, consult the relevant files in `docs/` and `src/`. If conventions or workflows are unclear, ask for clarification or examples from maintainers.