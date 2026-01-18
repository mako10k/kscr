# REPL / Typechecker Performance

## Goal

Make `kscr repl` practical: ideally **< 1s per input** on a warm process.

## Status (2026-01-18)

### Completed: avoid `apply_env` cloning in inference

The previous bottleneck was repeatedly applying substitutions to the entire `TypeEnv` (`apply_env`), which cloned/applied every scheme in the environment in hot loops.

Implementation: thread `&Subst` through expression inference and apply substitution lazily at lookup/instantiate points.

Commit: `perf(types): avoid apply_env cloning`.

## Current bottleneck hypothesis (next)

After the `apply_env` removal, the next likely bottleneck is **top-level generalization**:

- `ftv_env_applied(subst, env_global)` currently iterates over **all schemes** in `env_global` and computes free type variables after substitution.
- This happens once per SCC (and in REPL, the synthetic module changes per input, so this repeats).

This is effectively *O(#SCC × |env_global|)* and can dominate `infer_module_with_class_env` even for small user inputs when the environment is large (stdlib + imported forwarders).

## Plan: eliminate repeated full-env scans

### Idea A (recommended / low-risk): maintain incremental env FTV cache

Maintain an `env_global_ftv: HashSet<u32>` that tracks `ftv_env(&env_global)` incrementally.

When inserting a new generalized scheme `scheme` into `env_global`, update:

- `env_global_ftv.extend(ftv_scheme(&scheme))`

Then, when generalizing the next SCC under a current substitution `subst`, compute the "applied env ftv" without walking all schemes:

- Start from `env_global_ftv` (vars seen in env).
- For each `v` in `env_global_ftv`, account for substitution if `subst` maps `v`.

This avoids cloning/applying schemes across the whole environment.

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
