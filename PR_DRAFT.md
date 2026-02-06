# Title
Typeclass dict-passing rewrite: make `Eq`/`Show` regular typeclasses + stabilize KSIF and constraint solving

# Summary
This PR finishes stabilizing the typeclass dict-passing rewrite by making `Eq`/`Show` behave like normal user-definable typeclasses (including `deriving (Eq, Show)`), while fixing regressions/flakiness around KSIF caching and local constraint solving.

Key outcomes:
- `deriving (Eq, Show)` works without special-casing `Eq`/`Show` as builtins.
- Local `let`/`where` generalization solves constraints against the full `ClassEnv` (fixes `Prelude.Eq Integer` failures).
- KSIF staleness from concurrent test runs is handled via a single best-effort auto-rebuild retry.

# Motivation
- `Eq`/`Show` should be regular typeclasses: users can define/override instances the same way as other classes.
- CI should not fail due to KSIF cache races in parallel test runs.
- Constraint solving must use the same instance environment in local generalization as at top-level.

# Changes
## Typechecker / typeclass system
- `InferCtx` now carries `full_class_env: Arc<ClassEnv>` and local generalization uses it when simplifying constraints.
- Improve error message for ambiguous method names (same method defined in multiple classes).
- Deriving expansion references `Prelude.Eq` / `Prelude.Show` explicitly so they behave as ordinary classes.

Files:
- src/types.rs

## Prelude ctor injection respects import specs
- Qualified Prelude ctors are always available.
- Unqualified ctors are injected only when the Prelude import actually brings them into scope (respects `only`/`hiding`).
- Anonymous/REPL-style modules keep unqualified ctors as a convenience.

Files:
- src/types.rs

## KSIF staleness handling
- When a cached KSIF fails dependency-hash validation, the loader performs a single auto-rebuild attempt and retries.
- If it still fails after the retry, it reports a stale KSIF error (no infinite retries).

Files:
- src/types.rs

## Dict-passing rewrite
- Avoid re-rewriting injected import-forwarders (e.g. `print = Prelude.print`) to prevent double-dict application.
- Improve `needs_dicts` lookup to consider shadowing and unqualified-to-qualified mapping.
- Pass an expected root type where appropriate to keep rewrite/type alignment sane.

Files:
- src/types/typeclass_dict_passing_common.rs
- src/types/typeclass_dict_passing_rewrite.rs

## Runtime value boundary stabilization
- Force/auto-apply dict-lambdas more robustly at value boundaries (bounded loop).
- Add a heuristic to identify likely typeclass dictionary records while avoiding ctor-encoded records.

Files:
- src/ir.rs

## CLI compile stability (parallel builds)
- Copy the selected `libkscr.rlib` into a temp directory before invoking `rustc --extern` to avoid races when `cargo build` runs concurrently.

Files:
- src/cli/cli_compile.rs

# Tests
Updated tests to match current KSIF artifact placement (`target/ksif`) and to accept multiple valid lowering shapes:
- tests/cli_compile_incremental_ksif.rs
- tests/ksif_hash_rebuild.rs
- tests/export_restriction_type.ks
- src/lib_test.rs
- crates/kscr_lsp/src/backend.rs (test module organization)

# User-visible behavior changes
- KSIF staleness is handled with a single auto-rebuild retry instead of failing immediately.

# Risks / review focus
- KSIF auto-rebuild retry: ensure it only triggers on proven dependency-hash mismatch and does not hide persistent issues.
- Runtime dict auto-apply heuristic: confirm it does not misclassify common record values (ctor-encoded records are excluded).
- Prelude ctor injection: confirm `only`/`hiding` semantics match expectations.

# Test plan
- cargo fmt --check
- cargo clippy --workspace -- -D warnings
- cargo test --workspace

# Follow-ups (optional)
- Consider adding a dedicated regression test for the stale-KSIF auto-rebuild retry path.
