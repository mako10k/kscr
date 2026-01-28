# Copilot Instructions (kscr)

Goal: improve and expand `stdlib/` while keeping the Rust execution engine (lexer/parser/types/IR/runtime) correct.

## Language / Writing (MANDATORY)

- The codebase is English-first: write code comments, docs, commit messages, identifiers, and this instruction file in English.
- Keep wording short and concrete; prefer examples over prose.

## Default operation mode (MANDATORY)

- By default, route work through the **orchestrator agent** (あおい（司令）).
- The orchestrator should delegate to specialized agents for execution:
  - こうた（実装） for code changes + tests
  - りん（レビュー） for review (be strict about ad-hoc fixes)
  - はる（PR） for PR text
  - まなみ（要望） / さくら（計画） for requirements / plans
  - ゆい（保守） only for instructions/docs/agent prompt maintenance

Exceptions (allowed to skip orchestrator):
- Trivial one-file doc edit
- Single grep/view lookup
- Running an existing command to confirm a fact

## Git Safety (MANDATORY)

- Do not run destructive git commands without explicit user permission.
  - "Destructive" means hard to undo/recover locally (rewrites working tree/index/refs irreversibly).
  - Examples (NOT allowed without permission):
    - `git reset --hard ...`
    - `git clean -fd ...`
    - `git checkout -f ...`
    - `git rebase ...`, `git rebase -i ...`
    - `git commit --amend ...` (rewrites history)
    - `git push --force ...`
    - `git revert ...` (state-changing; keep requiring permission)
    - `git merge ...`, `git cherry-pick ...` (state-changing; keep requiring permission)
- "Recoverable" operations are allowed when needed (still prefer to keep changes minimal and visible).
  - Examples (allowed):
    - `git status`, `git diff`, `git log`, `git show`
    - `git reset --soft ...`, `git reset --mixed ...`
    - `git checkout -b ...`, `git switch -c ...` (new branch)

### Non-destructive workflow (MANDATORY)

The agent must avoid “panic edits” and preserve work by default.

- Never propose discarding local edits (e.g. `git restore …`) as a first response.
  - First, explain *why* rollback is being considered (scope, blast radius, failing tests).
  - Then propose a **work-preserving** option:
    - make a WIP commit, or
    - create a new branch and commit there.
- Do not run any rollback/discard action (`git restore`, `git reset`, etc.) without explicit user confirmation.
- When a change touches multiple subsystems, prefer: **save → isolate → minimize → verify**.

### Rollback decision gate (MANDATORY)

Rollback is a last resort. Before proposing rollback, the agent **MUST** complete evidence collection and stabilization attempts.

#### Evidence collection (required first)

1. **Re-run with isolation**: Run failing tests with `--test-threads=1` or `-- --test <single_test_name>` to rule out parallel interference.
2. **Check global state**: Verify whether tests modify `env::set_current_dir()`, `env::set_var()`, process-wide policy, or other shared state without proper guards.
3. **Check flakiness**: Re-run failing tests 3+ times to identify non-deterministic failures.
4. **Minimal reproduction**: Isolate to a single test or minimal `.ks` file that demonstrates the regression.
5. **Identify cause**: Compare behavior before/after the recent change; verify the change is the actual root cause.

#### Stabilization attempts (required second)

Before suggesting rollback, **attempt these fixes first**:

- Serialize tests: Use `#[serial]` (via `serial_test` crate) or `--test-threads=1` flag.
- Guard global state: Add `Drop` guards to restore `cwd`, `env`, or policies after tests.
- Fix test isolation: Ensure tests clean up side effects (files, env vars, mocks).
- Minimal targeted fix: If the root cause is a typo, logic error, or missing null check, fix it directly rather than rolling back.

#### Rollback is justified ONLY IF

- **(a)** Regression is clearly from the recent change (confirmed by git bisect or manual rollback test), **AND**
- **(b)** At least one minimal targeted fix has been implemented and tested but failed to resolve the regression, **OR** the root cause cannot be identified after completing all 5 evidence collection steps above.

#### Work-preserving fallback (default)

If rollback appears necessary, **do not discard work**. Instead:

1. Make a WIP commit: `git commit -am "WIP: preserve work before rollback investigation"`
2. Create a branch: `git switch -c fix/<issue-id>-preserve-work`
3. Propose rollback on main branch only after preserving the work.

#### Rollback decision checklist

Before proposing rollback, answer YES to all:

- [ ] Have I re-run tests with `--test-threads=1`?
- [ ] Have I checked for global state changes (cwd/env/policy)?
- [ ] Have I reproduced the failure in isolation (single test or minimal `.ks`)?
- [ ] Have I verified flakiness by re-running 3+ times?
- [ ] Have I implemented and tested at least one stabilization approach (serialize tests, add Drop guards, fix isolation)?
- [ ] Have I implemented and tested at least one minimal targeted fix?
- [ ] Is the regression clearly caused by the recent change (not pre-existing)?
- [ ] Has the minimal fix been tested and failed to resolve the regression, or did I fail to identify the root cause after completing all evidence steps?

**If any answer is NO, do not propose rollback.** Instead, report progress and ask for guidance.

### Change scope discipline (MANDATORY)

To avoid “ad-hoc / band-aid” behavior:

- Keep the diff proportional to the symptom. If one test fails, do not redesign imports/runtime in the same patch.
- If experimentation is necessary, keep it on a separate branch or a clearly-labeled WIP commit.
- Before making a broad change, state:
  - what will change,
  - what might break,
  - how it will be validated (which tests).

### Communication integrity (MANDATORY)

The agent must not rewrite history or minimize prior intent.

- If the agent previously proposed or attempted a risky action (e.g. rollback/discard), it must acknowledge that fact explicitly.
- If the agent misspoke, it must correct itself clearly ("I was wrong earlier; I previously said X").
- Distinguish **proposed** actions vs **executed** actions.
  - When relevant, include a short action log in the reply:
    - Proposed: `…`
    - Executed: `…`
    - Blocked by user gate: `…`

### Explicit approval prompts (MANDATORY)

For any rollback/discard operation proposal (`git restore`, `git reset`, etc.), the agent must ask a yes/no confirmation using wording like:
- "I am proposing `git restore …`. I will NOT run it unless you explicitly confirm. Proceed?"

## Packaging / Release Guardrails (MANDATORY)

### Shell-safe gh usage (MANDATORY)

- When running `gh issue create|edit`, never use backticks in `--body` (they execute in the shell).
- Always use `--body-file` or a single-quoted heredoc (e.g. `cat > /tmp/body.md <<'MD' ... MD`).

- Packaging/release changes MUST NOT remove, rename, or stop shipping existing artifacts without explicit user approval.
- If a task touches `.github/workflows/**`, release scripts, or archive layouts, you MUST enumerate the expected shipped artifacts (names + paths) before editing.
- If the shipped artifact set changes (add/remove/rename/move), you MUST ask for confirmation before implementing it.
- Prefer additive changes and keep filenames/layout stable when possible.

Confirmation questions (copy/paste):
- "This change would remove/rename/move shipped artifact(s): <artifact> (<old path> -> <new path or removed>). Is this intended?"
- "Release payload currently includes: <list>. Do you want to keep all of these shipped?"

## Stdlib Policy (IMPORTANT)

- Do not “fix” engine bugs via ad-hoc stdlib workarounds.
- If changing stdlib appears to make a failing program/test pass, first assume a Rust-side bug.
  - Create a minimal `.ks` reproduction.
  - Fix the Rust subsystem (Lexer/Parser/Typechecker/IR/Runtime).
  - Add regression coverage.
- If behavior/spec is unclear or docs disagree with reality, open a GitHub Issue first.
  - Include: minimal repro, expected behavior, actual behavior, suspected subsystem.

## No Test-Only Special-Casing (MANDATORY)

- Do not add conditional branches or special cases whose only purpose is to make tests pass.
- If a change "makes tests green" for unclear reasons, assume an engine bug first.
  - Add a minimal `.ks` reproduction.
  - Fix the Rust subsystem (Lexer/Parser/Typechecker/IR/Runtime).
  - Add regression coverage.

## Review delegation workflow (MANDATORY)

When the user requests “review of fix policy + fix results should be handled by agents”, follow this workflow:

1. **Orchestrator assigns reviewer agent(s)**
   - Default reviewer: りん（レビュー）.
   - If the change touches packaging/release/workflows, require a reviewer pass focused on shipped artifacts.

2. **Reviewer runs before declaring done**
   - Reviewer must produce: risk assessment + test recommendations.

3. **Final response must separate intent vs action**
   - Proposed actions
   - Executed actions
   - Blocked by user gate (if any)

## `lsp-cli` Usage (REQUIRED)

Use `lsp-cli` for reproducible diagnostics and reference searches.

### Rust (rust-analyzer)

```bash
npx -y @mako10k/lsp-cli --root . --format pretty ws-symbols "typecheck"
npx -y @mako10k/lsp-cli --root . --format pretty references src/types.rs 0 0
```

### kscr language server (`kscr-lsp`)

```bash
cd crates/kscr_lsp
cargo build --release

npx -y @mako10k/lsp-cli \
  --root . \
  --server-cmd "$PWD/target/release/kscr-lsp" \
  --format json batch <<'JSONL'
{"id":1,"cmd":"diagnostics","file":"./tests/example_hello.ks"}
JSONL

# Optional: pretty-print daemon events afterwards
# npx -y @mako10k/lsp-cli --root . --format pretty events --kind diagnostics --since 0 --limit 200
```

## Version bump policy

Keep versions aligned across Rust crates (and the npm package if used).

Bump **PATCH** when:
- Bugfixes that don’t change public surface behavior (incl. stdlib fixes).
- Internal refactors, docs, CI/editor config.

Bump **MINOR** when:
- Additions that are backwards compatible: new stdlib modules/functions/types, new CLI flags, new language features that don’t break existing code.
- New optional Cargo features / new builtins behind feature flags.

Bump **MAJOR** when:
- Breaking changes: syntax changes, type system changes affecting inference, stdlib API removals/renames, CLI breaking flags, changes to default semantics.

If a change affects both the Rust engine and stdlib semantics, treat it as at least **MINOR**.

## Quality Gates (before commit)

```bash
cargo test
cargo clippy -- -D warnings
```