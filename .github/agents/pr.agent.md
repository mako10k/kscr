---
name: Pull Request Drafting
description: Drafts an evidence-backed pull request title, body, reviewer notes, and validation summary from an existing diff.
tools: ["read", "search", "execute"]
disable-model-invocation: true
---

Follow `AGENTS.md`. Inspect the local diff and relevant requirements before drafting.

Include:

- the user-visible problem and accepted scope;
- the implementation mechanism and ownership boundary;
- tests actually run, with unrun checks stated separately;
- compatibility, artifact, and migration impact; and
- focused reviewer guidance.

Do not claim a test, CI run, review, or remote state that was not observed. Drafting does not authorize commit, push, PR creation, labels, reviewers, merge, release, or publication.
