# LLVM IR Generation

This document describes the LLVM IR generation feature in kscr.

## Overview

The `kscr_llvm` crate provides conversion from kscr's intermediate representation (IR) to LLVM IR text format. This enables:

1. Generation of native code through LLVM toolchain
2. Integration with LLVM-based JIT compilers
3. Optimization passes via LLVM
4. Native binary generation

## Architecture

The LLVM IR generation is implemented as an optional crate (`crates/kscr_llvm`) that is enabled via the `llvm` feature flag. This follows the pattern established for other optional features like `unsafe_ffi` and `unsafe_bigint`.

### Design Principles

1. **Text-based generation**: The MVP generates LLVM IR as text, which can be compiled with `llc` or `clang`. This avoids the complexity of LLVM library dependencies.

2. **Isolated implementation**: All LLVM-related code is in a separate crate, keeping the main kscr crate clean.

3. **Feature-gated**: The feature is disabled by default and must be explicitly enabled with `--features llvm`.

## Generated LLVM IR Structure

### Runtime Types

The generated LLVM IR includes definitions for runtime data structures:

```llvm
; Thunk structure for lazy evaluation
%struct.kscr_thunk = type { i8*, i8*, i8*, i32 }

; Value structure (tagged union)
%struct.kscr_value = type { i32, i8* }
```

### Runtime Functions

The generated code declares runtime support functions:

- `@kscr_force_thunk(%struct.kscr_thunk*)` - Force evaluation of a thunk
- `@kscr_execute_io(%struct.kscr_value*)` - Execute an IO action
- `@malloc(i64)` - Memory allocation
- `@puts(i8*)` - String output (C stdlib)

## Usage

### Command Line

Generate LLVM IR from a kscr source file:

```bash
cargo run --features llvm -- llvm-ir path/to/file.ks
```

Save to file and compile:

```bash
# Generate LLVM IR
cargo run --features llvm -- llvm-ir program.ks > program.ll

# Compile to object file (requires LLVM installed)
llc -filetype=obj program.ll -o program.o

# Link to executable (requires runtime implementation)
clang program.o -o program
```

### Programmatic API

```rust
use kscr_llvm::LLVMIRGenerator;

let mut gen = LLVMIRGenerator::new("my_module");
gen.generate_placeholder_main();
let llvm_ir = gen.to_string();
println!("{}", llvm_ir);
```

## Current Implementation Status

### Implemented (MVP)

- ✅ Basic LLVM IR text generation
- ✅ Runtime type definitions (thunks, values)
- ✅ Runtime function declarations
- ✅ Placeholder main function generation
- ✅ Integer arithmetic functions
- ✅ CLI integration (`llvm-ir` command)

### Planned (Future Work)

The current implementation is an MVP that demonstrates the architecture. Full implementation will include:

- [ ] Lower kscr IR expressions to LLVM IR
- [ ] Implement thunk allocation and forcing
- [ ] Implement closure representation
- [ ] Lower pattern matching to LLVM switch/branch
- [ ] IO action compilation
- [ ] String and list operations
- [ ] FFI boundary code generation
- [ ] Garbage collection integration
- [ ] Optimization passes

## Integration with kscr Pipeline

The LLVM IR generation fits into the kscr compilation pipeline:

```
Source Code
    ↓
Lexer → Tokens
    ↓
Parser → AST
    ↓
Typechecker → Typed AST
    ↓
IR Lowering → kscr IR
    ↓
LLVM Codegen → LLVM IR
    ↓
LLVM Toolchain → Native Binary
```

## Lazy Evaluation in LLVM

Thunks are represented as heap-allocated structures:

```c
struct kscr_thunk {
    void (*code)(void*);  // Function to evaluate
    void* env;            // Captured environment
    void* result;         // Memoized result
    int32_t state;        // 0=unevaluated, 1=evaluating, 2=evaluated
};
```

When a thunk is forced:
1. Check state: if evaluated, return memoized result
2. Set state to evaluating (detect cycles)
3. Call code function with environment
4. Store result and set state to evaluated
5. Return result

## References

- See `docs/ToolchainDesign.md` for overall toolchain architecture
- See `docs/IntermediateRepresentation.md` for IR semantics
- See `docs/ImplementationPlan.md` Milestone 7 for LLVM backend roadmap
