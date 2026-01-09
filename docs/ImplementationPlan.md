# Implementation Plan

This document describes the implementation plan for **kscr**, a Rust-based toolchain for a lazy functional scripting language.

Scope notes:
- The language design is described in `docs/LanguageBNF.md`, `docs/TypeSystem.md`, `docs/LanguageSemantics.md`, and `docs/IntermediateRepresentation.md`.
- The current codebase is a scaffold; it provides a minimal CLI and placeholder modules.

---

## Milestone 0 — Project Foundations (Buildable Skeleton)

### Mid-goal 0.1 — Repository ergonomics
- **Small-goal 0.1.1**: Ensure `cargo test` and `cargo run -- help` are stable in CI/local.
- **Small-goal 0.1.2**: Define crate/module boundaries (`lexer`, `parser`, `types`, `ir`, `cli`, `error`).
- **Small-goal 0.1.3**: Decide error-reporting format (source spans, pretty messages) and keep it consistent.

### Mid-goal 0.2 — Basic golden tests
- **Small-goal 0.2.1**: Add a small set of `.ks` fixtures.
- **Small-goal 0.2.2**: Add parser/lexer golden tests (expected AST/debug output).

---

## Milestone 1 — Lexer + Parser (Surface Syntax)

### Mid-goal 1.1 — Lexer implementation
- **Small-goal 1.1.1**: Tokenize identifiers, keywords (`module`, `import`, `export`, `data`, `type`, `let`, `in`, `where`, `case`, `of`, `if`, `then`, `else`, `do`).
- **Small-goal 1.1.2**: Tokenize literals: integers, Float64 literals, chars, strings, booleans, unit.
- **Small-goal 1.1.3**: Implement comments (line and nested block) and shebang stripping.
- **Small-goal 1.1.4**: Implement indentation tracking and emit INDENT/DEDENT.

### Mid-goal 1.2 — Parser implementation
- **Small-goal 1.2.1**: Parse module blocks (`module ... where` + indent group).
- **Small-goal 1.2.2**: Parse top-level items: bindings, `data` declarations, `type` aliases.
- **Small-goal 1.2.3**: Parse expression grammar (lambda, application, infix application, let/where, if, case, list/tuple/record).
- **Small-goal 1.2.4**: Parse patterns (including view/or patterns) and ensure binding rules (no duplicate variables).

---

## Milestone 2 — Type System Core (HM + Aliases + Effects)

### Mid-goal 2.1 — Type representation + alias expansion
- **Small-goal 2.1.1**: Represent surface types (`Integer`, `Bool`, `Float64`, `String`, lists, tuples, records, functions, ADTs).
- **Small-goal 2.1.2**: Implement type alias declaration storage and expansion.
- **Small-goal 2.1.3**: Implement type holes (`?` / `?name`) as constraints/placeholders.

### Mid-goal 2.2 — Hindley–Milner inference
- **Small-goal 2.2.1**: Implement unification, substitution, generalization/instantiation.
- **Small-goal 2.2.2**: Infer types for core expressions and patterns.
- **Small-goal 2.2.3**: Provide helpful type errors (unification mismatch, occurs check, missing constraints).

### Mid-goal 2.3 — Effect typing boundary (IO)
- **Small-goal 2.3.1**: Represent `IO` type constructor.
- **Small-goal 2.3.2**: Ensure the program entrypoint policy (`main :: IO ()`) is enforced.

---

## Milestone 3 — Type Classes (✅ Implemented)

### Mid-goal 3.1 — Constraints + dictionaries
- **Small-goal 3.1.1**: ✅ Represent constraints in types (e.g. `Show a => a -> String`).
- **Small-goal 3.1.2**: ✅ Lower constraints via dictionary passing in IR.
- **Small-goal 3.1.3**: ✅ Support `Show` and `Eq` typeclasses.

### Mid-goal 3.2 — Show and Eq typeclasses
- **Small-goal 3.2.1**: ✅ Implement `Show` and `Eq` for primitive types.
- **Small-goal 3.2.2**: ✅ Implement structural instances (lists, tuples, records).
- **Small-goal 3.2.3**: ✅ Support `deriving Show`, `deriving Eq`, and `deriving (Eq, Show)` for data types.

**Status**: Type classes are fully implemented with dictionary passing. See `TypeClassesPlan.md` for implementation details.

---

## Milestone 4 — IR Elaboration (Typed AST → Pure IR)

### Mid-goal 4.1 — Define Pure IR data structures
- **Small-goal 4.1.1**: Define IR for thunks/closures and core expressions.
- **Small-goal 4.1.2**: Add IR nodes for pattern matching/case and function application.

### Mid-goal 4.2 — Numeric lowering policy (LLVM-aligned)
- **Small-goal 4.2.1**: Introduce backend numeric types (integers `iN`, floats `f32/f64`).
- **Small-goal 4.2.2**: Implement **pure IR subtyping**: integer widening only (`i32 <: i64`), no float widening subtyping.
- **Small-goal 4.2.3**: Implement **checked casts at boundaries** (literals/FFI), failing at runtime on overflow/invalid conversion.

---

## Milestone 5 — Runtime + IR Interpreter (Correctness First)

### Note: String representation (future)
- **Current (MVP):** plain `String` values + minimal primitives (e.g. concatenation) for basic usability.
- **Future direction:** make String an internal structure where we can explicitly control **substrings**, **interning**, and **concatenation/ropes** via dedicated builtin primitives (so stdlib can choose policies without Rust-side ad-hoc helpers).

### Mid-goal 5.1 — Thunk runtime
- **Small-goal 5.1.1**: Implement thunk states (Unevaluated/Evaluating/Evaluated) and memoization.
- **Small-goal 5.1.2**: Implement blackholing/cycle detection policy.

### Mid-goal 5.2 — IOAction interpreter
- **Small-goal 5.2.1**: Represent and execute primitive IO actions.
- **Small-goal 5.2.2**: Implement monadic sequencing semantics (bind/then).

### Mid-goal 5.3 — Exceptions via IO
- **Small-goal 5.3.1**: Implement IR/runtime support for Throw/Catch/Try.
- **Small-goal 5.3.2**: Add tests for exception ordering and handler scoping.

---

## Milestone 6 — FFI (C ABI) with Safety

### Mid-goal 6.1 — FFI type mapping
- **Small-goal 6.1.1**: Define mapping between surface types and FFI/backend types.
- **Small-goal 6.1.2**: Enforce checked casts on boundary values (no silent truncation).

### Mid-goal 6.2 — Minimal FFI API
- **Small-goal 6.2.1**: Implement calling a small set of C functions (e.g., puts/strlen) as a proof.
- **Small-goal 6.2.2**: Decide/implement ownership and string encoding rules.

---

## Milestone 7 — LLVM Backend (Optional, After Interpreter)

### Mid-goal 7.1 — IR → LLVM IR lowering
- **Small-goal 7.1.1**: Lower a pure subset (literals, arithmetic, calls).
- **Small-goal 7.1.2**: Lower thunks/closures to heap structures and functions.

### Mid-goal 7.2 — JIT execution
- **Small-goal 7.2.1**: Run lowered code via LLVM JIT.
- **Small-goal 7.2.2**: Validate semantics equivalence vs the interpreter.

---

## Milestone 8 — Tooling (Formatter/Linter) and UX

### Mid-goal 8.1 — Diagnostics and source mapping
- **Small-goal 8.1.1**: Propagate spans through lexer/parser/typechecker.
- **Small-goal 8.1.2**: Improve CLI errors and add `--verbose`/`--debug` modes.

### Mid-goal 8.2 — Formatter / Linter
- **Small-goal 8.2.1**: Formatter for a stable subset.
- **Small-goal 8.2.2**: Linter rules (unused bindings, non-exhaustive matches, shadowing warnings).

---

## Suggested execution order
1. Milestone 1 (Lexer/Parser) + Milestone 2 (HM core) as the main backbone.
2. Milestone 4/5 (IR + interpreter runtime) for semantic validation.
3. Milestone 6 (FFI) once numeric lowering + checked casts are well tested.
4. Milestone 7 (LLVM) only after interpreter semantics are stable.
