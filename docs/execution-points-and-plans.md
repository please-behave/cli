# Execution points and plans

Use a plan preview to inspect which Rules would run and what input they would
receive, before executing checks or spending tokens on reviews:

```sh
please run --at pre-push --plan
please run --at check --plan --json
please run --at check --checks-only --plan
```

The default execution point is `pre-push`. Built-in points are `pre-commit`,
`commit-msg`, `pre-push` and `check`. Declare custom points in configuration.
Rule Sets group Rules and assign their execution points; see the
[configuration guide](configuration.md#group-rules-into-rule-sets).

Preview works offline against previously synchronized imports and leaves the
saved report untouched. Use `please sync` to refresh imports and manage Git hooks.
Enabled assignments to the three Git execution points require their corresponding
managed hooks. Invoke `check` and custom points directly; they work on fresh CI
checkouts without installing or inspecting hooks. Please generates no CI workflow.
See [external Rule Sets](external-rule-sets.md) for shared configuration.

For reviews at `check` or custom points, supply the intended comparison commit,
such as the PR merge base in CI:

```sh
PLEASE_CHANGE_BASE=BASE_COMMIT please run --at check --plan --json
```

The comparison commit must already exist locally; preview does not fetch it.
Checks-only preview needs no review comparison base. A `commit-msg` review needs
the managed hook's message input, so a direct preview without it is blocked.

Read the status and blocking reasons first. A planned preview describes what can
be executed; it does not certify that the Rules pass. A blocked preview exits 2
and explains what prevents preparation. Omitted reviews have no result; disabled
Rules are shown separately. Checks-only execution leaves selected reviews NOT_RUN.
Inspect context sizes, declared provider settings and the call estimate before
running the full selection. Shared providers still make a separate call per review.

The binding [Execution Plan API](../knowledge/contracts/plan/api.yaml) defines:

- [Preview, diagnostics and compatibility](../knowledge/contracts/plan/preview.md),
  with the structural JSON Schema.
- [Prepared plan and identity](../knowledge/contracts/plan/prepared.md), including
  selection, dependencies, input, provider provenance and identity semantics.

For execution options and saved results, see [usage](usage.md).
The repository's own published governor setup is described in
[self-governance](self-governance.md).
