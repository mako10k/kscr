# Repository Guidelines

## Scope and Instruction Sources

This file is the repository-wide contract for human and AI contributors. It applies to the whole repository unless a more specific `AGENTS.md` exists below the working directory. Higher-priority system, organization, and user instructions still take precedence.

`.github/copilot-instructions.md` is a thin GitHub Copilot adapter. Files in `.github/agents/` are optional specialist profiles, not mandatory workflow gates. Keep durable repository rules here instead of duplicating them across agent profiles.

Repository-facing documentation, code comments, identifiers, commit messages, issue text, and pull request text are English-first unless the user asks otherwise.

## Project Goal and Maturity

`kscr` is a lazy functional scripting language implemented in Rust. It includes a lexer, parser, Hindley-Milner type inference, typeclasses, module loading, IR lowering and evaluation, a REPL, an experimental KIR1/KSIF artifact path, optional LLVM support, and an LSP server.

Do not collapse these distinct claims:

- a command or test passes;
- the current interpreter baseline is supported;
- an optional or experimental subsystem works for its documented subset;
- a release is ready; and
- the language or self-hosting path is complete.

Use `docs/ImplementationPlan.md` for the reviewed maturity matrix and GitHub Issues for live work. A green CI run proves the tested revision, not global completeness.

## Repository Layout

- `src/`: main Rust implementation and CLI.
- `crates/`: isolated IR, LSP, LLVM, and unsafe optional components.
- `stdlib/`: kscr standard-library source.
- `tests/`: Rust integration tests and `.ks` fixtures.
- `docs/`: current specifications, proposals, historical records, and status documentation.
- `editors/`: editor integrations.
- `.github/workflows/`: CI and release definitions.

## Sources of Truth

Use this order when sources disagree:

1. Accepted user requirements and language compatibility decisions.
2. Observable implementation behavior and focused tests.
3. CI and release workflows for automated gates and shipped artifacts.
4. Current semantic/reference documentation.
5. `docs/ImplementationPlan.md` for maturity classification.
6. GitHub Issues for active work and acceptance criteria.

Documentation does not silently redefine behavior. When code and a current specification conflict, determine whether the implementation or the document is defective, then fix the owning source and add evidence. Keep proposals clearly marked as proposals.

## Working Method

For non-trivial work:

1. Inspect the repository, relevant requirements, tests, and current Git state.
2. State the requested outcome, accepted completion contract, and material unknowns.
3. Choose the smallest coherent change at the component that owns the cause.
4. Validate the narrow behavior first, then broaden checks only when risk or evidence justifies it.
5. Review the focused diff for regressions, ownership mistakes, and missing tests.
6. Report what is observed, inferred, changed, tested, and still unknown.

Ask a question only when a missing decision would materially change the result or authorize a new side effect. Do not require a separate planning, gatekeeper, or delegation pass for routine bounded work. Use specialist agents only when their independent contribution is useful; delegation does not expand scope or authority.

## Scope and Change Authority

Planning, explanation, diagnosis, and review requests are read-only unless the user also requests implementation or another write. Implementation authority does not automatically include commits, pushes, issue or pull-request changes, releases, publication, dependency installation, rollback, or cleanup.

- Preserve unrelated and pre-existing worktree changes.
- Never stage with `git add -A` when unrelated changes may exist; stage explicit paths only.
- Do not create a branch or WIP commit merely because a change crosses a file or line-count threshold.
- Do not run destructive or history-changing Git operations without explicit authorization for the exact operation and target.
- Do not discard, reset, clean, or overwrite work as an automatic recovery step.
- Treat rollback, retry, commit, push, issue mutation, PR mutation, release, and publication as separate actions with separate authority.

## Implementation Ownership and Semantics

Choose the fix location by causal ownership:

- Fix lexer, parser, typechecker, IR, runtime, module-linking, or CLI defects in the Rust subsystem that owns them.
- Change `stdlib/` when the standard-library definition or API is the cause.
- Do not hide engine defects behind stdlib workarounds, test-only branches, symbol-specific exceptions, or broad fallbacks.
- A simpler implementation is welcome only when it satisfies the accepted behavior; disclose and obtain approval for a material substitute.

Language changes must preserve the full vertical contract: tokenization/layout, parsing and AST, typechecking, lowering/runtime behavior, diagnostics, fixtures, and current reference documentation as applicable. A parser edit alone is not evidence that a language feature is complete.

For import and module changes, check qualified and unqualified paths, `Only` and `Hiding` filters, constructors, aliases, transitive imports, and both typechecking and runtime linking when relevant.

## Validation

Run the narrowest checks that prove the touched behavior. Do not rerun tests a fixed number of times without a flakiness hypothesis or new information value.

Typical focused commands:

```bash
cargo test <test-name> -- --test-threads=1
cargo test --test <integration-test> -- --test-threads=1
```

Full repository gates, matching CI intent:

```bash
cargo test -- --test-threads=1
cargo fmt -- --check
cargo clippy --all-features -- -D warnings
```

Use feature-specific checks for optional crates and backends. If external tools such as `clang`, Node packages, or an LSP client are absent, report the unverified boundary; do not install or download them unless that action is in scope. In particular, do not make `npx -y` a mandatory read or navigation step.

For documentation-only changes, verify links, commands, status language, and agreement with the cited code/tests; broad runtime tests are unnecessary unless the documentation changes an executable contract.

## Review and Root-Cause Analysis

Review findings should lead with bugs, regressions, compatibility risks, ownership errors, and missing evidence. Ground findings in file and line references.

For a defect, separate:

- root cause: the mechanism or decision that created the defect;
- contributing causes: conditions that enabled or amplified it;
- escape cause: why tests, review, or monitoring did not detect it; and
- corrective and recurrence-prevention work.

A missing test is not the root cause of an implementation defect. A new test improves detection but does not prove the source mechanism is fixed. If the root cause is not established, report it as unknown and continue bounded investigation.

Before a requested non-WIP commit, review `git diff` and `git diff --cached`, verify the intended file set, and run checks proportional to the change. Do not commit unless the user requested a commit.

## Artifacts, Releases, and GitHub

Before changing packaging, release workflows, install paths, archive layouts, or binary names, enumerate the current shipped artifacts and determine whether the accepted contract preserves or changes them. Removing, renaming, or moving a shipped artifact requires explicit approval.

Local planning never authorizes GitHub writes. Read-only issue and PR inspection is allowed when relevant; creating or editing an Issue or PR, pushing, tagging, or releasing requires an explicit request. After an authorized external write, read back the public state instead of inferring success from local state.

## Documentation and Status Maintenance

- `README.md`: user-facing capabilities and concise known limitations.
- `docs/ImplementationPlan.md`: reviewed capability status and roadmap ordering.
- `docs/PriorityChecklist.md`: concise view of currently accepted priorities, linked to Issues.
- `docs/DOC_INDEX.md`: classification of current, future, and historical documents.
- GitHub Issues: live task requirements and acceptance criteria.

Avoid creating another status ledger when one of these sources can be updated. Date-stamped audits and handoffs are historical evidence, not current authority. When maturity changes, update the implementation evidence and the smallest affected status surface in the same change.
