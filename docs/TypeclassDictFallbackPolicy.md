# Typeclass dictionary handling policy

## Goal

We allow *ambiguity* (insufficient information) to flow through the pipeline.
We avoid ad-hoc fallback and method-name special-casing.
Current implementation may still commit to a concrete dictionary when it can derive one from inferred application type or enclosing binding return type.

This is especially important for methods like `return :: a -> m a` where the dictionary cannot be chosen from argument types alone.

## Definitions

- **Ambiguity**: the compiler cannot choose a unique dictionary (instance) given the currently known types.
  - This is OK to keep as constraints and resolve later (or report if still ambiguous at the end).
- **Ad-hoc/heuristic commitment (NG)**: choosing a specific dictionary based on partial information such as:
  - method name special-casing (e.g. branching on the string `"return"`),
  - selecting an instance from argument types only when the method's type does not determine the class parameter,
  - any other "best guess" rule.

## Allowed behavior

- Keep the call polymorphic by producing an expression that *references a dictionary parameter* (i.e. defer selection).
- Select a concrete dictionary when evidence is available from inferred application type (`!determined_by_args` path) or enclosing binding return type (`find_enclosing_binding` fallback).
- Report ambiguity only at a phase where no further information can arrive (or in an explicit debug/failfast mode).

## Disallowed behavior

- Any method-name special casing in type inference / dict resolution paths.
- Opaque guesses not backed by inferred type evidence.

## Diagnostics policy

- Default mode: do not hard-error on ambiguity during rewrite/typecheck of incomplete programs (needed for LSP and partial edits).
- FailFast mode (opt-in): surface ambiguity sites loudly and fail when a dictionary must be chosen but cannot.
  - Current knob: `KSCR_FAILFAST_METHOD_DICT=1`.

## Work items (TODO)

1. Remove any remaining method-name branching (e.g. `"return"`) from `src/types.rs`. ✅
2. Audit dictionary resolution paths for early commitment:
   - functions that return `Ok(Some(...))` / `Ok(None)` etc,
   - document each `Ok(Some(...))` path as either evidence-based selection or explicit policy choice.
3. Make ambiguity representation explicit:
   - tag deferred dictionary sites with structured metadata for diagnostics (method name, location, required class, partial types).
4. Fix remaining letrec/SCC recursion mismatches:
   - ensure dictionaries available from expected types (annotations / surrounding context) are visible during rewrite,
   - for methods whose class parameter is not determined by argument types (e.g. `return`/`pure`), current code first tries inferred application type and then enclosing binding return type.
5. Add regression tests:
   - `tests/repro_return_in_letrec_fail.ks` typechecks in current CLI (`kscr typecheck`). Add automated regression coverage. TODO
   - keep a test that verifies incomplete code used by LSP can still produce completions in default mode.
