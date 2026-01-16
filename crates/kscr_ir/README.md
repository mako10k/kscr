# IR Optimizer

The `kscr_ir::optimize` module provides safe optimization passes for the kscr intermediate representation.

## Quick Start

```rust
use kscr::ir::{lower_to_ir, optimize_ir, run_main};

// Parse and typecheck your program
let ast = kscr::parser::parse_module(src)?;
let typed = kscr::types::typecheck(ast)?;

// Lower to IR
let ir = lower_to_ir(&typed.module)?;

// Apply optimizations
let optimized_ir = optimize_ir(&ir);

// Execute
let result = run_main(&optimized_ir)?;
```

## Available Optimization Passes

### Constant Folding

Evaluates constant expressions at compile time.

**Examples:**
- `if True then 42 else 0` → `42`
- `(\x -> x) 42` → `42` (beta reduction for values)

### Dead Code Elimination

Removes unused bindings via reachability analysis from `main`.

**Algorithm:**
1. Mark `main` as live
2. Compute transitive closure of live bindings
3. Remove all non-live bindings

### Case Simplification

Simplifies trivial case expressions.

**Examples:**
- `case x of _ -> e` → `e`
- `case x of v -> body` → `let v = x in body`

## Custom Optimization Pipelines

You can build custom optimization pipelines:

```rust
use kscr_ir::optimize::{
    run_passes, ConstantFolding, CaseSimplification, 
    DeadCodeElimination, OptimizationPass
};

let passes: Vec<Box<dyn OptimizationPass>> = vec![
    Box::new(ConstantFolding),
    Box::new(CaseSimplification),
    Box::new(DeadCodeElimination),
];

let optimized = run_passes(&ir, &passes);
```

## Safety Guarantees

All optimizations preserve:

1. **Correctness**: Optimized programs produce the same observable results
2. **Lazy Semantics**: Thunks and sharing are respected
3. **Effects**: IO actions and exceptions are properly sequenced

## Testing

Each optimization pass has comprehensive tests:

```bash
# Test optimizer module
cargo test --package kscr_ir optimize

# Test integration
cargo test ir_optimize
```

## Documentation

- [`IROptimization.md`](../../docs/IROptimization.md) - Detailed optimization framework documentation
- [`IntermediateRepresentation.md`](../../docs/IntermediateRepresentation.md) - IR specification

## Performance

Optimization adds compile-time overhead but can significantly reduce:
- Runtime memory usage
- Execution time
- IR module size

Use selectively for development (fast compile) vs. production (fast runtime).
