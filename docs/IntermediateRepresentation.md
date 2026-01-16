# Intermediate Representation

This document describes the internal representation (IR) for thunks, IO actions, exceptions, and execution strategies.

## Purpose
- Details IR for lazy evaluation, effectful computation, exception handling, and execution backends (LLVM, JIT, interpreter).
- Type aliases are expanded before IR generation (see `TypeSystem.md`).
- For language semantics, see `LanguageSemantics.md`.
- For type system, see `TypeSystem.md`.
- For grammar and syntax, see `LanguageBNF.md`.

---

## 1. Thunk Representation

A **thunk** is a deferred computation. Internally, a thunk is represented as a data structure containing:

- **Code Pointer**: Reference to the function or code to execute.
- **Environment**: Captured variables (closure) required for execution.
- **Memoization Cell**: Stores the result after evaluation (initially empty).
- **State**: One of {Unevaluated, Evaluating, Evaluated} to detect cycles and support sharing.

### Thunk Structure (Pseudocode)
```
struct Thunk {
	CodePointer code;
	Env environment;
	Cell result;
	State state; // Unevaluated | Evaluating | Evaluated
}
```

---

## 2. IO Action Representation

An **IO action** is a thunk whose evaluation produces an effectful computation. IO actions are compiled to an internal bytecode or instruction sequence.

### IO IR Structure
- **IOAction**: A tagged union representing primitive and composite IO operations.
- **Sequencing**: Monadic bind (`>>=`) and sequencing (`>>`) are explicit in the IR.
- **Primitive Operations**: Read, write, file, network, etc.
- **User-defined IO**: Lambda or closure with environment.

#### Example IR (Pseudocode)
```
enum IOAction {
	Pure(Value),
	Bind(IOAction, Function),
	Print(Value),
	ReadLine,
	// ... other primitives
}
```

---

## 3. Intermediate Bytecode

The runtime executes IO actions by interpreting a sequence of bytecode instructions. Each instruction corresponds to a primitive operation or control flow (e.g., push thunk, force thunk, call, return, jump, etc.).

### Example Bytecode Instructions
- PUSH_THUNK <code, env>
- FORCE
- CALL <function>
- RETURN
- BIND
- PRINT
- READLINE
- ...

---

## 4. Evaluation Model

- Forcing a thunk checks its state:
	- If Unevaluated: execute code, store result, mark as Evaluated.
	- If Evaluating: detect cycles (blackhole), raise error if necessary.
	- If Evaluated: return memoized result.
- IO actions are executed by the runtime in the order specified by the monadic composition.

---

## 5. Infix Operators (Functions)

The language supports infix notation for functions, allowing any binary function to be used as an infix operator.

### Syntax
- An infix operator is written between its two arguments: `a `op` b`.
- Any function can be used in infix position by enclosing its name in backticks: ``a `f` b``.
- Standard symbolic operators (e.g., `+`, `*`, `==`) are infix by default.

### Semantics
- Infix application ``a `f` b`` is equivalent to the prefix form `f a b`.
- Operator precedence and associativity can be defined for custom operators (see grammar).

### Examples
```
1 + 2           # symbolic infix operator
3 `max` 5       # function used as infix
map (`div` 2) xs  # partially applied infix operator
```

### Internal Representation
- Infix applications are desugared to standard function application in the IR.

---

## 6. IR Execution Strategies

The intermediate representation (IR) for thunks and IO actions can be executed in multiple ways, supporting different backends and optimization levels. The following execution strategies are supported:

### 1. LLVM IR Generation
- The language IR can be compiled to LLVM IR (either as bytecode or text).
- Thunks and IO actions are lowered to LLVM functions and data structures.
- The resulting LLVM IR can be further compiled to native code or run in an LLVM-based VM.

### 2. LLVM JIT Compiler Runtime
- The IR is translated to LLVM IR and then executed via the LLVM JIT (Just-In-Time) compiler.
- Thunks are represented as heap-allocated closures; IO actions as callable LLVM functions.
- The JIT runtime manages thunk forcing, memoization, and effect sequencing.
- This approach enables high performance and native interop.

### 3. Direct IR Interpreter
- The IR can be executed directly by a custom interpreter (virtual machine) without lowering to LLVM.
- Thunks and IO actions are represented as tagged data structures and executed by the interpreter loop.
- This mode is useful for rapid prototyping, debugging, or platforms where LLVM is unavailable.

### Thunk and IOAction Handling
- In all strategies, thunks encapsulate deferred computations and may contain IO actions as their result.
- Forcing a thunk may yield a pure value or an IOAction, depending on the computation.
- IO actions are ultimately executed by the runtime (LLVM or interpreter) in the order specified by the monadic composition.

---

## 7. Exception Handling via IO

Exceptions are modeled as effectful computations within the IO monad. This approach allows exceptions to be thrown, caught, and handled in a controlled, type-safe manner.

### IR Representation
- **Throw**: An IO action that aborts the current computation with an exception value.
- **Catch**: An IO action that installs a handler for exceptions raised in a sub-computation.
- **Try**: An IO action that attempts a computation and returns either a result or an exception.

#### Example IR (Pseudocode)
```
enum IOAction {
	...
	Throw(Value),
	Catch(IOAction, Handler),
	Try(IOAction),
	...
}
```

### Bytecode Instructions
- THROW <value>
- CATCH <action> <handler>
- TRY <action>

### Evaluation Model
- `Throw` aborts the current IO computation and unwinds to the nearest enclosing `Catch`.
- `Catch` executes its action; if an exception is thrown, the handler is invoked with the exception value.
- `Try` returns a tagged result (e.g., `Either Exception Value`).

### Example Usage
```
do {
	x <- try (readFile "foo.txt");
	case x of
		Left err  -> print ("Error: " ++ err)
		Right val -> print val
}
```

### Notes
- All exceptions are values (can be strings, data types, etc.).
- Pure code cannot throw exceptions; only IO actions can raise/catch exceptions.
- This design is similar to Haskell's `Control.Exception` but is explicit in the IR.

---

## 8. Numeric Types and Conversions

The IR uses LLVM-aligned numeric types for code generation and FFI boundaries.

### Types
- Integers: `i1`, `i8`, `i16`, `i32`, `i64`, ...
- Floats: `f32`, `f64`

### Subtyping (Pure IR)
- Integer widening only: if `N < M` then `iN <: iM` (e.g., `i32 <: i64`).
- No float widening subtyping: `f32` and `f64` are distinct and unrelated for subtyping.

### Checked casts (Boundaries)
- Lowering from surface `Integer`/`Float64` to backend types may insert checked casts at boundaries (literals/FFI).
- Checked casts validate range/validity; if they fail, evaluation raises a runtime error.

---

## 9. IR Optimization

The IR can be optimized using safe transformation passes before execution. See [`IROptimization.md`](IROptimization.md) for details.

### Optimization Passes

1. **Constant Folding**: Evaluates constant expressions at compile time (e.g., `if True then 42 else 0` → `42`)
2. **Dead Code Elimination**: Removes unused bindings via reachability analysis from `main`
3. **Case Simplification**: Simplifies trivial case expressions (e.g., `case x of _ -> e` → `e`)

### Safety Guarantees

All optimizations preserve:
- **Correctness**: Observable behavior remains unchanged
- **Lazy Semantics**: Thunks and sharing are respected
- **Effects**: IO actions and exceptions are properly sequenced

### Integration

The optimizer is integrated into the compilation pipeline between IR lowering and execution:

```
Parser → Typechecker → IR Lowering → **Optimizer** → Runtime/LLVM
```

---

