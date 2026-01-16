# IR Optimization Framework

This document describes the IR optimization framework for kscr.

## Design Principles

### Safety First

All optimizations must preserve program semantics:

1. **Correctness**: Optimized programs produce the same observable results as unoptimized programs
2. **Lazy Semantics**: Optimizations respect lazy evaluation semantics (thunks, sharing)
3. **Effects**: IO actions and exceptions must be sequenced correctly
4. **No Speculation**: Do not evaluate expressions that might not be needed

### Optimization Pass Architecture

The optimization framework is based on composable passes:

```rust
pub trait OptimizationPass {
    fn optimize_module(&self, module: &IrModule) -> IrModule;
    fn name(&self) -> &'static str;
}
```

Each pass:
- Takes an `IrModule` and returns an optimized `IrModule`
- Is independent and can be composed with other passes
- Has a name for debugging and profiling

## Implemented Optimizations

### 1. Constant Folding

**Purpose**: Evaluate constant expressions at compile time.

**Safety**: Only folds pure, ground terms (literals). No side effects.

**Examples**:
- `if True then 42 else 0` → `42`
- `(\x -> x) 42` → `42` (beta reduction for values)

**Limitations**:
- Does not fold arithmetic operations (requires value-level evaluation)
- Only beta-reduces when all arguments are values (literals)

### 2. Dead Code Elimination

**Purpose**: Remove unused bindings to reduce module size.

**Safety**: Uses reachability analysis from `main` to determine live bindings.

**Algorithm**:
1. Mark `main` as live
2. Compute transitive closure: if a binding is live, mark all free variables in its RHS as live
3. Remove all bindings not marked as live

**Limitations**:
- Conservative: keeps all bindings reachable from `main`
- Does not remove dead branches within expressions

### 3. Case Simplification

**Purpose**: Simplify trivial case expressions.

**Safety**: Preserves pattern matching semantics.

**Examples**:
- `case x of _ -> e` → `e`
- `case x of v -> body` → `let v = x in body`

**Limitations**:
- Only simplifies single-arm cases with trivial patterns
- Does not reorder or merge case arms

## Usage

### Running Individual Passes

```rust
use kscr_ir::optimize::{ConstantFolding, OptimizationPass};

let module = /* ... */;
let pass = ConstantFolding;
let optimized = pass.optimize_module(&module);
```

### Running Multiple Passes

```rust
use kscr_ir::optimize::{run_passes, ConstantFolding, DeadCodeElimination, CaseSimplification};

let passes: Vec<Box<dyn OptimizationPass>> = vec![
    Box::new(ConstantFolding),
    Box::new(CaseSimplification),
    Box::new(DeadCodeElimination),
];

let optimized = run_passes(&module, &passes);
```

## Future Optimizations

### Planned

1. **Inlining**: Inline small, non-recursive functions
2. **Common Subexpression Elimination**: Share computation of identical subexpressions
3. **Deforestation**: Eliminate intermediate lists via fusion
4. **Strictness Analysis**: Insert eager evaluation where safe

### Not Planned (Unsafe)

1. **Speculative Evaluation**: May change termination behavior
2. **Reordering Effects**: May change observable IO behavior
3. **Unsafe Inlining**: Inlining recursive or large functions may cause code explosion

## Testing Strategy

Each optimization pass has:

1. **Unit Tests**: Test individual transformations
2. **Roundtrip Tests**: Verify optimized code produces same results
3. **Property Tests**: Check optimization invariants

Example test structure:

```rust
#[test]
fn test_optimization_preserves_semantics() {
    let module = /* original module */;
    let optimized = pass.optimize_module(&module);
    
    // Both should produce the same result
    assert_eq!(
        run_main(&module),
        run_main(&optimized)
    );
}
```

## Integration

The optimizer is integrated into the compilation pipeline:

```
Parser → Typechecker → IR Lowering → **Optimizer** → Runtime/LLVM
```

To enable optimizations, use the `optimize` parameter:

```rust
let ir = lower_to_ir(&module)?;
let optimized_ir = if optimize {
    run_passes(&ir, &default_passes())
} else {
    ir
};
```

## Performance Considerations

- **Compilation Time**: Optimization passes add compile-time overhead
- **Runtime Benefit**: Optimizations reduce runtime overhead and memory usage
- **Tradeoff**: Use selective optimization for development (fast compile) vs. production (fast runtime)

## Debugging

Each pass has a name for debugging:

```rust
for pass in &passes {
    eprintln!("Running pass: {}", pass.name());
    let before = module.clone();
    let after = pass.optimize_module(&before);
    // Compare before/after
}
```

Set `KSCR_OPTIMIZE_DEBUG=1` to enable verbose optimization logging.
