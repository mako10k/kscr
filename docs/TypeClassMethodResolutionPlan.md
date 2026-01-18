# Type Class Method Resolution Without Ground Types

## Context / Current Behavior

Today, class methods like `mod` are *not* treated as ordinary polymorphic values that can keep a `Class a` constraint around until generalization.

Instead, `typecheck_internal()` runs a **method-call rewrite** (`rewrite_class_method_calls_in_module` in `src/types.rs`) that rewrites:

- `mod n d`

into something like:

- `(__recordGet <dict> "mod") <dict> n d`

where `<dict>` is chosen **immediately**.

### Current Failure Mode (REPL)

In the REPL, this program fails:

```ks
canDiv n d = (n `mod` d) /= 0
```

with:

```
cannot resolve method call `mod`: no ground argument type available
```

because the rewrite tries to pick the instance dictionary for `Integral` by looking at the *argument* types of `mod` (`n` and `d`).
At that point they are polymorphic (`a`), and the rewrite does **not** use expected/result type information from the surrounding context.

## Goal

Allow normal HM-style inference to infer and generalize constrained schemes, e.g.:

```ks
canDiv : Integral a => a -> a -> Bool
```

and make method calls work in the REPL and in modules without requiring the user to add a type annotation solely to make method resolution succeed.

## Non-Goals (for this change)

- No Haskell-style defaulting rules (e.g. `Num` defaulting to `Integer`).
- No overlapping/ambiguous instance resolution.
- No changes to stdlib semantics as a workaround.

## Why “ground type” is required today

The current pipeline needs to emit executable code where a method call is implemented via a **dictionary record lookup**.
That requires a concrete dictionary value at the call site.

The current rewrite only knows how to obtain that dictionary by:

1. Using an in-scope dictionary parameter `__dict_<Class>` (rare at this stage), or
2. Picking a concrete instance dictionary name by matching `ClassEnv.instances[(class, instance_head)]`.

But the only input used for (2) is the inferred type of one of the method-call arguments.
If those argument types contain type variables (are not ground), the rewrite can’t compute an instance head key and errors out.

This is a *pipeline/design limitation*, not a fundamental type theory limitation.

## Proposed Design: Delay Method Desugaring Until After Dictionary Passing

### High-level idea

1. During type inference: keep class constraints (already supported via `Scheme.constraints`).
2. First lower constraints into **explicit dictionary parameters** (dictionary passing pass).
3. Only after dictionaries exist in scope, desugar method uses into dictionary lookups.

In other words: method calls should primarily be satisfied by *passing dictionaries*, not by guessing instances from partial/ground argument types.

### Required pipeline change

Current (simplified):

1. infer schemes + constraints
2. **rewrite class method calls** (requires dict selection)
3. rewrite dictionary passing (add `__dict_` params, supply dict args)

Proposed:

1. infer schemes + constraints
2. **rewrite dictionary passing** (add dict params to constrained bindings; rewrite call sites)
3. **rewrite class method calls** (now `__dict_<Class>` exists in the body scope)

Rationale: once `canDiv` is rewritten to take a `__dict_Integral` parameter, the body has a dictionary in scope and `mod` can be rewritten to use it without needing any ground type.

### What changes in method rewriting

Method rewriting should follow these rules:

1. If a method is used as a value (e.g. `f = mod`), rewrite it to a dictionary-lambda when no dictionary is in scope:

   ```ks
   mod  ~~>  \__dict_Integral -> (__recordGet __dict_Integral "mod") __dict_Integral
   ```

   This keeps the program well-typed without committing to an instance.

2. If a dictionary parameter for the required class is in scope (`__dict_Integral`), use it directly (no instance selection):

   ```ks
   mod n d  ~~>  (__recordGet __dict_Integral "mod") __dict_Integral n d
   ```

3. Avoid selecting a concrete instance dictionary name based on argument ground types during this pass.
   Instance selection should be handled by dictionary passing + constraint solving:
   - top-level constrained bindings receive dict params
   - callers supply dicts based on their own constraints or concrete instance dictionaries

### Constraint solving remains the authority

Constraint solving already exists and can decide whether `Integral a` is satisfiable.
Ambiguity should be reported as a constraint error at generalization / export boundary, not as a “method call rewrite” error.

Examples:

- `x = mod` should infer: `Integral a => a -> a -> a`
- `x = mod 10` should infer: `Integral a => a -> a` (or concretize if literal typing forces `Integer`)

If a constraint remains ambiguous in a place where we must run the program (e.g. `main`), we should emit a type error:

- “ambiguous constraint: Integral a”

## Implementation Plan (Incremental)

### Step 0 — Add regression repro

Add a minimal REPL/module test that currently fails:

```ks
-- tests/typeclass_method_ambiguous_ok.ks
canDiv n d = (n `mod` d) /= 0
```

Expected: typechecks and `:info canDiv` shows a constrained scheme.

### Step 1 — Reorder passes

Move dictionary passing earlier than method-call rewriting inside `typecheck_internal()`.

- Before: `rewrite_class_method_calls_in_module()` ran before dict passing
- After: run dict passing first

This should make `__dict_<Class>` params available in most bodies.

### Step 2 — Make method rewrite never require ground types

In `resolve_method_dict_name(...)` (or an equivalent new helper), remove the “pick instance by inspecting argument ground types” fallback.

Instead:

- Prefer in-scope `__dict_<Class>`
- Else prefer derived dict from `dicts_in_scope` / `known_dicts_in_scope`
- Else produce a dictionary-lambda (method value) rather than error

This shifts ambiguity reporting to the typechecker/constraint solver.

### Step 3 — Ensure dicts are in scope where needed

Verify that dictionary passing adds dict params to constrained bindings *before* method rewrite traverses those bodies.

If needed, extend the dict passing rewrite to track dict-params in nested lambdas/lets consistently.

### Step 4 — REPL quality

REPL `:info` / `:type` should be allowed to show constrained schemes even when no concrete instance is chosen.

This implies:

- `Scheme` printing must include `Class a => ...` (already implemented)
- method rewrite must not crash on unground args

## Key Discussion Points / Open Questions

1. **Ambiguity policy:**
   - Where do we reject ambiguous constraints?
   - Likely: reject at `main`, and at module exports; allow locally when generalized.

2. **Method-as-value semantics:**
   - Returning a dict-lambda for bare `mod` is the most predictable.
   - Ensure it composes with partial application and higher-order use.

3. **Performance:**
   - Reordering passes should not add extra typechecking runs.
   - Ensure we do not re-infer entire expressions inside rewrites (avoid repeated `infer_in_module_with_class_env` calls).

4. **Compatibility with current instance table keying:**
   - Current instance selection uses `instance_head_key_ty_for_class`.
   - After this change, that mechanism should be used mainly when supplying *concrete* dictionaries (e.g. when the caller’s type is ground), not when rewriting method calls.

5. **Nested bindings:**
   - If local `let` binds introduce constrained functions, ensure dict params are introduced and in scope for method calls inside them.

## Alternatives Considered

### Alternative A: Use expected type context during method rewrite

We could try to choose an instance dictionary by also considering expected/result type information, not just argument types.
This is complicated and still fragile (higher-order use, partial application, etc.).

### Alternative B: Fully elaborate dictionaries during type inference

Elaborate to a typed core language that explicitly carries dictionaries as part of inference (like GHC).
This is more invasive than necessary for the current MVP.

## Summary

The “ground type required” error is caused by an early desugaring pass that tries to pick instance dictionaries before dictionary passing has introduced `__dict_<Class>` parameters and before constraints can be generalized.

Reordering the pipeline and making method rewriting dictionary-driven (never ground-driven) enables:

- correct constrained inference (`Integral a => ...`)
- better REPL UX (`:info mod`, `:info canDiv`)
- fewer ad-hoc instance selection hacks
