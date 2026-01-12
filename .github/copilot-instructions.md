# Copilot Instructions for kscr

## Project Overview
kscr is a Rust-based toolchain for a lazy functional scripting language with Haskell-like semantics. The language features:
- **Lazy evaluation** (call-by-need with memoization)
- **Hindley-Milner type inference** with type classes (`Show`, `Eq`)
- **Module system** with qualified imports and export control
- **Pattern matching** including record patterns, guards, and or-patterns
- **Algebraic data types** with deriving support
- **Do-notation** for monadic IO operations
- **Interactive REPL** (with optional readline support via `--features readline`)

## Architecture: Multi-Phase Pipeline

The toolchain follows a traditional compiler pipeline:

1. **Lexer** (`src/lexer.rs`) → Tokens
2. **Parser** (`src/parser.rs`) → AST (`src/ast.rs`)
   - Collects fixity declarations and applies precedence during parse
   - Desugars Haskell-style function clauses (e.g., `f 0 = a; f x = b`) into single lambda+case bindings
3. **Typechecker** (`src/types.rs`) → TypedModule with inferred types
   - Hindley-Milner unification with constraints for type classes
   - Multi-file module resolution and import/export boundary enforcement
4. **IR Lowering** (`src/ir.rs`) → IrModule
   - Converts AST to intermediate representation
   - Performs dictionary passing for type class constraints
5. **Runtime/Executor** (also in `src/ir.rs`) → Lazy evaluation with thunks and memoization

Key architectural decisions:
- **Module resolution**: Import paths are relative to the importing file's directory. Prelude auto-imports unless explicitly disabled.
- **Type class implementation**: Constraints compile to explicit dictionary arguments in IR (no vtables).
- **Lazy evaluation**: All values are `Value::Thunk` until forced; memoization prevents re-evaluation.
- **Error handling**: Centralized in `src/error.rs` with a simple `Error::Msg(String)` or `Error::Io(std::io::Error)`.

## Mandatory Quality Gates (Pre-Commit)

**Before every commit**, these checks MUST pass:

```bash
# 1. Run all tests
cargo test

# 2. Zero clippy warnings
cargo clippy -- -D warnings

# 3. Verify no unsafe code (main crate)
cargo geiger
# ⚠️ Note: Optional features `unsafe_bigint` and `unsafe_ffi` are isolated in `crates/` and disabled by default

# 4. (Optional, nightly only) Check for unused deps
cargo +nightly udeps
```

**DO NOT commit if any check fails.** Fix issues first.

The codebase enforces `#![forbid(unsafe_code)]` in `src/lib.rs`. Unsafe code is only allowed in optional feature crates (`kscr_unsafe_bigint`, `kscr_unsafe_ffi`) which are disabled by default.

## Development Workflow: Execution-First Policy

**Current priority** (see `docs/PriorityChecklist.md` for full context):
1. **Make the system runnable end-to-end** over adding more syntax
2. Avoid AST churn unless a failing end-to-end test proves it's needed
3. Add **run smoke tests** that exercise multi-file imports, qualified references, export/import boundaries, `data` types, `case`, and `do`
4. Only after smoke tests exist, incrementally fill gaps in typecheck/IR/runtime

When design choices are ambiguous, **lean toward Haskell-like semantics** (especially for import/qualified name resolution).

### Adding New Language Features (Only When Tests Demand It)

Follow this sequence:
1. Add failing test case in `src/lib_test.rs` or create new `.ks` file in `tests/`
2. Extend `src/lexer.rs` if new tokens are needed
3. Update `src/parser.rs` to recognize new syntax (consider desugaring to existing AST forms)
4. Modify `src/ast.rs` only if truly necessary (prefer parser desugar)
5. Update `src/types.rs` for type inference/checking
6. Adjust `src/ir.rs` for IR lowering and runtime behavior
7. Verify all quality gates pass

Example: Function clause syntax (`f 0 = a; f x = b`) desugars to `f = \x -> case x of 0 -> a; _ -> b` in the parser, avoiding AST changes.

### CLI Commands & Testing

**Key commands** (entry point: `src/main.rs` → `src/cli.rs`):
```bash
cargo run -- lex tests/example_hello.ks       # Debug: show tokens
cargo run -- parse tests/example_hello.ks     # Debug: show AST
cargo run -- typecheck tests/example_hello.ks # Show inferred types
cargo run -- ir tests/example_hello.ks        # Debug: show IR
cargo run -- run tests/example_hello.ks       # Execute (requires main :: IO Unit)
cargo run -- repl                             # Interactive REPL
```

**REPL commands**:
- `:type <expr>` (or `:t`) — Show type of expression
- `:load <path>` (or `:l`) — Load module from file (sets import base to file's directory)
- `:modules` (or `:m`) — List loaded modules
- `:quit` (or `:q`) — Exit

**Test patterns** (`src/lib_test.rs`):
- Unit tests use `crate::parser::parse_module()`, `crate::types::typecheck_file()`, etc.
- Smoke tests read `.ks` files from `tests/` directory
- Import/multi-file tests verify transitive imports and qualified references
- Example: `tests/module_import_export.ks` tests export/import boundaries

### File-by-File Component Guide

| File | Responsibility | Key Types/Functions |
|------|---------------|---------------------|
| `src/lexer.rs` | Tokenization, indent-based layout | `lex()` → `Vec<Token>` |
| `src/parser.rs` | Parsing with operator precedence, clause desugaring | `parse_module()` → `ast::Module` |
| `src/ast.rs` | Abstract syntax tree definitions | `Module`, `Item`, `Expr`, `Pattern` |
| `src/types.rs` | Hindley-Milner inference, module resolution | `typecheck_file()` → `TypedModule` |
| `src/ir.rs` | IR lowering, dictionary passing, lazy runtime | `lower_to_ir()`, `run_main()` |
| `src/error.rs` | Centralized error type | `Error::msg()`, `Result<T>` |
| `src/cli.rs` | CLI command dispatch | `run()` function matches commands |
| `src/debug.rs` | Debug utilities | Helper functions for debugging |

## Project-Specific Conventions

### Parser Fixity Handling
- Fixity declarations (`infix`/`infixl`/`infixr` + precedence) are collected before parsing via `collect_fixities()`
- Default fixities: `*,/` (70L), `+,-,++` (60L), comparison (50L), `&&` (40L), `||` (30L)
- Shunting yard algorithm in parser handles operator precedence and associativity

### Type System Specifics
- Surface types: `Integer` (arbitrary precision), `Float64`, `Bool`, `Char`, `String`, `Unit`
- Backend types (internal): LLVM-aligned `i32`, `i64`, `f32`, `f64` for literals/FFI
- Subtyping: Integer widening allowed (`i32 <: i64`), but **no float widening**
- Checked casts at boundaries prevent silent precision loss

### Module Resolution
- Import paths are **relative to the importing file's directory** (not the workspace root)
- Prelude is auto-imported unless module has explicit `import` declarations
- Qualified imports: `import Foo as F` allows `F.x` syntax
- Unqualified imports: `import Foo` allows both `x` and `Foo.x`
- Export boundaries: `export x, y` restricts what importers can see

### Lazy Evaluation Runtime
- All values start as `Value::Thunk(expr, env)`
- Forcing evaluates the thunk and memoizes the result (`Rc<RefCell<Value>>`)
- IO actions are strict but sequenced via `IoBind` and `IoThen` constructs

## Documentation Reference

- `docs/LanguageBNF.md` — Formal grammar and syntax rules
- `docs/TypeSystem.md` — Type system details, polymorphism, type classes
- `docs/LanguageSemantics.md` — Lazy evaluation semantics and operational details
- `docs/PriorityChecklist.md` — Current priorities and roadmap (updated by maintainers)
- `docs/ToolchainDesign.md` — Long-term toolchain vision (LLVM JIT, FFI, etc.)
- `README.md` — Quick start, usage examples, feature flags

## Known Gotchas

1. **Integer type**: Surface `Integer` is arbitrary precision, but currently backed by `i64` in runtime (overflow is runtime error)
2. **Unsafe features**: `--features unsafe_bigint` and `--features unsafe_ffi` enable experimental unsafe crates; avoid in production
3. **Readline**: REPL history/editing requires `--features readline` (adds unsafe dependency via `rustyline`)
4. **Cyclic imports**: Detected and rejected with error message containing "cyclic imports"
5. **Pattern or-syntax**: In function clauses, or-patterns require parentheses: `f (0 | 1) = x` (not `f 0 | 1 = x`)

---

For deeper understanding, read `docs/PriorityChecklist.md` for current focus areas and consult individual source files. When uncertain, ask for clarification or examples from maintainers.