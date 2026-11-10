# v0.1 implementation decisions

## Modules

`config` defines the project, Rule and Rule Set schemas and validates loaded
objects. `discovery` reads only the fixed configuration paths and resolves Rule
resources within their source boundaries. Local discovery and explicit external references share Rule
loading; the central document is intentionally different: project settings locally,
a single Rule Set in an imported directory. External sets select repository-relative
Rule directories; only those definitions are loaded and their IDs enter the resolver. `rules` resolves effective assignments;
`planner` prepares selection and review context; `checks` uses the shared process
runner; `reviews` validates provider verdicts; `report` persists results; `terminal`
renders them. There are no alternative configuration loaders or execution engines.

Loaded Checks hold either a literal command argument array or a validated absolute
entrypoint, plus source roots, declaring
Rule directories and optional interpreters. Filename entrypoints resolve within
the Rule directory; paths containing `/` resolve within the source repository root.
Execution rechecks both paths from that root without following symlinks.
Loaded Reviews hold prompt text and consumer-relative context. There is no parallel
metadata registry. Blocking belongs to each Rule and follows it into plans/results.
Plan schema 3 and report schema 4 describe output formats, independently of config.

## Desired-state reconciliation

Configuration validation describes valid desired state; `reconciliation` compares
it with external state and builds an ordered plan. The CLI owns confirmation;
`sync` and direct Git-backed `run` points share
inspection/application/verification. Direct `check` and custom points retain the
repository recursion guard, then execute without inspecting hook locations or
synchronizing hooks. Every Check subprocess receives the repository guard,
including direct executions without a hook context. Managed hook invocations still validate hook context and
synchronization read-only; the execution point must equal the Git event.
The CLI exposes explicit unattended approval through `sync --yes`;
it uses the same inspection, application and verification path as interactive
confirmation, including recursion protection and conflict detection.
For Git-backed points the rule engine starts only after that prerequisite.
Offline execution previews remain read-only and do not synchronize. No generic domain registry is needed:
Git hooks are the sole current reconciliation domain.

The plan records before/after hook bytes and modes. Application rechecks the
plan, stages replacements and uses per-file atomic renames with best-effort rollback.
It does not promise multi-file atomicity or protection from hostile concurrent
filesystem replacement. Ownership markers bind managed sections to the common
Git directory, allowing default hooks to serve linked worktrees of one repository.
Safe composition inserts a section before existing POSIX sh commands; no shell
parser or arbitrary rewriting is used. Repository-local custom paths are supported;
external paths, links and ambiguous ownership fail before mutation.

Explicit hook context is checked against the fixed event/execution point and effective
wrapper path. Hook invocations inspect the same plan read-only; checker subprocesses
receive arguments as indexed environment values plus a repository recursion guard.
No provider or network participates. See [usage and recovery](usage.md#optional-git-hook).

## Snapshot definition

A snapshot is Git HEAD plus a sorted manifest of SHA-256 hashes, byte sizes and
executable flags for current tracked files and nonignored untracked files returned
by `git ls-files --cached --others --exclude-standard -z`. Local configuration
documents, rule prompts/entrypoints, review entry/context files, and existing
local files explicitly named by check commands, provider commands/arguments or
parameter values are also included even when ignored. Check path references
retain their declared boundaries. Discovery is read-only and does not resolve
external imports. It includes disabled definitions but does not infer transitive
script dependencies. Ignored artifacts are not recursively scanned beneath
`.please/`. `.git/` and `.please/reports/` are always excluded. Ignored build
products, caches and secrets outside declared inputs are outside scope; a tracked
file stays in scope even if an ignore rule matches it. Deleted files are absent
from the manifest. There are no temporary commits, Git index writes or content
backups. Git subprocesses use `GIT_OPTIONAL_LOCKS=0` and do not run diff helpers.

Git index/staging changes alone do not invalidate content: this verifies working
files, not staging state or a future pushed commit. HEAD changes do invalidate it,
even if all working files are identical. File additions, edits, deletes and renames
are found by comparing manifests; a rename is counted as two paths. Changes made
and reverted entirely between snapshots cannot be detected. Snapshots are not
filesystem transactions. Please Behave captures before and after execution, and before an active provider call, and blocks
PASS if they differ; concurrent edits could evade this check if reverted.

Limits: 8 MiB per file, 128 MiB total contents, 100,000 paths, 128 MiB per Git path-list stdout
capture, and 16 levels of configuration nesting. Unsupported symlinks, submodules,
special files, non-UTF-8 paths, unreadable files or exceeded limits make coverage
incomplete. UNKNOWN and unavailable totals take precedence over known differences.
Incomplete snapshots also block verification with ERROR and prevent reviews.
Git command deadlines are 10..30 seconds. If enumeration itself fails, verification
writes an ERROR report with an explicitly incomplete baseline when possible.

No previous contents or line counts are stored. Therefore additions/deletions of
lines since verification cannot be derived reliably, and are explicitly unavailable
both per file and in aggregate. Paths and exact path totals are available for
complete baselines. Binary files up to the same size budget are hashed normally.

## Execution and reporting

Checks tokenize the preferred command string into an argument array at load time.
Deprecated literal arrays bypass tokenization. Checks run the argument array
directly, run a file entrypoint directly, or pass a file entrypoint to one explicit
interpreter. All use the same process runner, without an implicit shell or language
detection. String quoting and escaping are defined in the configuration reference.
The consumer remains the working directory; PLEASE_RULE_DIRECTORY supplies the
absolute Rule directory. Exit 0 is PASS, 1 is FAIL and 2 is ERROR; unexpected
numeric exits produce an explicit FAIL diagnostic.
Check ERROR, fail-fast NOT_RUN and CANCELLED prevent success; dependency NOT_RUN
inherits the originating outcome. Advisory review errors remain visible.

Commands run through a bounded scheduler without an implicit shell. Both pipes are drained
concurrently to EOF, retaining only the first 64 KiB each and their complete byte
totals. Git path-list stdout instead uses the snapshot byte budget described above;
stderr remains at 64 KiB. Truncation is recorded. Timeouts, signal termination, spawn failures and
capture failures are ERROR. POSIX processes run in separate groups
and descendants are killed on exit/timeout. Windows uses `taskkill /T /F` for timeout
cleanup; detached descendants may escape termination. Pipes remaining open beyond
two seconds produce ERROR. Optional fail-fast termination sends SIGTERM to running
check groups, waits the configured grace period, then sends SIGKILL if necessary. A surviving descendant
receives the same grace even when its direct parent exits first. Cancellation
retains captured output and is reported as CANCELLED; reviews are unaffected.
This is not a hostile-process sandbox.

Reports contain UTC timestamps, repository/project, HEAD, manifest/fingerprint,
per-item status/duration/evidence/output and overall result. ERROR takes precedence,
then blocking FAIL; fail-fast NOT_RUN and CANCELLED block success but do not replace an existing FAIL with ERROR.
Dependency skips carry an optional blocked_by identity and do not independently
block success; saved reports validate that it names a declared non-PASS prerequisite.
Provider review durations reflect their batch invocation, not individual model timings;
do not sum them. Available token usage is summed across batches;
missing counters stay unavailable and are never estimated.

JSON is synced to a temporary file in the local reports directory and renamed
atomically over `last.json`; on Unix the directory is synced too. Normal file
permissions for tempfile (owner-only on Unix) protect local reports. Reports are
not signed or tamper-proof; repository configuration and local report storage are
trusted. Corrupt or incompatible reports fail explicitly. Configuration errors
leave a prior report untouched; status then sees changed configuration as STALE.

## Provider boundaries

The planner prepares provider-neutral review input once (`review_input`): selected
requirement excerpts, manifest hints and the bounded orientation diff. Execution
passes that exact preparation to the provider. Prompt rendering does no filesystem
reads or Git work; independent snapshot stability checks still guard execution.

Each batch of Reviews sharing a provider reference and resolved parameter environment receives configured
criteria, deduplicated requirement excerpts with original line numbers, and bounded
orientation data. Exact per-review entry
files focus the initial manifest/diff when every active review is focused; otherwise
project-wide orientation remains. Markdown sections are selected by unique ATX
heading, excluding fenced/indented code, with overlaps merged and a strict 64 KiB
selected-text budget. No context is silently dropped. These are entry points,
not an evidence boundary; dependencies can be inspected using targeted read-only
queries. Full working-tree snapshots and file/line evidence validation remain.

Adapters receive prepared evidence and the common verdict schema through the
[versioned protocol](adapter-protocol.md). They own authentication, tool access,
model settings and provider response translation. The core supervises processes
and validates verdict coverage, evidence paths, line ranges and input stability.
No provider implementation or default is supplied.

The provider trait returns a raw final document so tests can inject successful,
failed or malformed responses without real credentials. Schema-shaped serde types
reject unknown fields, wrong types, invalid statuses and duplicate object keys.
Semantic checks require exact requested ID coverage, one verdict each, nonempty
reasons, at most 20 findings per review and real in-manifest file/line references
for substantive PASS/FAIL. Any invalid combined document makes every review in that batch
ERROR, even if another verdict looked valid. ERROR can express model uncertainty.
Grounded references are necessary, not proof of accurate conclusions.

Parameter declarations live in the existing Rule schema. Loaded Rules carry a
binding of those declarations and import-local values. The shared `parameters`
module resolves and validates only active Rules, and configures child environments.
The planner counts distinct valid pairs of provider reference and review environment;
execution groups Reviews by that pair and reuses the prepared context. Inherited parameter variables
are removed before applying values. No global environment mutations occur.
Missing check entrypoints and review prompts are retained for per-rule ERROR
results; disabled Rules need neither implementation nor parameter resolution.

## External source boundaries

`external` supplies each import binding to the existing discovery/validation flow.
`external::cache` owns host Git, shared object storage, immutable checkouts,
integrity receipts and OS locks. Each selected directory contains exactly one
set. Its explicit Rule directory references resolve within the immutable repository
checkout, allowing multiple sets to share definitions and resources. The ordinary
resolver validates their IDs and source execution-point names within each binding. Per-import mappings translate effective
assignments after inheritance; all resulting names must be consumer-known.
Mappings never affect cache keys or immutable source files. Namespacing follows validation,
before effective results merge. Local resolution runs first, preventing consumer
references from modifying imports. No second execution engine is introduced.

`sync` refreshes sources and atomically publishes per-project resolution state
only after complete configuration validation. Hook reconciliation still follows
validation; its failure does not invalidate successfully resolved configuration.
Execution/previews read state and validate cache integrity without fetching or
creating missing state. Project locks span validation and publication. Different
projects also lock shared cache writers.

Repositories are keyed by the selector's SHA-256; checkouts by resolved commit.
No URL is persisted in cache remotes or fetch metadata. Git plumbing materializes
commits without repository hooks, global filters or submodule initialization.
Failed validation can leave reusable cache contents but never changes project
resolution state. Corrupt immutable entries fail explicitly. See
[storage, trust and recovery](external-rule-sets.md).

Checks use the consumer root. Review prompts resolve within their Rule directories;
the loader enforces containment. Checks receive PLEASE_RULE_DIRECTORY for resources. No argument rewriting or working-directory
switch occurs. Providers, snapshots, selection and reports remain shared.

The repository uses the same unversioned Rule model with the published Please
Behave release pinned in `.please-version` as its governor. Candidate binaries are exercised only in isolated fixture repositories. No hook installation, remote services or releases
are triggered by loading configuration.

Named provider configurations reuse `ProviderConfig` and the ordinary bounded,
symlink-free TOML loader. Local files live in `.please/providers/`; imported files
live in `providers/` inside the selected set directory. Independent
`provider_imports` instead select a package directory containing `provider.toml`
and adapter resources, with a consumer-assigned ID. Both import kinds share
cache retrieval and atomic resolution state; neither infers dependencies.
Independent package references resolve into the verified cache at invocation,
without copying adapters into the consumer. Provider origins participate in plan
identity and remain visible in reports. Scope-local rule validation
runs first. Provider definitions merge globally, rejecting collisions except for
repeated references to the same repository/commit/file. Review references resolve
after merging all explicit imports, before publishing resolution state. No source
files are rewritten. The explicit project default remains separate and is selected
only when a review omits its reference. All invocations use the same executable
adapter boundary, with matching provider references, Rule parameter environments
input sources and diff selections batched together. Each review carries independently merged
adapter settings. See the [adapter protocol](adapter-protocol.md).

File-change triggers filter enabled, point-selected Reviews inside the existing
planner before preparing input or counting provider groups. `git_snapshot::changed_paths`
resolves an explicit `PLEASE_CHANGE_BASE` and uses bounded NUL Git output for a
net commit-to-worktree comparison, including untracked configuration. The baseline
commit and omission decisions enter plan identity. No history inference, fetch,
provider-specific CI resolution or additional scheduler is involved. Omitted reviews
are excluded from required IDs/results; reports retain their omission metadata.

Check dependencies follow DEC-0016; review gating follows DEC-0018. Discovery
canonicalizes top-level depends_on; import merging qualifies bare references within
their binding. The planner validates exact references against the effective check
selection and rejects cycles before execution. The existing scheduler dispatches
only checks with all prerequisites PASS and an available resource. Pending dependency
waits consume no worker slots. Known unsuccessful prerequisites propagate NOT_RUN
before fail-fast marks unrelated pending work. The report's aggregate outcome
ignores dependency skips and retains the originating failure's enforcement. Reviews
remain in the separate phase behind that aggregate gate and snapshot validation.

Commit-header reviews explicitly select a separate evidence source. The validated
managed hook supplies the message path only to core preparation; adapters receive
its bounded first line, never the raw path or Git hook arguments. Header contents
participate in plan identity and are checked for stability independently of the
working-tree snapshot. Input source separates adapter batches, and the verdict
validator requires header-specific evidence rather than a fabricated file location.
