# Implementation Status and Roadmap

This document is the reviewed maturity baseline for `kscr`. It separates implemented capability, supported scope, experimental paths, known blockers, and future work.

Last reviewed: 2026-09-08
Evidence revision: `21207dfdad075ae9f862e6140507f4f4a7ef34df`

## Evidence Rules

- Code and focused tests establish observable behavior.
- CI establishes the automated gates passed by one revision.
- Current semantic documentation defines intended supported behavior when it agrees with accepted requirements and implementation evidence.
- GitHub Issues own live defect details and acceptance criteria.
- A passing test or CI run does not establish global language, release, or self-hosting completeness.
- When this document and current evidence disagree, investigate which source is defective and update the owning source. Do not silently redefine behavior to match the document.

## Maturity Vocabulary

| Status | Meaning |
|---|---|
| Supported baseline | Implemented and exercised by repository tests for the stated scope. |
| Partial | A useful implemented subset exists, with known missing or incorrect behavior. |
| Experimental | Implemented for evaluation, but its contract or compatibility is not yet stable. |
| Blocked | A known defect prevents an accepted workflow or completion claim. |
| Proposed | Design material exists, but implementation acceptance has not been established. |

These labels apply to a stated capability, not to the repository as a single percentage.

## Current Capability Matrix

| Capability | Status | Evidence and boundary |
|---|---|---|
| Lexer, layout, and parser | Supported baseline | `src/lexer.rs`, `src/parser_impl.rs`, parser tests, and `docs/LanguageBNF.md`. A syntax change still requires a complete lexer-to-runtime vertical slice. |
| Hindley-Milner inference and core types | Supported baseline | `src/types.rs` and typechecking regressions. |
| Typeclasses and deriving | Partial | User-defined `class`/`instance`, dictionaries, and deriving are implemented. Direct `Eq String`/`Eq [Char]` remains open in [#100](https://github.com/mako10k/kscr/issues/100). |
| Modules and imports | Partial | Typechecking supports qualified/unqualified imports and filters. Runtime alias forwarding does not preserve each declaration's filter; see [#105](https://github.com/mako10k/kscr/issues/105). |
| Lazy IR interpreter and IO | Partial | The current evaluator supports thunks, pattern matching, exceptions, and IO. Tail-recursive source calls still consume native stack frames; see [#107](https://github.com/mako10k/kscr/issues/107). |
| REPL | Supported baseline | `kscr repl`, module loading, type inspection, and optional readline are implemented for the interpreter path. |
| KIR1 and KSIF | Experimental | Encoding/decoding and interface artifacts exist in `src/kir1.rs`. Artifact behavior under `KSCR_USE_KSIF=0` is unresolved in [#101](https://github.com/mako10k/kscr/issues/101), and deriving metadata export still has implementation TODOs. |
| Default native compile path | Experimental | `kscr compile` embeds KIR1 in a Rust runner that executes through the current runtime. This is not an independent self-hosted backend. |
| Checked and unsafe FFI | Partial | Checked numeric boundary helpers and opt-in unsafe subcrates exist; broad ABI, ownership, and encoding support is future work. |
| LLVM backend | Experimental | Optional lowering and CLI paths exist for a subset. Interpreter parity and full lowering are not established. |
| LSP and VS Code extension | Partial | Diagnostics, navigation, completion, rename, and semantic-token baselines exist. Workspace-wide indexing, provisioning, and formatter integration remain incomplete. |
| Release workflow | Supported pipeline | CI and the tagged release workflow build the documented binaries. A successful workflow does not by itself authorize or establish a new release. |
| Self-hosting readiness | Blocked | [#107](https://github.com/mako10k/kscr/issues/107) prevents ordinary-stack execution of source-scanning stage1 workloads without a workaround. Artifact isolation in [#101](https://github.com/mako10k/kscr/issues/101) also remains part of reliable harness behavior. |

## Current Self-Hosting Priority

This ordering is for self-hosting impact; it does not assign GitHub labels or close Issues.

1. Remove the evaluator's native-stack dependency for saturated tail calls ([#107](https://github.com/mako10k/kscr/issues/107)).
2. Make imported runtime aliases obey the same per-declaration filters as type inference ([#105](https://github.com/mako10k/kscr/issues/105)).
3. Define and enforce isolated KIR1/KSIF artifact behavior for cache-disabled and concurrent runs ([#101](https://github.com/mako10k/kscr/issues/101)).
4. Restore direct equality for `String` and `[Char]` ([#100](https://github.com/mako10k/kscr/issues/100)).
5. Expand optional LLVM, FFI, and editor tooling only against explicit user value and acceptance criteria.

Items 2-4 can be reordered when a downstream acceptance target supplies stronger evidence. Do not stretch one fix to absorb another Issue.

## Completion Contracts

### Supported behavior

A supported capability is complete for its stated scope only when:

- the owning implementation satisfies the accepted semantics;
- focused positive and negative regressions cover the behavior;
- relevant repository gates pass;
- user-facing and semantic documentation agree with the implementation; and
- no known open defect contradicts that supported claim.

### Defect resolution

Closing a defect requires both:

- evidence that the source mechanism was corrected; and
- a regression or other check that detects recurrence.

A new test alone addresses the escape cause, not the implementation root cause.

### Self-hosting readiness

The current Rust implementation remains stage0 infrastructure for downstream self-hosting work. A self-hosting-ready claim requires, at minimum:

- accepted stage1 programs run on the normal process stack without source-level chunking or stack-size overrides;
- module/typechecking/runtime namespaces agree for accepted import forms;
- concurrent or isolated runs do not share undocumented artifact state;
- the accepted language slice has reproducible source, tests, and diagnostics; and
- remaining stage0 dependencies are named rather than hidden behind the completion claim.

### Release readiness

Release readiness is revision- and artifact-specific. It requires passing relevant gates, verifying the packaged artifact set and version alignment, checking documented compatibility, and separately authorizing publication. It is not implied by this roadmap.

## Updating This Document

Update this matrix when a capability crosses a maturity boundary. Link the implementation, tests, and Issue or decision that justify the change. Keep detailed execution history in commits, pull requests, and Issues rather than adding completed-task narratives here.
