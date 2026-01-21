# Agents

This folder contains reusable GitHub Copilot Chat agents for this repository.

## Available agents

- `issues.agent.md` (まなみ（要望）): define requirements/specs; create/modify GitHub Issues
- `plan.agent.md` (さくら（計画）): break Issues into implementation-ready plans
- `implement.agent.md` (こうた（実装）): make focused code changes + validate via tests
- `review.agent.md` (りん（レビュー）): review changes; identify risk and missing tests
- `pr.agent.md` (はる（PR）): draft PR title/description/checklist/release notes
- `maintenance.agent.md` (ゆい（保守）): maintain instructions/docs/agent prompts consistency
- `orchestrator.agent.md` (あおい（司令）): route work to the right agent and integrate outputs

## Conventions

- Repo-facing content is English-first.
- Follow `.github/copilot-instructions.md`.
- Keep diffs minimal; avoid unrelated refactors.

## Tools policy

- Prefer least privilege: only enable tools the agent actually uses.
- Keep `#tool:<name>` references consistent with the frontmatter `tools:` list.
- Use only tool names recognized by the GitHub Copilot Chat agent runner in this repo.
