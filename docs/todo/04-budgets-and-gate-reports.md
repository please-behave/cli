# TODO 4: Budgets and complete gate reports

Status: TODO. Depends on [TODOs 1–3](README.md).

## Goal and decisions

Bound waiting and provider activity, and make incomplete verification impossible
to mistake for a complete gate. Extend existing command/provider deadlines and
context/output limits rather than introducing an agent control loop.

- Run budgets cover total elapsed run time, maximum model calls and prepared
  review-context bytes. Proposed `[budgets]` fields and initial defaults are
  `total_timeout_seconds = 600` (1..3600), `max_model_calls = 4` (1..64), and
  `max_context_bytes = 65536` (1..65536). Invalid values fail validation. These
  are execution policy limits, not predictions of model speed, price or tokens.
  The context ceiling retains the current 64 KiB safety bound; existing narrower
  limits remain in force. Count prepared evidence and criteria bytes across all
  groups, deduplicating within each group, with cross-group copies counted again.
- The offline plan gives a conservative model-call upper bound after grouping.
  If the call/context plan already exceeds a hard budget, reject before checks.
- Total elapsed time includes preparation, checks and provider discovery/calls.
  Every child gets at most the remaining deadline. On exhaustion, terminate its
  process tree, preserve available evidence and mark remaining rules NOT_RUN.
  Report-writing cleanup has a separate bounded allowance.
- No automatic retries or fallback model calls. No hidden extra review calls.
  A failed call still consumes the call allowance and retains available usage.
- Never advertise an enforceable token ceiling based only on a prompt instruction
  or after-the-fact usage counter. A hard token cap is available only when the
  provider demonstrably enforces it; otherwise reject that requested capability.
- Measured token thresholds may warn and stop subsequent groups, but cannot
  guarantee the current call stayed below the threshold. Label them accordingly.
- No currency/quota estimates without authoritative billing data. Cached input
  and reasoning counters remain subsets where reported, not extra total tokens.

## Work

- [ ] Implement the budget fields, units, defaults and provider capability checks.
- [ ] Enforce admission checks and a shared deadline across all phases/groups.
- [ ] Show immediate phase/rule progress and elapsed time during long calls,
      without dumping raw traces or source; preserve non-TTY readability.
- [ ] Persist planned/attempted/completed calls, per-call usage availability,
      elapsed time, budget exhaustion and all omissions/NOT_RUN reasons.
- [ ] Record required IDs, policy/plan identity, input and diff identity, scope,
      selection completeness, execution completeness and input stability.
- [ ] Render compact verdicts/short AI summaries in stdout; keep full findings,
      check output and detailed budget evidence in the saved report/details.
- [ ] Test deadline cancellation, process cleanup, no retries, partial usage,
      unsupported hard caps and incomplete report rejection using fakes.

## Done when

Execution succeeds only if all required Rules complete without ERROR or blocking FAIL, selection/execution are
complete, and the verified input remains valid. Configuration, provider errors,
missing findings, exhausted budgets and incomplete snapshots cannot become PASS.
Partial runs remain inspectable with honest scope and one usage record per actual
call. Status needs no new provider invocation. Tests verify strict call and time
limits without using a paid model.

Implementation commits: none.
