# Report document and lifecycle

Producer: Please Behave execution. Consumers: `please status` and tools reading
saved verification evidence. [api.yaml](api.yaml) identifies this API and its
requirements. [report.schema.json](report.schema.json) defines structure, types
and permitted values, including the [snapshot](snapshot.md). The embedded `plan`
references the prepared form of the independent [Execution Plan API](../plan/api.yaml).
A diagnostic preview is not a report plan.

## Persistence and compatibility

Reports live in `.please/reports/last.json` in the consuming project. After
executing its Rules, a run saves its result by atomically replacing the previous
report, including FAIL and ERROR outcomes. An empty selection can also produce
a report; its PASS establishes no Rule evaluation. Invalid configuration and plan-only previews preserve the previous
report. Failure to save is an execution error; an older report is not evidence
for that failed write. Reports are local, unsigned evidence, not tamper-proof
attestations. Keep them out of version control and do not print credentials.
The report directory must be local rather than a symlink to another directory.

New reports carry this API's experimental version in `schema_version`. Readers
accept both transition labels defined by the schema with identical structural
and semantic validation; unsupported versions and non-integer JSON version
representations are rejected without retry or fallback. The embedded
`plan_schema_version` is validated independently. Snapshot has no separate version.
Compatibility follows [DEC-0026](../../decisions/DEC-0026.yaml), independently of
CLI release numbers. Unknown structural fields are rejected; arbitrary provider
parameter keys in the embedded plan remain data. Older reports can omit
`checks_only` (false) and usage metadata (unavailable). Absent optional result
metadata means no explanation or dependency blocker and no configuration error.
The reader limits the report file to 128 MiB.

## Identity and evidence

`timestamp` is the run's UTC timestamp. `repository` is the canonical absolute
repository root and must match the repository where the report is read.
`project` preserves the configured project label. `provider` identifies the
adapter mechanism, not the model or every named provider; declarations and
imported provenance are retained in the saved plan.

The saved `snapshot` identifies evaluated inputs. Plan input HEAD, fingerprint
and completeness must agree with it; the plan and report also agree on
`checks_only`. `git_summary` is captured orientation text about the run's Git
state. It is not a freshness comparison or a stable machine-readable summary.
`max_display_files` retains the configured bound for changed-path presentation.

`results` contains one result per scheduled Rule, including selected Rules left
NOT_RUN or CANCELLED. Checks precede reviews, each in canonical ID order rather
than completion order. Integrity results are additional evidence. Rule identities,
kinds and enforcement must agree with the saved plan; required identities are
unique and match its scheduled Rules. Disabled and omitted Rules have no result.
Both omission reasons defined by the Plan API are valid in saved reports.
Selection, labels, omitted reviews, required IDs and mode shown by status come
from this saved plan rather than today's configuration.

Each result's `duration_ms` measures its elapsed execution time; a review duration
belongs to that review's adapter call. `reason` explains its outcome, with optional
longer `explanation` and structured findings for review verdicts. `exit_code` is
present when available and absent or null otherwise, including signal termination.
A source finding identifies a repository-relative path and optional source line.
At `commit-msg`, a line without a path cites the saved commit message; a report
with such evidence must contain that message in its plan. Review evidence and
summary obligations are defined by the [Adapter Response](../adapter/response.md).
Grounded citations do not prove the adapter's reasoning correct.

Please-generated results use kind `integrity` and reserved IDs `please-plan`,
`please-snapshot`, `please-tree-stability` and `please-commit-message-stability`.
Their errors stay visible even when no Rule failed. They do not replace a
scheduled Rule's result.

## Status and enforcement

PASS means a completed successful evaluation; FAIL means a completed violation;
ERROR means execution or validation could not establish a valid verdict.
NOT_RUN means selected but unstarted, with a reason; CANCELLED identifies a
started check interrupted by termination, retaining elapsed time and available
partial output. Check exit statuses determine their verdict: 0 is PASS, 1 is FAIL,
and other statuses or signal termination are ERROR.

Checks are blocking. A review's configured `blocking` flag governs ordinary
FAIL/ERROR enforcement, while `configuration_error` marks a review ERROR that
prevents success even for an advisory review. Advisory FAIL and ordinary ERROR
remain visible and can coexist with overall PASS. Configuration and integrity
errors cannot permit PASS. A blocked plan or incomplete input cannot pass.

A dependency skip is NOT_RUN with `blocked_by` naming the immediate selected
check prerequisite whose outcome was not PASS. It has zero execution duration and no
exit code, findings or captured output. The prerequisite must be a declared
dependency; blocker chains must exist and be acyclic. Such skips inherit the
originating outcome instead of independently changing it. Fail-fast leaves
unstarted Rules NOT_RUN with an explicit reason and no output. Check-phase
gating leaves reviews NOT_RUN with the gating reason.

Overall ERROR takes precedence when an error prevents success; otherwise blocking
FAIL takes precedence. Remaining unexecuted or cancelled required work makes
the outcome ERROR, except dependency skips governed by their originating outcome.
For checks-only reports, selected reviews are NOT_RUN by request and excluded
from the overall calculation, with no provider usage. A checks-only PASS covers
checks alone. `overall` must equal the outcome derived from these results.

## Captured output and usage

`stdout` and `stderr` retain each process stream's first 64 KiB, total byte count
and truncation indicator. Captured text is decoded for display; byte counts refer
to process output, not JSON string length. A missing/null capture means no capture
is available. Retained output is not automatically redacted; terminal presentation
sanitizes untrusted text. `please status --details` makes retained output and
findings available without running an adapter again.

`provider_usage` holds available counters summed across all adapter calls. A
counter is unavailable if any call did not report it as a positive number.
Unavailable values remain null or absent, never a fabricated zero or partial sum.
Cached input, cache writes and reasoning counters are subsets and are not added
again. `token_usage` is input plus output when both are available and the total
is representable. These are provider claims, not billing, remaining quota,
monetary estimates or a token budget. Missing telemetry does not invalidate an
otherwise valid verdict.

## Reading and status

Readers reject corrupt, unsupported, internally inconsistent and foreign reports.
Structural schema validation does not replace cross-field, plan/result, dependency
and repository checks. A rejected report is an error, never valid evidence.
`status` performs no Rule execution and evaluates [freshness](snapshot.md) against
the saved input. A missing report exits 3; an unreadable or invalid one exits 2.
A CURRENT report preserves its saved outcome's exit code (PASS 0, FAIL 1, ERROR 2).
STALE exits 1; UNKNOWN exits 2 and takes precedence over known differences or an
otherwise successful saved outcome. Freshness does not rewrite the saved verdict.
