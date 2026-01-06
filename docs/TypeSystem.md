# Type System

This document describes the type system and related structural features of the lazy evaluation scripting language.

## Purpose
- Describes primitive and composite types, polymorphism, type holes, contextual overloading, module system, and indent grouping.
- For language semantics and evaluation, see `LanguageSemantics.md`.
- For grammar and syntax, see `LanguageBNF.md`.
- For internal representation, see `IntermediateRepresentation.md`.

---

## 1. Type System Overview

### Primitive Types
- **Unit**: Only one value, representing the absence of information. Syntax: `()`
- **Integer**: Arbitrary precision, platform-optimized. Syntax: `Integer`
- **Char**: Unicode character values. Syntax: `Char`
- **Bool**: Two values, `True` and `False`. Syntax: `Bool`
- **Float64**: IEEE-754 binary64 floating-point. Syntax: `Float64`
- **String**: A list of characters, defined as `[Char]`. Syntax: `String` (alias for `[Char]`)

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

## 3. Contextual Overloading

Contextual overloading allows variable bindings and function definitions to resolve differently depending on the expected type context. This enables limited ad-hoc polymorphism without explicit type classes.

### Definition Syntax
A variable or function can have multiple definitions, each annotated with a type. The actual binding is selected based on the type expected by the surrounding context.

#### Example
```
plus :: Integer -> Integer -> Integer
plus x y = x + y

plus :: [a] -> [a] -> [a]
plus xs ys = xs ++ ys

f = plus 1 2        # resolves to Integer addition
g = plus [1] [2,3]  # resolves to list concatenation
```
In this example, `plus` is overloaded for both integers and lists. The type checker selects the appropriate definition based on the type of the arguments and the expected result type.

### Type Inference
During type inference, the compiler uses the expected type from the context to disambiguate overloaded bindings. If the context is ambiguous or insufficient, a type error is reported.

### Restrictions
- Overloading is only allowed when all definitions are unambiguous in their usage contexts.
- In surface typing, implicit conversions are not performed.
- During IR elaboration, integer widening subtyping (`iN <: iM`) may be used internally; other conversions require boundary checked casts.

### Comparison
This mechanism is similar to Haskell's type classes, but does not require explicit class declarations. It is also related to ML's value restriction and SML's ad-hoc overloading.

---

## 4. Module System

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
- Type variables and contextual overloads are resolved within module boundaries unless explicitly re-exported.

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

### Compilation Units
- Each module is compiled separately. Only exported definitions are visible to other modules.

---

## 5. Indent Grouping

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
		print (x + 1)
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

## 6. Type Inference and Safety

The language employs static type inference to ensure type safety at compile time.

### Hindley-Milner Type Inference
- The type system is based on Hindley-Milner type inference, extended with contextual overloading.
- Types are inferred automatically for expressions without explicit annotations.
- Type annotations can be provided for clarity or to resolve ambiguities.
- Integer literals default to `Integer`; when constrained at boundaries to a backend integer type (e.g., `i32`), the compiler inserts a checked cast.

### Type Safety Guarantees
- **No runtime *type* errors**: Well-typed programs do not encounter type errors at runtime.
- **Exhaustiveness checking**: Pattern matches are checked for exhaustiveness at compile time.
- **Effect isolation**: Pure and effectful code are separated by the type system (see `LanguageSemantics.md`).
- **Checked cast failure**: Inserted checked casts may still fail at runtime (overflow/invalid conversion), raising a runtime error.

---

## 7. Patterns and Bindings

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
- **Record Pattern (Strict/Loose)**: Matches record fields exactly or partially (e.g., `{x, y}` or `{x, ...}`).
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
