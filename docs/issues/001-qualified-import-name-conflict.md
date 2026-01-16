# Issue: False `name conflict` for qualified imports

## Summary
Qualified imports should allow the same exported name from multiple modules without conflicts (e.g. `P.filter` vs `L.filter`).

Currently, kscr rejects some programs with an error like:

```
error: name conflict: filter (previously from import P, now from import L); try `import ... as ...` or qualify
```

This blocks stdlib usage patterns such as `import Prelude as P` + `import Data.List as L`.

## Minimal reproduction
File: `tests/import_qualified_conflict_minrepro.ks`

```kscr
module Main where
  import Prelude as P
  import Data.List as L

  main :: IO Unit
  main = P.IO ()
```

Run:

```bash
cargo run -- typecheck tests/import_qualified_conflict_minrepro.ks
```

## Expected
Typecheck succeeds.

- `P.<name>` and `L.<name>` are distinct names.
- No unqualified names are imported here.

## Actual
Typecheck fails with `name conflict`.

After adjusting import-forwarder emission to avoid leaking unqualified names from aliased imports, the original `name conflict` is resolved, but a new error appears:

```
error: in binding __inst_Functor_P_Maybe_fmap: in case arm 1: in case arm 1: unknown constructor
```

This suggests an additional bug around alias-qualified imports of data constructors (e.g. `P.Just`/`P.Nothing`) or how imported instance dictionaries/forwarders refer to constructors.

## Suspected subsystem
Typechecker / import lowering / name conflict checking.

Relevant code:
- `src/types.rs`:
  - `collect_imports(...)` emits imported items into the module item list.
  - `push_item_checked(...)` reports `name conflict: {n}` when the same `n` is defined twice.

Observation (from `KSCR_DEBUG_IMPORTS=1`):
- Qualified import lowering creates alias-prefixed bindings (e.g. `L.null`, `L.filter`, ...).
- Despite this, conflicts are reported for unqualified names (`filter`, `Just`, `map`, ...), suggesting:
  - some imported items are emitted without the qualifier prefix, or
  - conflict detection is applied to a pre-desugared name set.

## Proposed fix direction
Make name-conflict checking aware of qualified imports:

- When imported bindings are emitted for a qualifier `Q`, their defined names must be `Q.<name>` only.
- Unqualified forwarders should only be emitted when the import is truly unqualified.
- `push_item_checked` should only treat identical fully-qualified names as the same name.

## Why this matters
Stdlib expansion requires predictable import semantics; qualified imports are the standard way to avoid collisions.
