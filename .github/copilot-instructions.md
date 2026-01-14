# Copilot Instructions for kscr

kscr is a Rust-based toolchain for a lazy functional scripting language with Haskell-like semantics.

## Safety / Git operations (MANDATORY)

- NEVER run destructive or state-changing git commands without explicit user permission.
   - Includes (non-exhaustive): `git checkout`, `git switch`, `git reset --hard`, `git clean -fd`, `git merge`, `git rebase`, `git cherry-pick`, `git revert`.
   - If the user has not clearly approved it in the current conversation, ask first.
- Prefer read-only commands when gathering context (e.g. `git status`, `git diff`, `git log`, `git show`).
- If the user asks to merge, do not merge until CI status is confirmed and the user explicitly approves the merge action.

## What to optimize for

- End-to-end runnability first (lexer → parser → typechecker → IR → runtime).
- Readability and debuggability over micro-optimizations.
- Haskell-like semantics when behavior is ambiguous (especially imports/qualification).

## Language features (high level)

- Lazy evaluation (call-by-need with memoization)
- Hindley–Milner type inference with type classes (`Show`, `Eq`)
- Module system (qualified imports, export control)
- Pattern matching (record patterns, guards, or-patterns)
- Algebraic data types (with deriving)
- Do-notation for monadic IO
- Interactive REPL (optional readline via `--features readline`)

## Architecture (pipeline)

1. Lexer: `src/lexer.rs` → tokens
2. Parser: `src/parser.rs` → AST in `src/ast.rs`
    - Collects fixity declarations; parses with precedence/associativity
    - Desugars Haskell-style function clauses into lambda+case bindings
3. Typechecker: `src/types.rs` → typed module
    - HM inference + constraints for type classes
    - Multi-file module resolution; import/export boundary enforcement
4. IR + Runtime: `src/ir.rs` → IR lowering + lazy executor
    - Dictionary passing for type class constraints
    - Thunks + memoization for laziness

## Core implementation rules (must stay consistent)

- Module resolution: import paths are relative to the importing file’s directory.
- Prelude: auto-imported unless the module has explicit imports or disables Prelude.
- Type classes: compile constraints into explicit dictionary arguments in IR (no vtables).
- Laziness: values start as `Value::Thunk(...)`; forcing memoizes the result.
- Errors: centralized in `src/error.rs` (`Error::Msg(String)` / `Error::Io(std::io::Error)`).
- Unsafe: the main crate forbids unsafe code (`#![forbid(unsafe_code)]` in `src/lib.rs`). Unsafe is only allowed in optional feature crates under `crates/`.

## Mandatory quality gates (pre-commit)

These must pass before committing:

```bash
cargo test
cargo clippy -- -D warnings
cargo geiger
# optional (nightly)
cargo +nightly udeps
```

## Source code quality rules (review-driven)

### Cyclomatic complexity

- Recommended: ≤ 20 per function.
- 21–30: must refactor/split before merging.
- > 30: immediate refactor required; do not add branches/features on top.
- Practical approaches:
   - Split large `match` arms into helpers by responsibility.
   - Prefer data-driven tables over repeated branching.
   - Optional heuristic linting:
      - `cargo clippy -- -W clippy::cognitive_complexity -W clippy::too_many_lines`

### Source length (primarily file length)

- Recommended: ≤ 800 lines per `.rs` file.
- 801–1200: must split.
- > 1200: split immediately before adding features.
- Practical checks:
   - `find src -name '*.rs' -print0 | xargs -0 wc -l | sort -n`
   - Split by pipeline responsibility (lexer/parser/types/IR/runtime), not arbitrary chunks.

### Code clones (duplication)

- Goal: no copy/paste logic; share via helpers / small structs / traits.
- If clones already exist: every change must reduce duplication (it does not have to hit zero in one PR, but it must not increase).
- Practical checks:
   - If you touch two similar blocks, extract a shared helper before adding new behavior.
   - Use `rg`/`git grep` for repeated error strings or near-identical control flow.
   - Optional: run a clone detector (e.g., `jscpd`) if you already have Node tooling.

## Development workflow: execution-first policy

Current priority (see `docs/PriorityChecklist.md`):

1. Make the system runnable end-to-end over adding more syntax.
2. Avoid AST churn unless a failing end-to-end test proves it’s needed.
3. Add run smoke tests for multi-file imports, qualified references, export/import boundaries, `data`, `case`, and `do`.
4. Only after smoke tests exist, fill gaps in typecheck/IR/runtime.

## Stdlib policy: language-first (no ad-hoc workarounds)

The standard library is a validation target on top of the language/runtime.

- If a stdlib change “fixes” a failing test, first suspect a language/implementation bug (lexer/parser/typechecker/IR/runtime).
- Do not add ad-hoc stdlib workarounds just to make tests pass. If a temporary workaround is unavoidable, keep it minimal and file an issue.
- Always follow: reproduce → fix the implementation/spec mismatch → add regression coverage.

### Spec/implementation mismatch: issue first

If behavior is unclear or docs and Rust disagree:

1. Create the smallest `.ks` reproduction with expected vs actual behavior.
2. Open a GitHub issue titled with the subsystem (Parser/Typechecker/IR/Runtime).
3. Update docs if needed to lock the behavior as the language spec.

## Adding language features (only when tests demand it)

Follow this sequence:

1. Add a failing test (`src/lib_test.rs` or a new file under `tests/`).
2. Extend `src/lexer.rs` if new tokens are required.
3. Update `src/parser.rs` (prefer desugaring to existing AST forms).
4. Modify `src/ast.rs` only if truly necessary.
5. Update `src/types.rs` for inference/checking.
6. Update `src/ir.rs` for lowering/runtime semantics.
7. Re-run the quality gates.

Example: function clauses (`f 0 = a; f x = b`) desugar to `f = \x -> case x of 0 -> a; _ -> b`.

## CLI + REPL

Entry point: `src/main.rs` → `src/cli.rs`

```bash
cargo run -- lex tests/example_hello.ks
cargo run -- parse tests/example_hello.ks
cargo run -- typecheck tests/example_hello.ks
cargo run -- ir tests/example_hello.ks
cargo run -- run tests/example_hello.ks   # requires: main :: IO Unit
cargo run -- repl
```

REPL commands:

- `:type <expr>` / `:t`
- `:load <path>` / `:l` (import base becomes the file’s directory)
- `:modules` / `:m`
- `:quit` / `:q`

## Testing conventions

- Unit tests typically call `crate::parser::parse_module()`, `crate::types::typecheck_file()`, etc. (see `src/lib_test.rs`).
- Smoke tests use `.ks` programs under `tests/`.
- Multi-file tests verify transitive imports and qualified references.
- Example: `tests/module_import_export.ks` checks export/import boundaries.

## Component map (by file)

| File | Responsibility | Key entry points |
|------|----------------|------------------|
| `src/lexer.rs` | Tokenization + layout | `lex()` |
| `src/parser.rs` | Parsing + precedence + desugaring | `parse_module()` |
| `src/ast.rs` | AST definitions | `Module`, `Item`, `Expr`, `Pattern` |
| `src/types.rs` | Type inference + module resolution | `typecheck_file()` |
| `src/ir.rs` | IR + runtime + dictionary passing | `lower_to_ir()`, `run_main()` |
| `src/error.rs` | Error type | `Error::msg()`, `Result<T>` |
| `src/cli.rs` | CLI dispatch | `run()` |
| `src/debug.rs` | Debug helpers | (module helpers) |

## Project-specific conventions

### Parser fixity handling

- Fixity declarations (`infix`/`infixl`/`infixr` + precedence) are collected before parsing via `collect_fixities()`.
- Default fixities:
   - `*,/` (70L)
   - `+,-,++` (60L)
   - comparisons (50L)
   - `&&` (40L)
   - `||` (30L)
- The parser uses a shunting-yard style algorithm for precedence/associativity.

### Type system specifics

- Surface types: `Integer` (arbitrary precision), `Float64`, `Bool`, `Char`, `String`, `Unit`.
- Internal/FFI-facing numeric types: `i32`, `i64`, `f32`, `f64` (LLVM-aligned).
- Subtyping: integer widening allowed (`i32 <: i64`), but no float widening.
- Casts at boundaries must be checked (no silent precision loss).

### Module resolution

- Import paths are relative to the importing file’s directory (not workspace root).
- Qualified imports: `import Foo as F` enables `F.x`.
- Unqualified imports: `import Foo` enables both `x` and `Foo.x`.
- Export boundaries: `export x, y` restricts what importers can access.

### Lazy runtime

- Values start as thunks; forcing memoizes via `Rc<RefCell<Value>>`.
- IO is strict but sequenced via `IoBind` and `IoThen`.

## Documentation reference

- `docs/LanguageBNF.md` — grammar and syntax
- `docs/TypeSystem.md` — type system details
- `docs/LanguageSemantics.md` — operational semantics (laziness)
- `docs/PriorityChecklist.md` — current priorities
- `docs/ToolchainDesign.md` — long-term vision
- `README.md` — quick start

## Known gotchas

1. `Integer` is surface-level arbitrary precision, but runtime is currently backed by `i64` (overflow is a runtime error).
2. `--features unsafe_bigint` / `--features unsafe_ffi` enable experimental unsafe crates; avoid in production.
3. REPL readline support requires `--features readline`.
4. Cyclic imports are detected and rejected (error contains “cyclic imports”).
5. In function clauses, or-patterns require parentheses: `f (0 | 1) = x` (not `f 0 | 1 = x`).