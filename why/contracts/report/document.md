# Report document and lifecycle

Producer: Please Behave execution. Consumers: `please status`, `please publish` and tools reading
saved verification evidence. [api.yaml](api.yaml) identifies this API and its
requirements. [report.schema.json](report.schema.json) defines structure, types
and permitted values, including the [snapshot](snapshot.md). The embedded `plan`
references the prepared form of the independent [Execution Plan API](../plan/api.yaml).
A diagnostic preview is not a report plan.

## Persistence and compatibility

Reports live in `.please/reports/<number>.json` in the consuming project. After
executing its Rules, a run atomically saves its result as a new report with the
next higher number, including FAIL, ERROR, cancelled and blocked outcomes. The
number is the report's file name and part of no document; it starts at 1, rises
with every saved report and is never reused while a newer report exists.
Concurrent runs receive distinct numbers. The run's summary line names the number;
the path follows from it. An empty selection can also produce a report; its PASS establishes no Rule
evaluation. Invalid configuration and plan-only previews save no report, take no
number and leave saved reports unchanged. Failure to save is an execution error;
an older report is not evidence for that failed write.

After saving, only the newest `report.retain_runs` reports, counting the one
just saved, are kept; older files are removed and numbering continues. Removal
failures never fail the run. The setting's default and permitted values are
defined by the [configuration schema](../configuration/project.schema.json).
Report retention is independent of run-context retention: a report holds its
own plan and [snapshot](snapshot.md) and does not reference the run context,
which can be removed earlier or later. Reports are local, unsigned evidence, not tamper-proof
attestations. Keep them out of version control and do not print credentials.
The report directory must be local rather than a symlink to another directory.

New reports carry this API's experimental version in `schema_version`. Readers
accept both transition labels defined by the schema with identical structural
and semantic validation; unsupported versions and non-integer JSON version
representations are rejected without retry or fallback. The embedded
`plan_schema_version` is validated independently. Snapshot has no separate version.
Compatibility follows [DEC-0026](../../decisions/DEC-0026.yaml), independently of
CLI release numbers. Unknown structural fields are rejected; arbitrary adapter profile
parameter keys in the embedded plan remain data. Older reports can omit
`checks_only` (false) and usage metadata (unavailable). Absent optional result
metadata means no explanation or dependency blocker and no configuration error.
The reader limits the report file to 128 MiB.

## Identity and evidence

`timestamp` is the run's UTC timestamp. `repository` is the canonical absolute
repository root and must match the repository where the report is read.
`project` preserves the configured project label. The report has no run-level
adapter: one run can use several adapter profiles, and an adapter profile is a
configured adapter call whose AI service or model Please Behave does not know.
Declarations and imported provenance are retained in the saved plan.

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
A review result whose adapter call was made carries `adapter`, the ID of the
named adapter profile that produced it, matching the adapter profile of that review in
the saved plan. The explicit default adapter profile has no ID, so results of reviews
that use it omit `adapter`, as do reviews that made no adapter call (NOT_RUN,
configuration errors, input changed before the call) and all non-review results.
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
from the overall calculation, with no adapter usage. A checks-only PASS covers
checks alone. `overall` must equal the outcome derived from these results.

## Captured output and usage

`stdout` and `stderr` retain each process stream's first 64 KiB, total byte count
and truncation indicator. Captured text is decoded for display; byte counts refer
to process output, not JSON string length. A missing/null capture means no capture
is available. Retained output is not automatically redacted; terminal presentation
sanitizes untrusted text. `please status --details` makes retained output and
findings available without running an adapter again.

`adapter_usage` holds available counters summed across all adapter calls. A
counter is unavailable if any call did not report it as a positive number.
Unavailable values remain null or absent, never a fabricated zero or partial sum.
Cached input, cache writes and reasoning counters are subsets and are not added
again. `token_usage` is input plus output when both are available and the total
is representable. These are AI provider claims, not billing, remaining quota,
monetary estimates or a token budget. Missing telemetry does not invalidate an
otherwise valid verdict.

## Reading and status

Readers reject corrupt, unsupported, internally inconsistent and foreign reports.
Structural schema validation does not replace cross-field, plan/result, dependency
and repository checks. A rejected report is an error, never valid evidence.
`status` performs no Rule execution and shows the report with the highest number,
or the one selected with `--report <number>`, naming its number. It evaluates
[freshness](snapshot.md) of the shown report against that report's saved input.
A missing report, including a requested number that is not saved, exits 3; an unreadable or invalid one exits 2.
A CURRENT report preserves its saved outcome's exit code (PASS 0, FAIL 1, ERROR 2).
STALE exits 1; UNKNOWN exits 2 and takes precedence over known differences or an
otherwise successful saved outcome. Freshness does not rewrite the saved verdict.

## Publishing evidence

`please publish` reads the newest report using the same selection and freshness
check as `please status` ([REQ-0342](../../requirements/REQ-0342.yaml),
[REQ-0238](../../requirements/REQ-0238.yaml)). Delivery requires CURRENT and a
saved HEAD, regardless of the report's overall verdict.

The [gateway ingest contract](https://github.com/please-behave/gateway/tree/main/why/contracts/ingest)
defines the attestation. The projection uses the report's `schema_version`,
`snapshot.head` as `commit`, the saved plan's execution point as the sole
`execution_points` entry, and the saved `overall`. Each result, including an
integrity result, contributes its `id`, `kind` and `status`. A non-blocking
review's FAIL or ERROR maps to WARN and CANCELLED maps to NOT_RUN. All other
statuses pass through. The projection excludes the local repository path and
all captured output, messages, findings and explanations.

The repository identity is the configured override or Git's fetch URL for
`origin`, normalized as host and path. Normalization preserves path case and
removes the transport scheme, user information, port, URL query and fragment,
surrounding path slashes and a terminal `.git`. The hostname is lowercased.
SCP-style locations are supported. A local origin requires an override.

Delivery uses a single POST with bearer authorization from the configured
environment variable. The command succeeds on HTTP 201 and reports credential
rejection, rate limiting, document rejection, other gateway responses and
transport failures distinctly. It follows no redirects and leaves the report
and its verdict intact.
