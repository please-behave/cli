# Execution points and plans

Rules are checks or reviews. Rule sets reference rule IDs and assign execution
points. Each enabled rule uses the union of its Rule Sets' points; without
membership it uses its own run_at. An overridden own list generates a hint.
Disabled Rules never run. Present Rule Sets always apply. Every reference is
validated even when disabled. See the [configuration reference](configuration.md).

Use `please run --at pre-push --plan` for an offline, human-readable plan with
English labels and explanations. Configured text and technical identifiers are
preserved. Add `--json` for the structured plan.
The default point is pre-push. Built-in points are pre-commit, commit-msg, pre-push and
check. Enabled effective assignments to the first three require managed hooks;
each hook invokes its identically named point, with no configurable association.
`check` is invoked only explicitly with `please run --at check`, without inspecting or synchronizing Git hooks.
Direct custom points behave the same way, including on fresh CI checkouts with
missing or outdated hooks. `please sync` manages hook installation and updates.
Please Behave generates no workflow and requires no CI integration. Custom points, including release, must be centrally declared.
Empty points are valid.

The loader records source paths. The domain resolver computes effective
assignments once. Validation, CLI planning and execution share that result.
Review file-change triggers then filter enabled reviews at the requested point,
before context preparation or provider requirements. Checks run before scheduled
reviews, deduplicated and dispatched by estimated remaining dependency-path
duration among runnable checks, with canonical ID order for ties. Reports
retain ID order; resource waits and concurrent completion do not change selection. Every scheduled rule is required; dependency skips inherit the originating outcome.
`run --fail-fast` may leave selected rules `NOT_RUN` after a blocking failure or
fatal execution error. Completed work retains its results; running checks finish
by default or become `CANCELLED` when configured termination interrupts them.
This invocation-only option does not change plan identity or selected IDs. Checks-only execution rejects required reviews before any command starts.

Plans include the selected point, rule sources, effective points, contributing
sets, required IDs, input identity, bounded context and declared provider settings.
Each review with an explicit provider reference also records `provider` (the named
ID) and `provider_config` (its complete effective settings). Absent fields select
the plan's legacy project provider. Model calls count distinct provider references
and resolved Rule parameter environments and include_diff settings for adapters.
Local scheduling history, concurrency limits and fail-fast termination settings change execution strategy and are excluded from semantic plan
identity. Resource declarations and canonical dependency IDs participate in check identity. No check, adapter
request, network access or reconciliation occurs during preview.
Incomplete snapshots or context preparation block execution. Preview does not
certify a later source state. Reports use schema 4 and plans use schema 3. Blocking metadata distinguishes
advisory FAIL/review ERROR from blocking failures and check/integrity errors; disabled Rules are listed separately.

Triggered reviews require an explicit `PLEASE_CHANGE_BASE` commit/ref, resolved to
`change_base` in the plan. Net commit-to-worktree changes include committed and
uncommitted work. Supply the intended PR merge-base commit in CI; no baseline is
inferred or fetched. Missing comparison data blocks the plan. Nonmatches appear
under `omitted` with `trigger_not_matched`, outside required IDs, results, context
and model-call estimates. Disabled/inapplicable reviews need no baseline. See
[trigger configuration and limits](configuration.md#add-a-review).

Every JSON preview includes `plan_schema_version`, `status` and `blocking_reasons`.
Configuration and selection errors use schema 3, `status: "blocked"` and nonempty
`blocking_reasons`, with exit code 2. They omit unavailable plan metadata instead
of inventing input or selection details. A prepared plan blocked by an incomplete
snapshot retains its full metadata. Consumers can read the common diagnostic
fields before processing plan details.

External Git rule sets select shared repository-relative Rule directories, then
use the same resolver in independent import scopes. The
loader maps effective points per import, then namespaces validated rules before
combining the effective result. Mapping accepts only used custom sources and
consumer-declared custom targets. Unmapped names retain their meaning; many-to-one
mappings deduplicate execution. Built-ins cannot be mapped. Plans
use previously validated cached revisions; sync refreshes sources. Origins
retain the resolved commit. See [external sets](external-rule-sets.md).

The repository's self-integration uses the separate published Please Behave version pinned in `.please-version`
governor with the same unversioned Rule model. Its CI setup calls `please sync --yes` before executing the
configured point. See [repository self-integration](self-governance.md).

Checks alone may declare top-level `depends_on`. Resolve exact IDs within the
effective enabled selection, after assignments and import mapping; no additional
rules are selected. Imported bare references use their own binding namespace.
Invalid references and cycles block preview and execution before any command runs.
Plans record sorted, deduplicated `depends_on` IDs. Reviews neither declare nor
satisfy dependencies. Dependency skips use NOT_RUN with `blocked_by` and the
prerequisite outcome in the reason. They inherit the original failure's enforcement;
non-blocking failures and their dependent skips permit reviews. Check errors,
blocking failures, incomplete fail-fast execution and input errors prevent reviews.
The check phase always finishes before review invocation.
