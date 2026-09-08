# GitHub Copilot Repository Instructions

Read and follow `@AGENTS.md`. It is the repository-wide contract for project goals, source-of-truth order, authority boundaries, implementation ownership, validation, review, releases, and documentation status.

This file intentionally contains only Copilot-specific adaptation:

- Repository-facing output is English-first unless the user asks otherwise.
- Do not infer implementation, GitHub mutation, commit, push, release, install, rollback, or cleanup authority from a planning, explanation, diagnosis, or review request.
- Custom agents in `.github/agents/` are optional specialists. Do not route every task through an orchestrator or gatekeeper, and do not delegate unless it adds an independently useful contribution.
- Use the standard tool aliases declared by each custom agent. If a tool is unavailable, use an in-scope equivalent or report the unverified boundary; do not invent a tool name or install a replacement automatically.
- Prefer repository search and focused tests. External LSP clients and `npx -y` are optional diagnostics, not prerequisites.
- When editing packaging or release workflows, identify the current shipped artifact set before changing it and preserve that set unless the user explicitly accepts a contract change.
- Before presenting completion, distinguish the tested baseline from optional, experimental, release-ready, and self-hosting-complete claims.

Use `.github/agents/README.md` to select a specialist profile when the user or task benefits from one.
