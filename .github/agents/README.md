# Custom Agents

These GitHub Copilot profiles are optional specialists. The repository-wide contract is `AGENTS.md`; no profile is a mandatory gateway for another profile.

| Profile | Use when |
|---|---|
| `orchestrator.agent.md` | The user explicitly requests coordination, or independent workstreams have distinct decision value. |
| `issues.agent.md` | A report or idea needs requirements and acceptance criteria. |
| `plan.agent.md` | Accepted requirements need an implementation-ready plan. |
| `implement.agent.md` | The user requested a bounded code or documentation change. |
| `review.agent.md` | An existing design or diff needs read-only risk-first review. |
| `pr.agent.md` | An existing diff needs a pull-request draft. |
| `maintenance.agent.md` | Instructions or status documentation need reconciliation. |

Profiles use GitHub's portable tool aliases: `read`, `search`, `edit`, `execute`, `agent`, and `web`. Unknown or unavailable tools are not prerequisites and must not be installed automatically.

`disable-model-invocation: true` keeps each profile manually selected. Selecting a profile does not grant commits, pushes, GitHub mutations, releases, installation, rollback, or cleanup authority.

Use `GUIDANCE_SCENARIOS.md` to check that future instruction changes handle
the same mechanisms under positive, paraphrased, and non-applicable cases.
