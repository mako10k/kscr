# REPL / Typechecker Performance

## Goal

Make `kscr repl` practical: ideally **< 1s per input** on a warm process.

## Status (2026-01-18)

### ✓ Completed: avoid `apply_env` cloning in inference

The previous bottleneck was repeatedly applying substitutions to the entire `TypeEnv` (`apply_env`), which cloned/applied every scheme in the environment in hot loops.

Implementation: thread `&Subst` through expression inference and apply substitution lazily at lookup/instantiate points.

Commit: `perf(types): avoid apply_env cloning`.

### ✓ Completed: cache env FTV for generalization

Maintained `env_global_ftv: HashSet<u32>` tracking free type variables in the global environment and computed "applied env ftv" without walking all schemes.

Commit: `perf(types): cache env ftv for generalization`.

**Measured (warm process):** `typecheck_internal` ~0.4–0.6s per REPL input.

## Next bottleneck hypothesis (only if we need further wins)

### Idea B (secondary): reduce `apply_constraints` churn in infer_expr

Many loops do:

- `cs = apply_constraints(&s, cs)` repeatedly

This can be batched (apply once at the end of a loop) where correctness allows, reducing allocations.

### Idea B (secondary): reduce `apply_constraints` churn in infer_expr

Many loops do:

- `cs = apply_constraints(&s, cs)` repeatedly

This can be batched (apply once at the end of a loop) where correctness allows, reducing allocations.

## Execution order

1) Implement Idea A (env FTV cache) with minimal surface changes.
2) Re-measure `KSCR_DEBUG_TIMING=1 cargo run --release -- repl`.
3) If still >1s: implement Idea B in targeted expression inference loops.

## Validation

- `cargo test`
- `cargo clippy -- -D warnings`
- Timing check:

```bash
KSCR_DEBUG_TIMING=1 cargo run --release -- repl
```
