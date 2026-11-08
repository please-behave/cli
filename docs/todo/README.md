# Rule and review roadmap

This roadmap tracks implementation in the order below, one coherent, validated
change at a time. Item 1 is implemented; item 2 is superseded by adapters and items 3–5 remain planned work. Writing a
specification alone does not complete an implementation.

| Order | TODO | Status | Depends on |
| --- | --- | --- | --- |
| 1 | [Execution points and plans](../execution-points-and-plans.md) | DONE (`18897c9`) | — |
| 2 | [Per-review MCP permissions and sessions](02-review-mcp-and-sessions.md) | Superseded by adapters | 1 |
| 3 | [Inputs and review scope](03-inputs-and-review-scope.md) | TODO | 1, 2 |
| 4 | [Budgets and gate reports](04-budgets-and-gate-reports.md) | TODO | 1–3 |
| 5 | [Measure and decide result caching](05-cache-decision.md) | TODO; decision task | 1–4 |

## Agreed decisions

- Checks are deterministic and use no LLM provider. Reviews produce structured
  LLM verdicts. Both are rules. Execution Points select Rules for a particular invocation.
- Provider-specific permissions and sessions belong to external adapters.
- Reviews default to snapshot scope. Change reviews require an explicit diff
  base in the selected execution context, including whether to compare directly or against
  a merge-base. No inferred standard branch or silent HEAD fallback.
- Worktree, index and commit are different inputs. A PASS applies only to the
  selected rules, scope and recorded input; it does not prove arbitrary commits
  or deployment artifacts satisfy policy.
- Checks run before reviews. Checks-only execution never starts provider
  adapters or models. No paid calls in automated tests.
- Compact output keeps verdicts and short AI summaries; full evidence belongs in
  saved details. Usage is measured per actual model call, never per duplicated row.
- No automatic repairs, retries, unconfirmed hook installation or credential copying. Each
  implementation updates its TODO status, documentation and acceptance tests,
  then is committed and pushed under the repository's Git policy.

## Completion tracking

Mark an item done only after implementation, relevant tests, documentation and
commit/push are complete. Record the implementing commit(s) in that item's file.
For item 5, a documented decision to defer caching is a valid completion; writing
an unmeasured cache implementation is not required.

The [configuration reference](../configuration.md) describes current
behavior. The [execution-point specification](../execution-points-and-plans.md) contains the
detailed contract for item 1; later TODOs extend it without pretending those
extensions already exist.
