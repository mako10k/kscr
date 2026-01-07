# Type Classes Implementation Plan (Draft)

This plan replaces the deprecated *contextual overloading* idea with a principled, minimal type-class system.
Initial scope is intentionally small: **only `Show`**, but extended beyond primitives to cover:
- primitive types
- structural types (lists/tuples/records)
- user-defined `data` types (auto-derived)
- open-record types produced by `{..., ...}` patterns (requires a constraint on the residual row)

Dictionary-passing in IR is still the intended long-term implementation, but the current compiler may implement `show` as a builtin while enforcing the same constraints at typecheck time.

## Goals (MVP)
- Allow `show :: Show a => a -> String` / `toString` to be typed without `forall a. a -> String` escape hatch.
- Make `show` fail at **typecheck time** for non-`Show` values (e.g. functions), not at runtime.
- Provide predictable, structural `Show` for composite values and `data` values.
- Keep runtime model simple: **dictionary passing** (explicit values passed as extra args).

## Non-goals (for MVP)
- No defaulting rules (e.g. `Num`-like defaulting).
- No user-defined `instance` declarations.
- No higher-kinded classes, superclasses, functional dependencies, overlapping instances.

---

## Phase 1 — Internal-only constraints (no new surface syntax)
**Outcome:** We can represent and solve constraints internally and lower them to IR.

### 1.1 Types: represent constraints
Files: `src/types.rs`
- Extend type representation to carry constraints.
  - Option A: `Ty::Qual { constraints: Vec<Constraint>, ty: Box<Ty> }`
  - Option B: keep `Ty` unchanged, but extend `Scheme` to `Scheme { vars, constraints, ty }`
- Introduce `Constraint` type:
  - `enum Constraint { Show(Ty), ShowRow(Ty) }` (MVP: only Show/ShowRow)

### 1.2 Type environment: class + instance tables
Files: `src/types.rs`
- Add a separate environment for instances:
  - `Show Integer`, `Show Bool`, `Show String`, `Show Char`, `Show Unit`
- Extend solver rules:
  - Structural instances:
    - `Show [a]` requires `Show a`
    - `Show (a, b, ...)` requires `Show a`, `Show b`, ...
    - `Show {x: a, y: b}` requires `Show a`, `Show b`
  - Open-record instance (row constraint):
    - `Show {x: a, ... | r}` requires `Show a` and `ShowRow r`
    - `ShowRow r` means "all fields in row `r` are `Show`" and is solved structurally.
  - `data` instances (auto-derived from declarations):
    - For `data T p1 .. pn = C1 t11 .. t1k | ...`, `Show (T a1 .. an)` holds iff every constructor field type `tij[p:=a]` is `Show`.
    - Recursive occurrences of `T a1 .. an` are allowed (fixed-point instance).

### 1.3 Inference: generate `Show` constraint
Files: `src/types.rs`
- Change prelude signature to:
  - `show :: Show a => a -> String`
  - `toString :: Show a => a -> String` (alias)
- When typing `Expr::Var("show")` and applying it, constraints should flow to the call site.

### 1.4 Generalization / instantiation with constraints
Files: `src/types.rs`
- `generalize`: quantify type vars as today; keep constraints with the scheme.
- `instantiate`: freshen type vars *and* constraints.

### 1.5 Constraint solving pass
Files: `src/types.rs`
- After `infer_module`, solve constraints for each exported binding.
- Error message requirements:
  - ambiguous/unresolved constraint => type error (e.g. `show` used on a function)
  - include binding name context like current errors.

**Commit checkpoints:**
- Commit A: add constraint data structures + plumbing (no behavior change).
- Commit B: change `show/toString` types and add solver errors; update tests.

---

## Phase 2 — IR: dictionary passing for `Show`
**Outcome:** `show` becomes a normal function that takes a dictionary argument, and evaluation is uniform.

### 2.1 IR representation
Files: `src/ir.rs` (IR + evaluator)
- Long-term: add IR for dictionaries.
- Short-term (until dictionary passing lands): it is acceptable to keep `show` as a builtin *as long as* the type checker enforces the same `Show` / `ShowRow` constraints.

Recommended MVP: **explicit dict values** per primitive type to avoid record field dispatch overhead.

### 2.2 Lowering strategy
Files: `src/ir.rs` (lowering from typed module)
- When encountering a constrained use `show :: Show a => a -> String`, lower it to:
  - `show dict a`
- Insert `dict` based on solved instance at the call site.

### 2.3 Runtime: implement `Show` dictionaries
Files: `src/ir.rs`
- Provide builtin dictionaries for primitives:
  - `showInt :: Integer -> String` (can reuse `intToString`)
  - `showBool :: Bool -> String` (reuse `boolToString`)
  - etc.

**Commit checkpoints:**
- Commit C: IR support for dictionaries + evaluator builtins.
- Commit D: lowering inserts dictionaries; `show` works without runtime typecase.

---

## Phase 3 — Surface syntax (optional, after MVP is stable)
**Outcome:** Users can write class/instance declarations, but we still keep it minimal.

### 3.1 `class` / `instance` syntax
Files: `src/lexer.rs`, `src/parser.rs`, `src/ast.rs`, `src/types.rs`
- Parse:
  - `class Show a where show :: a -> String`
  - `instance Show Integer where show = intToString`
- Restrict instance heads to `Show <TypeCon>` only (no type variables) for now.

### 3.2 Instance coherence
- Disallow duplicates.
- No overlap.

**Commit checkpoints:**
- Commit E: parse/AST for class/instance.
- Commit F: typecheck instance declarations and populate instance table.

---

## Migration notes (current state)
- Current `show/toString` are `forall a. a -> String` + runtime checks.
- MVP will change them to constrained forms; update tests accordingly.
- `intToString` / `boolToString` remain useful as primitive building blocks.
