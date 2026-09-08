# Current Priority Checklist

This is the concise execution view for the current self-hosting objective. `docs/ImplementationPlan.md` owns maturity definitions; GitHub Issues own live requirements and acceptance criteria.

Last reviewed: 2026-09-08

## P0 — Remove the Self-Hosting Runtime Blocker

- [ ] [#107 Make tail-recursive programs stack-safe in the IR evaluator](https://github.com/mako10k/kscr/issues/107)
  - Correct the recursive evaluator mechanism with an explicit iterative or trampoline path.
  - Preserve laziness, dictionary auto-application, pattern matching, and IO behavior.
  - Pass a source-level regression above the current normal-stack threshold.
  - Re-run downstream self-hosting regressions without stack-size or source-chunking workarounds.

## P1 — Align Supported Semantics and Artifacts

These items are independently scoped. Order them by the next accepted downstream target.

- [ ] [#105 Preserve per-declaration import filters in runtime alias forwarding](https://github.com/mako10k/kscr/issues/105)
  - Make typechecking and linked IR expose the same alias namespace.
  - Cover `Only`, `Hiding`, constructors, aliases, transitive imports, and REPL linking where applicable.
- [ ] [#101 Clarify or enforce KSCR_USE_KSIF=0 artifact behavior](https://github.com/mako10k/kscr/issues/101)
  - Define whether the flag disables all KIR1/KSIF reads and writes or only cache reuse.
  - Provide a supported isolation contract for concurrent harness runs.
- [ ] [#100 Support direct Eq String and Eq [Char]](https://github.com/mako10k/kscr/issues/100)
  - Correct the owning instance or alias-resolution mechanism.
  - Keep existing `Char` and list equality behavior green.

## P2 — Harden Partial and Experimental Surfaces

Create or select an Issue with an accepted completion contract before implementation.

- [ ] Establish interpreter-parity slices for the optional LLVM backend.
- [ ] Expand FFI only with explicit ABI, ownership, encoding, and safety boundaries.
- [ ] Add workspace-wide LSP indexing and improve server provisioning.
- [ ] Decide the supported KIR1/KSIF compatibility and separate-compilation boundary.

## Baseline Maintenance

For every completed item:

- [ ] Correct the source mechanism rather than only its detection gap.
- [ ] Add focused positive and negative regression evidence.
- [ ] Run validation proportional to the affected subsystem.
- [ ] Update `docs/ImplementationPlan.md` if maturity changes.
- [ ] Update current semantic or user documentation affected by the behavior.
- [ ] Leave commit, push, Issue mutation, PR mutation, and release actions to their separately authorized workflow.

Historical P-number inventories and completed snapshots remain in `docs/BACKLOG.md` and repository history. They are not active authority.
