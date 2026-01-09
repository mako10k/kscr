# Type System

This document describes the type system and related structural features of the lazy evaluation scripting language.

## Purpose
- Describes primitive and composite types, polymorphism, type holes, module system, and indent grouping.
- For language semantics and evaluation, see `LanguageSemantics.md`.
- For grammar and syntax, see `LanguageBNF.md`.
- For internal representation, see `IntermediateRepresentation.md`.

---

## 1. Type System Overview

### Primitive Types
- **Unit**: Only one value, representing the absence of information. Syntax: `()`
- **Integer (MVP)**: Signed 64-bit integer (`i64`). Syntax: `Integer` (overflow is a runtime error)
- **Char**: Unicode character values. Syntax: `Char`
- **Bool**: Two values, `True` and `False`. Syntax: `Bool`
- **Float64**: IEEE-754 binary64 floating-point. Syntax: `Float64`
- **String (MVP)**: Primitive string values. Syntax: `String`

### Composite Types
- **Tuple**: Fixed-size, ordered collections. Syntax: `(TyA, TyB, ...)`
- **List**: Homogeneous, possibly infinite sequences. Syntax: `[Ty]`
- **String**: Syntactic sugar for `[Char]` (see above).
- **Record**: Named fields, supporting both closed and open variants. Syntax: `{ TagA: TyA, ... }`
- **Function**: First-class, curried by default. Syntax: `TyA -> TyB`
- **Data**: Algebraic data types. Syntax: `TypeName TyArgA ...`

### Type Annotation
- Expressions and values can be annotated with types using: `Val :: Ty`

### Type Aliases (Type Synonyms)
Type aliases give an existing type a new name for readability; they do not create a new runtime representation.

- **Syntax**: `type Name a b = Ty`
- **Example**:
	```
	type String = [Char]
	type Pair a b = (a, b)
	```
- **Semantics**: the type checker expands aliases during type checking (aliases are transparent).

### LLVM-aligned Backend Numeric Types (Internal)
The compiler may lower surface types to LLVM-aligned backend types for literals and FFI boundaries.

- **Backend integer types**: `i1`, `i8`, `i16`, `i32`, `i64`, ...
- **Backend float types**: `f32`, `f64`

#### Subtyping (Pure IR)
- Integer widening is allowed: if `N < M` then `iN <: iM` (e.g., `i32 <: i64`).
- No float widening subtyping: `f32 <: f64` is **not** allowed.

#### Checked conversions (Boundaries)
- Conversions that may lose information are inserted only at boundaries (e.g., integer literals/FFI arguments/FFI returns) as **checked casts**.
- Examples of checked casts: `Integer -> i64`, `i64 -> i32`, `f64 -> f32`.
- If a checked cast would overflow, underflow, or otherwise be invalid, evaluation raises a runtime error.

---

## 2. Polymorphism and Type Holes

- **Type Variables**: Used for generic programming and type inference. Syntax: `%VarName`
- **Type Holes**: Represent unknown types to be solved during type checking. Syntax: `?` or `?VarName`
- **Parametric Polymorphism**: Functions and data types may be generic over type variables.

---

## 3. Type Classes (Implemented)

The language provides a principled type-class system for ad-hoc polymorphism.

### Current Implementation
- **Show typeclass**: Convert values to strings with `show :: Show a => a -> String`
- **Eq typeclass**: Equality comparison with `(==) :: Eq a => a -> a -> Bool` and `(/=) :: Eq a => a -> a -> Bool`
- **Dictionary passing**: Constraints are compiled to explicit dictionary arguments in IR
- **Automatic deriving**: Use `deriving Show`, `deriving Eq`, or `deriving (Show, Eq)` on data declarations

### Supported Types
- Primitive types: `Integer`, `Bool`, `String`, `Char`, `Unit`, `Float64`
- Structural types: lists `[a]`, tuples `(a, b, ...)`, records `{x: a, y: b}`
- User-defined data types (via deriving)
- Open-record types (with `ShowRow`/`EqRow` constraints)

### Example
```haskell
-- Single typeclass
data Maybe a = Nothing | Just a deriving Show

-- Multiple typeclasses (requires parentheses)
data Person = Person String Integer deriving (Eq, Show)

-- Usage
main = do
  print (show (Person "Alice" 30))
  print (show (Person "Bob" 25 == Person "Alice" 30))
```

### Design
- Type schemes carry constraints: `forall a. Show a => a -> String`
- Constraints are solved during type inference
- Non-showable types (e.g., functions) are rejected at typecheck time
- Dictionary passing ensures uniform runtime behavior

For detailed implementation, see `TypeClassesPlan.md`.

---

## 4. Overloading (Deprecated)

The original design included *contextual overloading* (selecting a definition based on expected type context).
This approach is **not implemented** in the current `kscr` compiler, and the design is considered deprecated.

For now, ad-hoc polymorphism is handled either by:
- explicit, separate names (e.g. `intToString`, `boolToString`), or
- a small number of polymorphic builtins (e.g. `show/toString :: forall a. a -> String`) with runtime behavior.

Future direction: introduce type classes (constraints + dictionary passing) for principled overloading.

---

## 4. Overloading (Historical Note - Deprecated)

The original design included *contextual overloading* (selecting a definition based on expected type context).
This approach was **not implemented** and is considered deprecated.

**Current approach:** Type classes (constraints + dictionary passing) provide principled ad-hoc polymorphism.
See section 3 above for the implemented typeclass system.

---

## 5. Module System

The language provides a module system for organizing code, managing namespaces, and supporting separate compilation.

### Module Definition
A module is defined in a file, optionally with a module header:
```
module MyModule where
	import OtherModule
	export foo, bar
	foo x = ...
	bar y = ...
```
- The `module` declaration names the module.
- The `import` statement brings definitions from other modules into scope.
- The `export` statement (optional) lists the public interface; if omitted, all top-level bindings are exported.

### Namespaces
- Each module forms its own namespace. Imported names can be qualified to avoid conflicts (e.g., `import OtherModule as OM`, use as `OM.foo`).

### Type System Integration
- Types, type synonyms, and data types can be defined and exported from modules.
- Type variables are resolved within module boundaries unless explicitly re-exported.

### Re-export and Dependency
- Modules can re-export imported definitions for building layered APIs.
- Cyclic dependencies between modules are not allowed.

### Example
```
module Math where
	add :: Integer -> Integer -> Integer
	add x y = x + y

module Main where
	import Math
	main :: IO ()
	main = print (add 1 2)
```

Note: the current `kscr` implementation provides low-level IO primitives such as `stdoutWrite` and `stdinReadLine`.
For early ergonomics/observability, `readLine` and `print` are currently provided as temporary built-ins.
In the future, `readLine`/`print` are expected to be implemented as library functions on top of lower-level IO primitives such as `stdinReadLine`/`stdoutWrite`.

### Compilation Units
- Each module is compiled separately. Only exported definitions are visible to other modules.

---

## 6. Indent Grouping

The language supports indent-based grouping for structuring code blocks, similar to Python and Haskell's layout rule.

### Syntax
- Code blocks can be grouped by consistent indentation instead of explicit braces or keywords.
- Indentation level determines the scope of definitions and expressions.

#### Example
```
module Main where
	import Math
	main :: IO ()
	main = do
		x <- readLine
		stdoutWrite x
```
In this example, all indented lines under `module Main where` belong to the `Main` module. The `do` block is also grouped by indentation.

### Semantics
- Indent grouping is used for:
	- Module and import declarations
	- Function and value definitions
	- `do` notation blocks
	- `where` and `let` clauses
- The parser interprets consecutive lines with the same indentation as belonging to the same block.
- Mixing indentation and explicit braces is discouraged.

---

## 7. Type Inference and Safety

The language employs static type inference to ensure type safety at compile time.

### Hindley-Milner Type Inference
- The type system is based on Hindley-Milner type inference.
- Types are inferred automatically for expressions without explicit annotations.
- Type annotations can be provided for clarity or to resolve ambiguities.
- Integer literals default to `Integer`; when constrained at boundaries to a backend integer type (e.g., `i32`), the compiler inserts a checked cast.

### Type Safety Guarantees
- **No runtime *type* errors**: Well-typed programs do not encounter type errors at runtime.
- **Exhaustiveness checking**: Pattern matches are checked for exhaustiveness at compile time.
- **Effect isolation**: Pure and effectful code are separated by the type system (see `LanguageSemantics.md`).
- **Checked cast failure**: Inserted checked casts may still fail at runtime (overflow/invalid conversion), raising a runtime error.

---

## 8. Patterns and Bindings

Patterns and variable bindings are central to the type system, enabling expressive deconstruction and safe value extraction.

### Pattern Types
- **Literal Pattern**: Matches a specific literal value (e.g., `42`, `'a'`, `True`).
- **Variable Pattern**: Binds the matched value to a variable (e.g., `x`).
- **Wildcard Pattern**: Matches any value, does not bind (e.g., `_`).
- **As Pattern**: Binds a value to a variable while matching a subpattern (e.g., `x @ (a, b)`).
- **Hole Pattern**: Placeholder for an unknown pattern (e.g., `?`, `?name`).
- **Tuple Pattern**: Matches tuple structure (e.g., `(a, b)`).
- **List Pattern**: Matches list literals (e.g., `[a, b, c]`).
- **Cons Pattern**: Matches head and tail of a list (e.g., `x : xs`).
- **Record Pattern (Strict/Loose)**: Matches record fields exactly or partially (e.g., `{x: p, y: q}` or `{x: p, ...}` / `{x: p, ...rest}`).
  - A loose record pattern `{x: p, ...}` gives the scrutinee an *open-record type* (required fields + residual row).
  - `{x: p, ...rest}` additionally binds `rest` to the residual record, and introduces `Lacks x rest` constraints so that `rest` cannot contain any required field labels.
  - If such a value is passed to `show`, the type checker requires a constraint that the residual row is also `Show` (see `ShowRow` in `TypeClassesPlan.md`).
- **Data Pattern**: Matches algebraic data constructors (e.g., `Just x`).
- **Or Pattern**: Matches if either subpattern matches (e.g., `p1 | p2`).
- **View Pattern**: Applies a function before matching (e.g., `p <- f`).

### Binding Semantics
- **Type Safety**: Each variable bound in a pattern is assigned a type inferred from the matched value.
- **Non-overlapping**: Variable names in a single pattern must not overlap.
- **Exhaustiveness**: Pattern matches should be exhaustive or explicitly handle non-exhaustive cases (runtime error otherwise).
- **Scoping**: Variables bound in a pattern are in scope only in the corresponding expression branch.
- **Shadowing**: Inner bindings can shadow outer ones, but shadowing is discouraged for clarity.

### Example
```
case expr of
	(x, y)        -> ...   # tuple pattern
	Just n        -> ...   # data pattern
	x : xs        -> ...   # cons pattern
	_             -> ...   # wildcard
	(a, b) | p a  -> ...   # or pattern with guard
```

### Where/Let Bindings
- **Where**: Local bindings at the end of an expression, visible only within that expression.
- **Let**: Local bindings within an expression, visible in the inner scope.
- Both forms support pattern bindings, not just simple variables.

### Pattern Binding Example
```
let (a, b) = foo in a + b
where Just x = bar
```

---
