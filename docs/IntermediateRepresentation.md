# Intermediate Representation

This document describes the IR that is currently implemented in `kscr`.

## Scope
- Source of IR data model: `crates/kscr_ir/src/ir.rs`.
- Lowering from AST to IR: `src/ir.rs`.
- Runtime execution of IR: `src/ir.rs`.
- Binary/container encoding for IR and interface payloads: `src/kir1.rs`, `src/ir_pack.rs`.

## IR Data Model
The implemented IR is an expression tree and pattern tree, not a dedicated VM bytecode instruction set.

- Module/item:
  - `IrModule { items: Vec<IrItem> }`
  - `IrItem::Binding { name, expr }`
- Literals:
  - `IrLiteral::{Unit, Integer, Float64, Bool, String, Char}`
- Patterns:
  - `IrPattern::{Var, Wildcard, Literal, Tuple, List, Record, RecordLoose, Cons, Constructor, Or, As, View}`
- Expressions:
  - `IrExpr::{Unit, Integer, Float64, Bool, String, Char, Var, Lambda, Apply, If, Let, Case, IoBind, IoThen, Cons, List, Tuple, Record, CheckedCast}`
- Numeric cast target:
  - `CastTarget::{I32, I64, F32, F64}`

## Lowering (AST -> IR)
Lowering entrypoint is `lower_to_ir`.

Implemented behavior:
- `data` constructors are lowered into bindings that build constructor-encoded records.
  - Encoding shape: `{ "__ctor": <name>, "__args": [...] }`.
- `do` notation lowers to `IrExpr::IoBind` and `IrExpr::IoThen`.
- Pattern bindings are lowered via temporary bindings + `case` extraction.
- Type annotations to backend numeric aliases (`i32`, `i64`, `f32`, `f64`) can insert `IrExpr::CheckedCast`.

## Optimization Passes
Default optimizer pipeline is implemented in `optimize_ir`:
1. `ConstantFolding`
2. `CaseSimplification`
3. `DeadCodeElimination`

These passes are defined in `crates/kscr_ir/src/optimize.rs` and run through `run_passes`.

## Runtime Execution Model
Runtime execution is provided by an in-process Rust evaluator.

- Entry: `run_main(&IrModule)`.
- Global bindings are memoized with states equivalent to unevaluated/evaluating/evaluated.
- `main` must evaluate to `Value::IoAction`, otherwise runtime returns an error.
- IO actions are represented by `IoAction` (for example: `Pure`, `StdoutWrite`, `ReadFile`, `WriteFile`, `ExitWith`, `Throw`, `Catch`, `Try`, bind/then variants) and executed by runtime IO handlers.
- Exceptions are modeled in IO runtime as `Throw/Catch/Try` and surfaced as uncaught runtime errors when not handled.

## Serialization and Containers
Two formats are currently implemented.

1. KIR1 container (`src/kir1.rs`)
- Magic/versioned container with section table.
- Uses `STRINGS` section and `IR` section for `IrModule` roundtrip.
- Also used for interface-oriented payloads via `KsifModule` (`STRINGS` + `INTERFACE`).
- Marked as experimental/proposal in the implementation comments.

2. Packed IR codec (`src/ir_pack.rs`)
- Direct tag-based binary encoding/decoding of IR nodes.
- Includes collection-length validation for decode safety.
- Includes unit test roundtrip for a small module.

## Backend Status
Current documented implementation status:
- Implemented: direct Rust IR execution (`run_main` path), IR optimization passes, KIR1/IR packing codecs.
- Available behind feature/command path: LLVM codegen compile path (`compile --llvm`).
- Not implemented as a stable standalone runtime target: dedicated IR bytecode VM instruction set.

## Future Work
Items that are not yet implemented as stable behavior are tracked in `docs/BACKLOG.md`.

- Dedicated IR bytecode VM instruction set and executor are backlog work (see `BG-009`).
