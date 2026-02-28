# kscr

A lazy functional scripting language with strong static typing, Hindley-Milner type inference, and typeclass support.

## Features

- **Lazy Evaluation**: Call-by-need evaluation with automatic memoization
- **Strong Static Typing**: Hindley-Milner type inference with type annotations
- **Typeclasses**: Support for `Show` and `Eq` with automatic deriving
- **Module System**: Import/export with qualified names and namespace management
- **Pattern Matching**: Comprehensive pattern matching including record patterns, as-patterns, and guards
- **Algebraic Data Types**: `data` declarations with deriving support
- **Do-notation**: Monadic composition for IO operations
- **REPL**: Interactive environment with type inspection and module loading
- **Language Server Protocol (LSP)**: IDE support with diagnostics and symbols

## Prerequisites

Install Rust (includes `cargo`) via rustup:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

## Build / Test

```bash
# Run all tests
cargo test

# Build the project
cargo build

# Build with optional features (readline support for REPL)
cargo build --features readline
```

## Usage

### Command Line Interface

```bash
# Show help
cargo run -- help

# Show version information
cargo run -- version

# Run a program (requires main :: IO Unit)
cargo run -- run path/to/file.ks

# Parse and print AST (debug)
cargo run -- parse path/to/file.ks

# Lex and print tokens (debug)
cargo run -- lex path/to/file.ks

# Typecheck and print inferred types
cargo run -- typecheck path/to/file.ks

# Show all types including imported names
cargo run -- typecheck --all path/to/file.ks

# Lower to IR and print (debug)
cargo run -- ir path/to/file.ks

# Generate LLVM IR (requires --features llvm)
cargo run --features llvm -- llvm-ir path/to/file.ks

# Compile to a native executable (stage 1: pack IR + thin runner)
# - Output defaults to `path/to/file` (extension stripped)
# - The produced executable contains packed IR and runs it via kscr's runtime
cargo run -- compile path/to/file.ks

# Also emits an interface-only artifact (.ksif) for separate compilation experiments
# - Default output: `./target/ksif/<file>.ksif`
# - Override output directory:
cargo run -- compile path/to/file.ks --ksif-out ./target/custom_ksif

# Specify output path
cargo run -- compile path/to/file.ks -o ./a.out

# Release build of the runner (slower compile, faster startup/runtime)
cargo run -- compile path/to/file.ks --release -o ./a.out

# Start interactive REPL
cargo run -- repl
```

### REPL Commands

The REPL provides an interactive environment for experimenting with the language:

```
> :type <expr>        # Show the type of an expression
> :info <name>        # Show the type of a name
> :load <path>        # Load a module from file
> :modules            # List loaded modules (always includes Prelude)
> :quit               # Exit REPL

# Command names accept unique prefixes:
> :t 1 + 2            # Same as :type
> :i Just             # Same as :info
> :q                  # Same as :quit
```

For enhanced editing and history support, build with readline feature:
```bash
cargo build --features readline
```

## Language Examples

### Hello World

```haskell
module Main where
  import Prelude
  
  main = do
    putStrLn "Hello, World!"
```

### Using Typeclasses

```haskell
module Example where
  export main
  
  -- Automatically derive Show for custom types
  data Person = Person String Integer deriving Show
  
  -- Derive multiple typeclasses (requires parentheses)
  data Color = Red | Green | Blue deriving (Eq, Show)
  
  main = do
    stdoutWrite (show (Person "Alice" 30))
    stdoutWrite "\n"
    stdoutWrite (show (Red == Blue))
    stdoutWrite "\n"
```

### Module System

```haskell
-- In MyModule.ks
module MyModule where
  export add, multiply
  
  add :: Integer -> Integer -> Integer
  add x y = x + y
  
  multiply :: Integer -> Integer -> Integer
  multiply x y = x * y
  
  -- private helper
  helper = \x -> x + 1

-- In Main.ks
module Main where
  import MyModule as M
  
  main = do
    print (show (M.add 1 2))
```

### Pattern Matching

```haskell
module PatternExample where
  import Prelude
  
  data Maybe a = Nothing | Just a deriving Show
  
  fromMaybe = \default -> \m -> case m of
    Nothing -> default
    Just x -> x
  
  main = do
    print (show (fromMaybe 0 (Just 42)))
    print (show (fromMaybe 0 Nothing))
```

## Standard Library

The `stdlib/Prelude.ks` provides common functions and types:
- **IO functions**: `print`, `readLine`, `putStr`, `putStrLn`
- **List functions**: `map`, `filter`, `concat`, `append`
- **Utility functions**: `id`, `const`
- **Data types**: `Maybe`, `Either` (with deriving Show)
- **Maybe utilities**: `maybe`, `fromMaybe`, `isJust`, `isNothing`, `listToMaybe`, `maybeToList`, `mapMaybe`, `catMaybes`

### Stdlib discovery

`kscr` resolves the stdlib root directory in this order:

1. CLI: `--stdlib-dir <path>`
2. Env: `KSCR_STDLIB_DIR`
3. `$EXE_DIR/stdlib` (next to the `kscr` executable)
4. (dev/test only) `CARGO_MANIFEST_DIR/stdlib`

5. Embedded stdlib (when bundled in the binary): `kscr` contains an embedded copy of `stdlib/` when distributed via `cargo install` or other packaging. At runtime, if no other stdlib location is found, `kscr` will extract the embedded stdlib into your user data directory (typically `$XDG_DATA_HOME/kscr/stdlib` or `$HOME/.local/share/kscr/stdlib`) and use that location.

You can explicitly extract the embedded stdlib with the CLI command:

```bash
# Extract embedded stdlib to the user data directory
kscr --install-stdlib
# or equivalently
kscr install-stdlib
```

If extraction succeeds the tool prints the extracted path. If you prefer a custom location, continue to use `--stdlib-dir <path>` or set `KSCR_STDLIB_DIR`.

## Documentation

Detailed documentation is available in the `docs/` directory:
- `LanguageBNF.md`: Complete grammar and syntax specification
- `LanguageSemantics.md`: Language philosophy and evaluation strategy
- `TypeSystem.md`: Type system details and module system
- `TypeClassesPlan.md`: Typeclass implementation details
- `IntermediateRepresentation.md`: IR design and lowering
- `ImplementationPlan.md`: Development roadmap
- `ToolchainDesign.md`: Compiler architecture
- `LSPDesign.md`: Language Server Protocol implementation design

## Language Server Protocol (LSP)

The kscr language server provides IDE features for any LSP-compatible editor.

### Building the LSP Server

```bash
cd crates/kscr_lsp
cargo build --release
```

The binary will be at `crates/kscr_lsp/target/release/kscr-lsp`.

### Current LSP Features

- **Real-time Diagnostics**: Parse, import, and type errors
- **Document Symbols**: Outline view of functions, types, and classes
- **Hover**: Type information on hover
- **Go-to-Definition**: Navigate to definitions
- **Completion**: Context-aware symbol completion
- **Find References**: Symbol usage search
- **Rename**: Workspace edit generation for symbol rename
- **Semantic Tokens**: Semantic highlighting tokens (`full`, `range`, `full/delta`)

### Editor Integration

See `crates/kscr_lsp/README.md` for integration instructions with VS Code, Vim, Emacs, and other editors.

## Development Status

Current implementation includes:
- ✅ Lexer with indent-aware tokenization
- ✅ Parser supporting full language syntax
- ✅ Type inference with Hindley-Milner algorithm
- ✅ Typeclass constraints (Show, Eq) with dictionary passing
- ✅ Module system with import/export
- ✅ IR with basic evaluator
- ✅ Pattern matching and case expressions
- ✅ Do-notation for IO
- ✅ Interactive REPL
- ✅ Automatic deriving for typeclasses
- ✅ LSP server with diagnostics, symbols, hover, definition, completion, references, rename, and semantic tokens

For the latest development status and priorities, see `docs/PriorityChecklist.md`.

## License

See the repository for license information.
