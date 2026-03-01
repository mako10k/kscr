# Typeclass Semantics (Current Behavior)

This document describes the current semantics for user-defined `class` / `instance` and migration guidance from deriving-centric assumptions.

## Scope

- Applies to current `main` behavior.
- Focuses on user-defined typeclasses, dictionary passing, and cross-module usage.
- For internal rewrite policy details, see `TypeclassDictFallbackPolicy.md`.

## Core model

- Typeclass constraints are represented as explicit dictionary parameters after rewrite.
- Methods are treated as ordinary values that may carry constraints.
- Dictionary resolution prefers:
  1. dictionaries already in scope,
  2. instance selection when enough type information exists,
  3. deferral (keep polymorphic) when resolution is ambiguous at that point.

## Module boundary behavior

- User instances imported through transitive module chains are available for typechecking.
- Runtime linking preserves dictionary bindings for transitive imports.
- Qualified and unqualified method-as-value forwarding works across boundaries, including nested chains.

Example shape that is supported:

```hs
-- A exports class + instance
-- B exports applyInc = inc
-- C exports useInc = BX.applyInc
-- D exports callInc = useInc
-- Main uses DX.callInc 1
```

Both typecheck and runtime paths are covered by regressions.

## Re-export behavior

- Re-exported method values remain dictionary-carrying values.
- Deep chains that mix qualified imports and re-exports are supported.
- CLI execution path (`kscr run`) includes regression coverage for this scenario.

## Coherence and restrictions (current)

- Overlap/duplicate checks are enforced by existing class/instance environment logic.
- Ambiguity is deferred when possible; failfast diagnostics are available via `KSCR_FAILFAST_METHOD_DICT=1`.
- Built-in `Eq` / `Show` behavior and deriving expansion remain active; user class handling is aligned with the same dictionary model.

## Migration notes (from deriving-centric usage)

If existing code relied mostly on deriving, these patterns are now safe and recommended:

1. Define user classes and pass methods as values (`f = methodName`) across modules.
2. Use qualified imports (`import qualified M as X`) in forwarding modules without manually threading dictionaries.
3. Keep exports explicit for forwarding symbols to avoid accidental visibility issues.

Recommended checks when migrating:

- Add one typecheck test for transitive import visibility.
- Add one runtime/CLI test for the same module chain.
- Prefer concrete callsites in smoke tests (e.g. `... 1`) to force dictionary resolution.

## Related docs

- `PriorityChecklist.md` (phase status)
- `TypeSystem.md` (type system background)
- `TypeClassMethodResolutionPlan.md` (historical plan)
- `TypeclassDictFallbackPolicy.md` (dict-resolution policy)
