# Please Behave specification

Status: current implemented contract. Changes to behavior start here and in the
acceptance criteria, before implementation. Explicit user decisions take precedence.

The [product vision](docs/product-vision.md) defines direction and architectural
boundaries: Please Behave makes development expectations explicit, executable and
verifiable, independently of who writes the code. This document defines supported
behavior. The [roadmap](docs/todo/README.md) contains planned extensions; they are
not implemented merely because they have a proposal.

The [README configuration reference](README.md#configuration) defines every public
configuration field, default and limit. The [execution-point contract](docs/execution-points-and-plans.md)
extends this specification for plan schema 3 and report schema 4. These documents must agree; a contradiction must be resolved before work
proceeds, not interpreted as permission to choose either behavior.

## CLI-001: Commands and project boundaries

Please Behave is a small native Rust executable, named `please`. It evaluates outcomes;
it does not implement features, repair files, orchestrate coding agents, prescribe
a development methodology or run production infrastructure. Source, configuration
and documentation are English. The supported commands are:

| Command | Contract |
| --- | --- |
| `please init` | Initialize a new `.please/` directory once, install no hooks and make no commits. |
| `please sync` | Validate configuration and reconcile declared external state after confirmation, without checks or provider calls. |
| `please run` | Validate, select rules, execute checks before reviews and save results; reconcile hooks only for Git-backed points. |
| `please status` | Read saved results and compare input freshness, without rerunning rules. `--details` shows full findings and captured output. |
| `please help` | Show help listing exactly the five commands. |

Top-level help uses the heading `Please Behave` and the description
`Enforce development rules across coding agents and workflows.`
Its `help` command description is `Show help`.

`-h`/`--help`, `-V`/`--version` and `--no-color` remain supported. Help must not
present command aliases or renamed option aliases. Other
commands show the same top-level help content as `please help` and exit 2.
Invalid arguments and options retain normal parser diagnostics.
Run from the Git working-tree root; subdirectory execution is rejected.
Please Behave must not create commits, modify the Git index or silently discover
another root.

## RULE-001: Rules, Rule Sets and Execution Points

Rules are implemented as Checks or Reviews. Each declares id, optional name, description,
enabled (default true), blocking (default true), optional run_at and exactly one
implementation table. Rule IDs are unique within a governance scope. Rule Sets
require id, description, rules and nonempty run_at; no enabled/blocking/source
fields are accepted. Set IDs are independently unique.

A Rule's optional name is a short display label: 1..=80 characters, without
control characters or leading/trailing whitespace. Its default is the original
scope-local ID, including for imported rules. Names do not change identity,
selection, ordering, parameters or provider IDs. Duplicate display names receive
a short origin suffix and, where needed, an occurrence number. Plans save names
so progress, results and saved status agree; technical IDs and full provenance
remain available in JSON and details. Reports without optional names remain readable.

A present set overrides its rules' own run_at; memberships contribute their union.
Rule references accept the existing string or a table with required id and optional
run_at. The id is the same local ID or external directory path as the string.
An omitted reference run_at inherits the set list; a specified nonempty list
replaces it for that reference. Both Checks and Reviews use this contract.
All point lists are validated, including disabled, overridden and repeated
references. Repeated references union their effective lists without changing
identity or executing a rule more than once per point.
Standalone enabled rules use their own assignments. Repeated references within
a Rule Set contribute one membership. Enabled rules need an effective
point. Disabled rules remain validated but never run. All references must resolve
within the same scope, including disabled and overridden assignments. A rule runs
once per point within each binding; checks precede reviews; plans and results sort each kind by ID.
Built-in points
are pre-commit, commit-msg, pre-push and check. The first three use managed Git
hooks; check is invoked explicitly and never automatically. Custom names come only
from the consumer; release has no special behavior.
The default is pre-push. Empty points succeed without claiming rule PASS.

Acceptance: `tests/execution_points.rs::reference_overrides_replace_set_points_and_union_without_changing_identity`
covers inheritance, replacement and repeated references;
`tests/configuration.rs::reference_overrides_drive_check_review_plans_and_saved_reports`
covers Check/Review selection and reports;
`tests/external_rule_sets.rs::reference_overrides_preserve_paths_mapping_identity_and_offline_reports`
covers external paths and mapped offline execution.

## EXT-001: External Git Rule Sets

Only the consumer's config.toml accepts [[imports]], with git/path/ref
and optional per-import execution_points mapping and rule_parameters tables.
Each import is a separate binding, even when it selects the same source. Values
never merge across imports. Rule identities distinguish these bindings while
repeated references within a binding still execute once.
No Rule overrides or nested imports are supported. Mapping sources are custom
points used in that import; targets must be consumer-declared custom points.
Built-ins cannot be sources or targets. Resolve scope-local assignments first,
then map effective points and deduplicate before merging. Unmapped names must
still be known to the consumer. Each import has an independent mapping namespace;
many sources may share a target. Mapping never changes cached definitions,
revision identity or provenance and works offline without fetching.
Each selected directory defines exactly one Rule Set in config.toml. Its rules
are paths relative to the external repository root, such as
rules/checks/secret-scan or rules/reviews/architecture. Each path names a Rule
directory directly under checks/ or reviews/; it contains rule.toml and its
resources. Multiple sets may reference the same directory. Only referenced Rules
are loaded, repeated paths load once, and distinct directories with duplicate
Rule IDs fail within a binding. References are resolved to IDs before ordinary
assignment validation. Each import remains a separate execution/parameter binding.
No nested imports or references to other sets are supported. Absolute paths,
parent traversal, symlinks and references outside the repository fail. Local sets
continue to reference IDs. This replaces external sets' former directory-local
ID references; migrate each entry to its repository-relative Rule directory.
Rules are namespaced only after scope-local validation; provenance retains source
identity, selected path/ref and resolved commit.

sync resolves exact branches, tags and full hashes using host Git, materializes
shared immutable revisions and validates everything before atomic publication of
project resolution state. Failure preserves prior state. Execution, hooks and
plans use the validated cache offline, never substitute revisions, and fail when
state or contents are missing/corrupt. Git-directory state, shared caches, bounded
OS locks, integrity receipts and retrieval trust boundaries remain in effect.
Checks run in the consumer root; PLEASE_RULE_DIRECTORY locates bundled resources.
Imported code has Please Behave's permissions and must be trusted; path confinement is
not a sandbox. Credential-bearing HTTP URLs are rejected and diagnostics never
echo Git stderr or secret repository selectors.

## EXT-002: Independent external providers

Consumer config.toml accepts `[[provider_imports]]` with `id`, `git`, `path` and
`ref`. Each selects one directory containing `provider.toml` and adapter resources
in an independently pinned Git source. The file uses the existing provider
configuration contract, without nested imports. Rules refer to the consumer's
explicit `id`; no provider imports are inferred from Rule references. Rulesets
and providers may use the same or different repositories, paths and revisions.
All named provider IDs remain global: local/imported collisions and repeated
IDs fail, including identical repeated imports. No overrides are inferred.
Existing Rule Set-distributed providers remain supported unchanged.

`please sync` resolves both import kinds through the shared immutable Git cache
and atomically publishes their combined resolution state only after validation.
Failures preserve prior state. Removal of the last import removes resolution
state. No provider code is copied into the consumer, executed or installed during
sync. Provider imports need no Rules, Rule Sets or execution points of their own.
Normal execution and plans use validated cached revisions offline; missing or
corrupt revisions block instead of downloading or substituting definitions.

Only independently imported providers interpret literal tokens in command, args
and requirements.executables: `@provider/` selects the provider directory,
`@source/` the imported repository root and `@repo/` the consumer root. `@@` escapes
one leading @. Unknown @ prefixes, absolute suffixes, traversal, missing resources
and symlink components fail validation, including for unused providers. Ordinary
tokens retain PATH/consumer-root semantics; no shell evaluation is introduced.
Local and legacy Rule Set-distributed provider arguments retain their literal
behavior. Runtime PATH/environment availability requirements still gate only selected
reviews after deterministic checks.

Imported provider provenance identifies repository hash, selected path/ref and
resolved commit in plans/reports, and participates in plan identity. Cache
integrity is rechecked before launching an imported adapter. Existing adapter
protocol, review evidence, blocking, timeout and output contracts remain intact.
Cache resource failures remain fatal for advisory reviews.
The product includes no provider or model by default. Imports execute trusted
code with the user's permissions; Git credentials and dependencies remain external.

Acceptance: `tests/provider_imports.rs::separate_provider_and_ruleset_repositories_run_offline`
covers independent sources, cached adapter execution and provenance. Further
fixtures cover shared sources, collisions, resource boundaries, atomic failure,
tampering and absence of implicit downloads/provider calls.

## CFG-001: Configuration and initialization

New projects use `.please/` exclusively; `.hannes/` is not discovered or migrated.
Acceptance: initialization and configuration fixtures use `.please/`; the product
binary is `please`. The repository itself uses the separately installed Please Behave release
pinned in `mise.toml` with `.please/` development configuration.

One unversioned configuration model uses lowercase .please/config.toml, optional
provider.toml, providers/*.toml, checks/*/rule.toml, reviews/*/rule.toml and rule-sets/*.toml.
There is no configuration version field. Unknown fields, duplicate keys/IDs,
invalid references and unsafe rule resources fail, including disabled Rules.
Missing check entrypoints or review prompts are execution errors only when
selected and enabled.
No recursive TOML discovery is supported. External sets explicitly select Rule
directories as defined in EXT-001; local definitions use only the paths above.
Supporting files inside rule directories are data, never governance documents.

Project settings are project, max_display_files (default 5), execution_points,
execution, imports and provider_imports. The README defines all fields and bounds. Review prompt filenames
must be single nonempty names directly in a Rule directory: no separators,
absolute paths, parent traversal, symlinks or special files. Review prompts are
UTF-8 Markdown without frontmatter requirements. Check entrypoints follow CHK-001.
Checks may bundle helpers.

init requires `.please/` to be absent. Any existing entry, including an empty
directory, regular file or symlink, fails with exit 2 and
`Please Behave: ERROR: Already initialized (.please/ exists).` before any files
or directories are created or modified. On success it prints
`Initialized .please/ (9 files created).` to stdout, using the actual created
file count, and exits 0. It installs no hooks.
It scaffolds a passing enabled Check, a disabled failing Check and disabled
product-consistency Review in self-contained directories. Reports remain local.
Initialization supplies no provider configuration or adapter. Configure an adapter
before enabling reviews.
Acceptance: `tests/acceptance.rs::init_exact_paths_and_repeated_initialization_rejected`
and `tests/acceptance.rs::init_rejects_existing_configuration_entries_without_writes`.

## HDR-001: Commit-header reviews

`review.input` selects `worktree` (default) or `commit-header`. Header reviews
require the validated managed `commit-msg` hook with exactly one regular,
nonsymlink message file. Only its first line is read: at most 4096 UTF-8 bytes,
excluding LF or CRLF, nonempty and without control characters. The body is never
read as review evidence. Direct runs and offline previews without hook context
are blocked when a header review is selected. Disabled or unselected header
reviews do not require a message file.

Header reviews reject file/context selections and file-change triggers; they
always omit the diff. Their adapter batches are separate from worktree reviews.
Only configured criteria and the captured header are supplied as review evidence,
without HEAD, file manifest, excerpts or diff. The provider-neutral adapter request
retains protocol version 1 and exposes `input.commit_header`. No model or provider
is built into the product. Header PASS/FAIL findings require
`source = "commit-header"`, `path = null`, `line = 1`; the generation schema
requires at least one finding, including for PASS. Adapter-generated technical
ERROR may still have no findings. Ordinary reviews retain
their existing file evidence contract and omit `source`.

The header is captured during planning, participates in plan identity and is
saved as `plan.input.commit_header` in reports. Recheck it after deterministic
checks, before and after each adapter call and before saving the result. Missing,
unreadable, invalid or changed headers prevent success; no stale verdict is
accepted. Changes confined to the body do not invalidate a header-only review.
Saved status evaluates working-tree freshness; the saved header is evidence of
that hook invocation, not a new candidate message. Existing checks-first gating,
blocking/advisory semantics and adapter failure handling apply unchanged.

Acceptance: `tests/commit_header.rs::commit_header_hook_and_adapter_contract`
covers real Git hooks, isolation, verdict evidence, gating, input bounds and
header stability using offline fake adapters.

## PROV-001: Named adapter configurations

All AI reviews use external executable adapters. No provider implementation or
implicit provider is supplied. Optional `.please/provider.toml` defines the project
default; `.please/providers/<id>.toml` defines named configurations. Review provider
references, imported definitions, conflict detection and offline resolution retain
their existing scope rules. An enabled selected review without a named or explicit
project default is blocked before execution. Disabled or omitted reviews need no
default; explicit unknown references remain invalid even when disabled.

Definitions require command and timeout_seconds (0 disables the deadline;
otherwise 1..=3600), with optional literal args, requirements.env,
requirements.executables and parameters tables. No shell expansion occurs. The
consumer root is the working directory. Availability requirements are evaluated
only after checks and immediately before invocation. Provider-specific fields
from the previous configuration model are rejected; migrate them into parameters.

## PROV-002: Adapter protocol

Protocol version 1 uses one UTF-8 JSON request on stdin and one JSON response on
stdout. Requests contain protocol_version, execution_point, input (prepared
excerpts, manifest, focused paths and bounded diff), head, prompt, response_schema,
and reviews with id, criteria, entry_files, context_files and effective parameters.
The common prompt/schema describe the verdict and evidence contract; adapters
translate them for their provider. Adapters own authentication, tools, models,
response extraction and other provider-specific behavior.

Provider parameters and review.parameters are arbitrary JSON-compatible TOML
tables. Merge top-level keys, with each review value replacing the provider value
at that key; nested tables and arrays are replaced whole. Please validates only
JSON compatibility and transports values without interpreting provider meaning.
Existing Rule parameter environments remain separate and are delivered as before.
Batching requires equal provider references, Rule parameter environments, input
sources and diff selection. Each review retains its own effective adapter parameters.

Responses require protocol_version = 1 and reviews matching the common verdict
contract. Optional usage contains common token counters; absent usage remains
unavailable. Each requested ID must occur exactly once with PASS, FAIL or ERROR.
PASS/FAIL require source-appropriate evidence: existing manifest paths and valid
line numbers for worktree reviews, or captured header evidence under HDR-001. Technical
adapter failures may exit nonzero; no verdict is accepted on nonzero exit.
Unsupported versions, unknown fields, malformed, incomplete, oversized or invalid
responses produce ERROR. Raw stdout/stderr and parser details are suppressed.
The shared process runner bounds output, drains pipes and terminates process
trees on timeout, including blocked stdin delivery. Nonblocking review errors
remain visible without blocking success; checks and integrity failures retain
their existing enforcement. Protocol changes require explicit version handling;
unknown versions never silently fall back.

Acceptance: `tests/cli_providers.rs` exercises the actual process boundary using
only deterministic provider-neutral adapters. No test requires AI credentials,
external services or AI token consumption. See [adapter protocol](docs/adapter-protocol.md)
for schemas, migration and a working dummy adapter.

## SEL-001: Selection and offline plans

`--at NAME` selects a known execution point, defaulting to pre-push. Selection
uses only centrally resolved effective assignments. Execution snapshots use the
worktree; HDR-001 adds captured invocation evidence for commit-header reviews.
Rules run once, checks first; results retain ID order within each kind.

`--plan` uses the same selection/preparation as execution. Its human-readable
output uses English labels and explanations, preserving configured values and
technical identifiers. It prints order, selection reasons, omissions, required
rules, input identity, declared provider settings and bounded context sizes,
without running checks, adapters or network
calls, changing project/Git state or replacing the report. `--plan --json` uses
plan schema 3 and structured diagnostics. Every JSON preview includes
plan_schema_version, status and blocking_reasons. Configuration or selection
errors return status blocked with nonempty blocking_reasons and exit 2; unavailable
plan metadata is omitted. Prepared plans retain their complete metadata, including
when blocked by an incomplete snapshot. Acceptance:
`tests/acceptance.rs::json_plan_errors_use_current_schema_and_blocking_reasons`.
A plan is neither a cache nor proof
that a later input is identical. Provider availability is not checked offline.

`--checks-only` runs checks only, without adapter processes or model calls. Reviews are excluded from progress/results and the limited scope is
recorded in plans and reports; terminal output warns only about actual omissions.
Full configuration validation still runs.
Every selected rule must execute unless dependencies or `--fail-fast` prevent scheduling; non-blocking FAIL and non-blocking provider/review ERROR are nonfatal.
Invalid review parameters and missing prompts remain fatal. A point requiring reviews rejects checks-only
execution before any check starts. Empty execution points succeed without proving
any rule PASS.

Checks and AI provider subprocesses receive `PLEASE_EXECUTION_POINT` from the
validated execution plan, including custom points and mapped imported assignments.
Please Behave overrides any inherited value. This is runtime context, not a configurable
rule parameter or TOML field; parameters, rule resources and Git hook arguments
retain their existing contracts.

## CHK-001: Deterministic checks

[check] requires exactly one of entrypoint or command, including for disabled
Rules. The preferred command syntax is a single string containing an executable
and arguments. After TOML decoding, spaces and tabs separate arguments outside
quotes. Single and double quotes group text and are removed; adjacent quoted and
unquoted segments concatenate, and empty quotes preserve an empty argument.
Backslash escapes the next character outside single quotes, including inside
double quotes; inside single quotes it is literal. Unclosed quotes, trailing
escapes and control characters other than tab are configuration errors.
Unquoted `|`, `&`, `;`, `<`, `>`, `(`, `)` and `#` are rejected. Unescaped `$` and
backticks are rejected outside single quotes, including inside double quotes.
Quoted or escaped special characters are literal arguments. There is no shell,
operator evaluation, pipeline, redirection, variable expansion, command chaining,
comment processing or globbing; wildcard and tilde characters remain literal.

The deprecated nonempty array syntax remains supported unchanged: executable
first, then literal arguments, without string-command parsing. The executable
must be nonblank and no element may contain NUL. Empty arguments are allowed.
The deprecated entrypoint and interpreter syntax remains supported unchanged.
No removal or runtime deprecation warning is introduced in this migration step.
Both command forms are mutually exclusive with entrypoint and interpreter.
Programs resolve through PATH or an explicit executable path, with the consumer
root as working directory. Please Behave performs no runtime installation.
Command availability is checked only during execution; missing executables
produce per-rule ERROR and offline plans never launch them. Parameters,
rule-directory context, timeouts, blocking and results use the same execution
path for commands and files. Review resolution is unchanged.

String-command tokens support exactly three explicit path prefixes after
tokenization: @rule/ uses the declaring Rule directory; @repo/ uses the consumer
repository root; @source/ uses the imported cached Git repository root, or the
consumer repository root for local Rules (the local ruleset source root, not
.please/ or a Rule Set directory). References may occupy the executable or any
argument position. Resolved native absolute paths remain single arguments,
including spaces; they are never tokenized again. Ordinary paths retain consumer
root semantics. No shell, expansion, new execution mechanism or working-directory
change is introduced.

A leading @ reserves a token for references; @@ removes exactly one leading @
and passes the rest literally. Quoting and escaping do not suppress references.
Tokens not beginning with @, including --file=@rule/data, are untouched. Arrays
retain literal @ and @@ tokens and never resolve references.

Reference suffixes use / on every platform and must consist of nonempty relative
components. Empty suffixes, empty components, trailing separators, ., ..,
backslashes and colons are configuration errors, including in disabled Rules.
Unknown prefixes and malformed reserved references are errors with supported
prefixes and literal-escape guidance. Before each execution, resolve using the
actual Rule provenance, reject all symlink components under the root and a
symlink root, enforce source containment and canonicalize the existing path.
This policy applies to all three prefixes. Missing/unsafe paths are per-rule
ERRORs without fallback. Files and directories are allowed as arguments; OS
executable lookup/permissions still apply. Plans validate syntax without
requiring existence. Existing filesystem race and trust boundaries remain;
references provide no sandbox.

Acceptance: `tests/configuration.rs::local_path_references_execute_with_consumer_working_directory`,
`tests/external_rule_sets.rs::imported_path_references_use_cached_provenance_offline`
and the offline resolver unit tests cover explicit roots, token boundaries,
spaces, executable/argument positions, literals, missing paths, symlinks and
cached shared scripts. Invalid syntax and legacy behavior retain the tests below.

Acceptance: `tests/configuration.rs::string_commands_parse_arguments_without_a_shell`,
`tests/configuration.rs::string_commands_reject_invalid_syntax`,
`tests/configuration.rs::command_representations_execute_equivalently`,
`tests/configuration.rs::commands_validate_without_execution` and
`tests/external_rule_sets.rs::imported_commands_plan_and_execute_offline`
cover quoting, escaping, invalid syntax, conflicts, legacy compatibility, offline
imports and equivalent execution. Existing process and entrypoint tests retain
coverage of timeouts, resource confinement and exit-code handling.

For file entrypoints, a filename such as index.sh resolves in the declaring
Rule directory. A relative path containing `/`, such as scripts/check.py, resolves
from the rule-source root: the consumer repository root for local Rules and the
imported repository root for Git Rules, regardless of the selected Rule Set path.
Resolution depends only on syntax; there is no fallback search. Paths use `/`;
empty paths, absolute paths, backslashes, drive prefixes, parent traversal,
symlinks in any component and nonregular files are rejected, including for disabled
Rules. Missing entrypoints remain per-rule execution ERRORs only when selected and
enabled. Source containment and regular-file checks run again before execution.
Review prompt resolution is unchanged.

Multiple Rules may use one entrypoint. Each retains its identity, parameters,
timeout, blocking behavior, execution and result; sharing never deduplicates
executions. PLEASE_RULE_DIRECTORY always identifies the declaring Rule directory,
including when the entrypoint lives elsewhere in the source. Cached Git imports
support the same resolution offline.

Acceptance: `tests/configuration.rs::local_rules_share_entrypoints_with_independent_results`
and `tests/external_rule_sets.rs::shared_entrypoints_execute_each_rule_offline`
cover shared scripts, independent parameters/results and declaring directories;
path confinement, execution-time replacement and missing/disabled cases are covered
in those test files alongside filename-only compatibility.

[check] accepts optional interpreter and timeout_seconds
(default 30, 1..=3600). Direct execution uses native OS behavior with no shebang
parsing or language detection. Explicit interpreters receive the absolute
entrypoint as one argument; no implicit shell, runtime installation or interpreter
argument parsing occurs. The consuming root stays the working directory and
PLEASE_RULE_DIRECTORY identifies bundled resources. Unix scripts may need an
installed interpreter on Windows; arbitrary scripts are not universally portable.

Exit 0 is PASS, 1 is FAIL and 2 explicitly reports an execution ERROR.
Unexpected numeric exit codes remain FAIL with an explicit diagnostic; signal
termination is ERROR. Wrappers map scanner infrastructure errors to 2. Spawn,
capture and timeout failures are always ERROR. Non-blocking FAIL remains visible and allows success
and subsequent Reviews. Check ERROR is fatal regardless of blocking; non-blocking review ERROR remains visible but permits success. Stdout/stderr
capture, process deadlines and cancellation remain shared in process.rs.

### Parallel check execution

Checks execute concurrently by default, with up to four running checks. Optional
`[execution] max_parallel` in `.please/config.toml` sets a positive integer limit;
`run --max-parallel N` overrides it. A limit of one preserves serial execution.
Concurrency changes execution strategy and does not affect semantic plan identity.

Optional `check.resource` is a nonempty, case-sensitive opaque string without
control characters or surrounding whitespace, validated even on disabled rules.
Equal names exclude simultaneous execution across local and imported checks in
one invocation only. Checks without a resource have no exclusion restrictions.
Checks sharing mutable state must declare the same resource. Waiting checks use
no execution slots, and later runnable checks may start ahead of them. All checks
finish before review gating; failures, timeouts and worker errors release resources
and do not stop remaining checks by default. Dispatch uses the history-based priority below among
runnable checks; saved results retain canonical order regardless of completion.
No resource list or build-tool-specific behavior is supported.

Interactive progress shows pending, resource-waiting, running and final rule
states, running elapsed times and completed/total and running counts. Visible
rule numbers are assigned in actual start-event order and retained on completion;
they are not plan positions or completion counts. Pending, waiting and never-started
NOT_RUN rules have no start number. Started rows precede unstarted rows and retain
start order, including members of shared review calls. Plain output uses the same
start numbers and reports completed/total and running counts after final events.
Canonical plan positions and saved result order remain unchanged. Refreshes are bounded;
short terminals show a bounded selection of rows and hidden-state counts. Plain
logs attribute waiting, starts and results without raw subprocess output. CI,
GitHub Actions and nonterminal streams never use cursor controls, including with
`--progress=tty`. Detailed captured output remains available through saved status.
Report schema 4 and plan schema 3 are unchanged.

Acceptance: `tests/parallel.rs` covers scheduling, configuration and report behavior;
`tests/progress.rs` covers interactive and append-only rendering.

### History-based scheduling

Runnable checks prefer the largest estimated remaining dependency path: their own
estimated duration plus the maximum path through their selected dependents. Equal
priorities use canonical ID order. Eligibility, resource locks and concurrency
remain authoritative; waiting work reserves no slots. Review batches remain serial
after checks and prefer the largest member estimate, retaining existing batching
and stable batch order for ties. Plans and reports retain canonical rule order;
history does not participate in semantic plan identity or offline previews.

Store one duration estimate per rule kind and exact stable ID (including import
binding namespaces) in Git's worktree-local `please-scheduling.json`. First PASS
sets the estimate; later PASS results average the previous estimate and duration
with equal weight, rounded down and bounded below by 1 ms. Other outcomes never
update history. Unknown rules use 1000 ms. Batched reviews use their measured
shared invocation duration per member. Updates occur after execution, independently
of aggregate verification success. Missing, invalid or unavailable cache data uses
fallbacks (cache size is bounded to 1 MiB); cache write failures never change verification outcomes. Writes replace
the cache atomically; concurrent invocations may lose updates. History is local,
non-versioned, never synchronized or persisted by CI integration, and can be reset
by deleting the file. No priorities, weights or durations are user-configurable.
The heuristic does not predict resource contention or guarantee an optimal plan;
rule edits retain estimates while their identity remains unchanged.

Acceptance: scheduler unit tests use supplied durations and channel-controlled
workers; `tests/scheduling.rs` covers persistence, identity, previews and cache
failure fallback with offline deterministic checks.

### Explicit check dependencies

A deterministic check may declare top-level `depends_on` (default absent, equivalent
to `[]`). Reviews cannot declare this property, even as an empty list, and cannot
be dependency targets. Dependencies use exact check IDs, never display names.
Imported bare references are qualified within their own import binding; consumer
references to imports require the full namespaced identity shown in JSON plans.
No short-name lookup across imports or implicit rule selection occurs.

Resolve dependencies against the enabled checks selected at the requested point,
after Rule Set assignments and import mappings. Reject missing or unselected targets,
ambiguous unqualified imported identities, self-dependencies and cycles before
any command starts. Disabled checks retain syntax validation but do not resolve
dependencies. Dependency order and duplicates do not affect semantic plan identity.
Plans record canonical dependency IDs on scheduled checks.

Only PASS satisfies a dependency, regardless of its blocking flag. Checks waiting
for dependencies or resources occupy no worker slot. A failed, errored, cancelled
or skipped prerequisite produces NOT_RUN for its dependents, with `blocked_by`
identifying the immediate prerequisite and a reason naming its outcome. Propagate
these skips transitively. Dependency skips do not independently change the aggregate
outcome; the originating failure's blocking status remains authoritative.
Independent checks continue unless explicit fail-fast stops scheduling. Fail-fast
skips without a completed blocking prerequisite retain the ordinary fail-fast reason.

Checks and reviews never overlap. Reviews start only after the check phase's
aggregate blocking outcome is successful. Advisory check failures and their
dependency skips permit reviews; blocking failures, execution errors, incomplete
fail-fast execution and snapshot failures prevent them. Uninvoked reviews are
NOT_RUN with an explicit check-phase gating reason.

Acceptance: `tests/dependencies.rs` covers validation, chains, branching, resource
conflicts, failure propagation, fail-fast, saved reports and check-gated reviews.

### Invocation-level fail-fast

`please run --fail-fast` stops scheduling after a blocking FAIL or fatal ERROR
becomes known. The default continues all independent work. Check ERROR is fatal
regardless of blocking; advisory review FAIL/ERROR does not stop scheduling.
Review parameter/prompt errors remain fatal. Already running checks finish normally
by default. Optional `[execution] fail_fast_terminate_running_instances = true`
requests termination of running checks after fail-fast triggers. This setting does
not enable fail-fast. `fail_fast_termination_grace_seconds` defaults to 5 and accepts
integers 1..=3600, even when termination is disabled. Both fields are optional and
excluded from semantic plan identity. No additional CLI flags are supported.
On supported macOS/Linux platforms, termination sends SIGTERM to each check's
process group, then SIGKILL after the grace period if processes remain, including
when the direct child has already exited. Deliberately detached descendants may
escape; this is not a process sandbox. Other platforms reject enabled termination
rather than silently skipping graceful termination.
Interrupted checks are `CANCELLED`, with an explicit reason, elapsed duration,
exit code when available and captured partial stdout/stderr. Checks completed
before cancellation retain their ordinary results. Cancellation never replaces
the triggering FAIL (exit 1) or ERROR (exit 2); without a triggering failure it
prevents success. Interrupted commands may leave incomplete builds or working-file
changes; Please Behave performs no rollback or repair. Final snapshot validation
still applies. Reviews in an invoked adapter batch finish normally,
and every completed result is saved.
Later review batches are not invoked after fail-fast triggers. No persistent
`fail_fast` configuration field is supported. Offline plans retain the full
selection because early termination depends on execution outcomes.

Unstarted selected rules retain their position and blocking metadata, with
`NOT_RUN`, no captured output, and an explicit fail-fast reason. Progress never
announces these rules as started. NOT_RUN does not replace a triggering FAIL
(exit 1) or ERROR (exit 2). Existing report and plan schemas are unchanged.

Acceptance: `tests/fail_fast.rs` covers termination configuration, graceful group
termination, escalation, partial output, saved cancellation and unchanged defaults
using offline subprocess fixtures, plus CLI defaults, serial stopping, errors,
advisory reviews, batch boundaries and saved reporting; scheduler unit tests use
channels to verify parallel completion and stopping without timing assumptions.

## PARAM-001: Rule parameters

Rules declare named `parameters` tables with `type` (`string` or `file`),
`required` (default false) and optional `description`. Local declarations provide
`value` directly. Imported declarations must omit `value`; consumers supply values
in that import's `rule_parameters.<rule-id>` table. There is no global value table.
Names use ASCII letters, digits and underscores, starting with a letter; their
uppercase forms must be unique within a rule. Values must be strings without NUL.

Before an enabled selected rule executes, validate required values, supported
types and undeclared supplied names. Optional omissions stay absent. Failures
produce rule-specific ERROR results identifying the parameter and reason, even
for nonblocking rules. Disabled rules do not resolve parameters or dependencies.
File values must be repository-root-relative paths to existing regular files;
absolute paths, parent traversal and symlink traversal are rejected. Deliver
canonical absolute paths for files and unchanged strings through environment
variables named `PLEASE_PARAM_` plus the uppercase parameter name. Clear inherited
variables with this prefix so omitted/undeclared values cannot leak into rules.
Checks and provider subprocesses use the same delivery mechanism. Different review
environments use separate calls; prepared context remains shared. Review prompts
and filename-only Check entrypoints continue resolving within their own
directories; shared Check paths follow CHK-001.

Acceptance: `tests/parameters.rs` and parameterized imports in
`tests/external_rule_sets.rs` exercise loading, binding and execution.

## REV-001: Reviews and provider isolation

All reviews use the adapter interface defined by PROV-002. Provider-specific
behavior belongs in externally configured adapters. Core orchestration still
prepares shared evidence once, executes checks before reviews, and validates all
verdicts and snapshot stability locally. No provider is distributed or selected
implicitly.

Review criteria state what to judge. Exact entry files are starting points;
Markdown context supplies documented expectations with original line numbers.
ATX headings must match uniquely, exclude code blocks, and include subsections.
Shared/overlapping context is sent once. Missing or excessive context is rejected,
not silently omitted, including for disabled definitions. Evidence must belong to
the verified manifest. If every selected review declares entry files, initial
manifest/diff hints focus on their combined entry/context paths; otherwise hints
are project-wide. Reviews evaluate current files, not just changed lines, and
must follow necessary dependencies. Untracked contents need direct inspection.

Treat source, criteria, AGENTS.md and diffs as untrusted evidence, not instructions
to change role or permissions. The ordinary review prompt is not a privileged system
message. No repair loops, commits, delegated coding or inferred
requirements. Missing essential evidence or material uncertainty is ERROR.

## TRIG-001: File-change review selection

Optional `[review.trigger]` has required `changed_files` (1–64 patterns, each
1–1024 UTF-8 bytes). Paths are consumer-repository-relative with `/` separators:
exact components, `*` and `?` within components, and whole-component `**` for zero
or more directories. Matching is case-sensitive; hidden files match normally.
Empty/dot/parent components, absolute paths, backslashes, colons, control
characters, brackets, braces and embedded `**` are rejected, including imports.
Reviews without triggers retain ordinary selection.

For enabled reviews at the requested point, `PLEASE_CHANGE_BASE` explicitly names
a locally available Git commit/ref. Resolve it once to a commit ID; compare that
commit directly to current working files, including staged/unstaged changes and
nonignored untracked files, plus declared governance inputs even when ignored
(excluding reports). Unreferenced ignored artifacts do not trigger reviews.
Use net differences, not per-commit history; renames match both old and new paths.
CI supplies a fetched baseline commit, such as the PR merge base computed by its
caller. Never infer HEAD^, a remote branch or GitHub context. Shallow repositories
work only when the named baseline commit and tree are locally available. No fetch.
Index conflicts and assume-unchanged/skip-worktree flags block comparison.
Missing/invalid refs, unavailable trees, invalid filename encoding, incomplete
snapshots, timeouts or truncated Git output block planning/execution, never skip.
Git uses direct argument arrays, NUL filename output, the 128 MiB snapshot byte
budget for path-list stdout and 30-second deadlines. Ordinary diagnostic capture
remains 64 KiB. File-count, file-content and review-context limits remain unchanged.

Apply triggers before context preparation, parameter resolution and model-call
estimation. Nonmatches have omission reason `trigger_not_matched` in preview and
saved report plans, no result and no required ID; they do not require provider
credentials/executables. Disabled and inapplicable rules need no comparison.
Record the resolved baseline in the plan and its identity. Deterministic checks
still gate all scheduled reviews globally. Acceptance: `tests/review_triggers.rs`.

## PREP-001: Shared review input

Planning prepares bounded requirement excerpts, manifest hints and orientation
diff once. Execution passes that same prepared input to the provider; prompt
rendering must not reread context or recalculate the diff. Snapshot stability
checks still run before the call and after execution; preparation is not a cache
or permission to run against changed files. The provider boundary owns transport
and provider settings, not a second interpretation of evidence selection.

## OUT-001: Structured review outcomes

Require exactly one verdict per requested ID and no unexpected IDs. Verdicts are
PASS, FAIL or ERROR. Each has a nonblank one-line summary of at most 120 characters
and at most 20 findings, each with a nonblank message of at most 2000 UTF-8 bytes.
Finding path and line keys are required but nullable. Worktree PASS/FAIL require
at least one existing in-manifest file and positive, in-range line reference. Grounded
references do not prove an LLM conclusion correct. Invalid types, unknown/duplicate
keys, malformed/missing/duplicate results and incomplete coverage make the batch
ERROR. Adapter stdout, including the envelope, is limited to 64 KiB. The shared
verdict validator additionally retains its 1 MiB document bound.

Provider schema, wire types and local validation share one response contract and
limits. The schema constrains requested IDs/count, verdicts, summary length/line
shape, finding count and positive line numbers before generation. UTF-8 byte
limits, exact unique ID coverage and real evidence remain local checks. Schema
compatibility errors are ERROR; constraints are never silently weakened.

Available token counters are summed across parameter batches, retaining unknown
counters as unavailable, including when the verdict is rejected. Total is input plus output; cached input, reasoning output
and cache writes are subsets. Missing telemetry remains unavailable, never fabricated as zero; invalid adapter
counters invalidate the response. No monetary estimate or total token budget is enforced.

## INP-001: Snapshots and stability

Input is Git HEAD plus a sorted manifest of current tracked and nonignored
untracked files, with SHA-256 hashes, sizes and executable flags. Also include
local governance configuration documents, rule prompts and entrypoints, review
entry/context files, and existing local files explicitly named by check commands,
provider commands/arguments or parameter values, even when ignored. Check path
references use their declared repository/source/rule boundaries. These inputs
include disabled definitions so enabling them cannot reuse stale evidence.
Do not recursively include ignored artifacts beneath `.please/`, such as Python
bytecode, or infer transitive script dependencies. Tracked files remain included
even when an ignore pattern matches. Always exclude `.git` and `.please/reports`.
No temporary commits, index mutations, content backups, external diff helpers or
optional Git locks.

Bound snapshots to 8 MiB/file, 128 MiB total, 100,000 paths, 128 MiB per Git path-list stdout
capture and 16 levels of configuration nesting. Unsupported symlinks, submodules,
special/non-UTF-8 paths, unreadable files or exceeded limits make coverage
incomplete. Incomplete input blocks execution and PASS. Compare input before an
active provider call and after execution; changed or unverifiable input blocks
PASS. Snapshots are not filesystem transactions and cannot detect edits reverted
entirely between captures.

Status is CURRENT for matching complete snapshots, STALE for differences, UNKNOWN
for incomplete/unavailable comparison. Staging alone does not change content;
HEAD changes do. Show bounded changed paths and exact totals only when coverage
allows. Do not invent line deltas: the baseline stores hashes, not prior content.
A working-tree PASS does not certify staged/pushed commits or deployed artifacts.
Acceptance: `tests/acceptance.rs::ignored_artifacts_do_not_change_verification_inputs`
and `tests/acceptance.rs::ignored_governance_inputs_remain_fingerprinted`.

## RPT-001: Reporting and CLI outcomes

Reports are written atomically to `.please/reports/last.json`, including FAIL/ERROR
when initialized input allows it. Invalid configuration preserves the prior
report. Save UTC timestamp, repository/project, provider, input identity, rule
results, durations, findings, output and available usage. Check ERROR and blocking
review ERROR, parameter errors and missing prompts take precedence over blocking FAIL; non-blocking review ERROR stays
visible and permits overall PASS. Fail-fast NOT_RUN and CANCELLED block success without replacing an existing blocking FAIL with ERROR.
Dependency NOT_RUN inherits the originating outcome and does not independently block reviews or success.
Disabled Rules stay out of results; blocking failed Checks leave selected Reviews NOT_RUN.

Report schema 4 records plan schema 3, required IDs, blocking and rule provenance.
An optional per-item configuration_error flag defaults to false in older reports
and identifies fatal review parameter or prompt errors independently of blocking.
Only the current report and plan schemas are accepted. Reject corrupt/inconsistent
reports and mismatched repositories. Reports are local, unsigned evidence, not
tamper-proof attestations. Status uses saved selection, not the current default.

Flush progress to stderr before each rule starts and after it finishes. Display
a short name, description, status and elapsed time. `--progress=auto|plain|tty`
controls verification progress (default auto). Auto redraws in an interactive
stderr terminal, except CI=true/1 or TERM=dumb; otherwise it appends plain lines.
Plain never emits cursor controls. Tty redraws rule rows, replacing their running
state on completion; descriptions must not wrap the row. Color is independent.
Announce shared provider calls without claiming individual model completion or
summing shared durations. Hide internal phases and successful process-exit reasons.
Normal execution uses English: `RUNNING`, `provider call` / `provider calls`,
and summary labels `rules`, `failed`, `errors`, `cancelled` and `not executed`.
Empty execution prints `No rules executed`.
Failures show actionable reasons and findings; raw captured output stays in details.
Execution ends with one overall status/count/duration line, without a duplicate
table, routine report path, timestamps or checks-only boilerplate. Integrity
failures remain visible even when no rule failed. Empty points explicitly say no
rules ran and do not claim rule PASS. Saved status lists results with display
names; details retain IDs, provenance, output and usage. No rules defined prints
`No rules defined yet.`; disabled definitions do not
trigger that message. Acceptance: tests/progress.rs covers names, plain/TTY
rendering, concise execution and actionable errors.

Saved status and `--details` use English labels and messages, including selection,
checks-only scope, provenance and token usage. Use `Selection`, `Omitted`,
`Required rules` and `Mode: Checks only` consistently with execution plans.
This presentation does not change saved data, freshness, streams or exit codes.
Acceptance: tests/acceptance.rs covers saved scope and token usage rendering.

Respect `NO_COLOR`, `--no-color` and each stream's terminal state. Status colors
are green PASS/CURRENT, red FAIL and amber ERROR/STALE/UNKNOWN; help uses cyan
accents and bold headings. Sanitize untrusted terminal content.

Exit codes: 0 success; 1 failed verification or STALE; 2 configuration, execution,
provider/report error, UNKNOWN or invalid arguments; 3 absent report for status.
An executable plan exits 0; a blocked/invalid plan exits 2.

## REL-001: Distribution and integrations

The public GitHub Action is maintained independently in
[please-behave/action](https://github.com/please-behave/action). Its repository
owns the Action implementation, inputs, supported runners and acceptance tests.
The Action revision and selected CLI release are independent. CI and native release jobs use
Action v1.2.0 pinned to `722d5967c094e5ac4bfe25e9f82b020ff34e9487`, with
the CLI version read from `mise.toml`. Installation-only mode precedes
explicit synchronization; the published Action then executes `check`, including
applicable reviews, with Job Summaries enabled. CI retains the CLI report and
Action Summary; local wrapper runs also retain the companion execution record.
Release readiness and native candidate jobs execute through the same Action with
`checks-only` and Job Summaries enabled, preserving their independent gates.
The Action supports Ubuntu and macOS runners.

Supported hosted CI/release targets are macOS Apple Silicon, macOS Intel and
Linux x64 and ARM64 (GNU/glibc); Windows builds/releases are excluded. Linux ARM64
uses the native `ubuntu-24.04-arm` runner and `aarch64-unknown-linux-gnu` target,
with the same formatting, lint, tests, build and executable smoke gates. Build with stable
Rust/Cargo (declared minimum 1.85) into a native executable. Git and configured
external tools remain runtime dependencies; no Python/Node runtime is bundled.

`mise deploy` installs the Please Behave development governor pinned in
`mise.toml`. It downloads that exact published GitHub release for
the detected OS and CPU (the four targets above), verifies SHA-256 and executable
compatibility,
and installs to `${CARGO_INSTALL_ROOT:-$HOME/.local}/bin/please`. It never builds
local source. Unsupported platforms, missing downloads, invalid checksums and
incompatible binaries fail with an actionable error and preserve the installed binary.

Push/PR CI runs the Linux x64 native gates; release runs all four native hosts.
CI runs formatting, Clippy, offline unit/acceptance tests, native smoke tests,
minimum-Rust checks, dependency audits and release-tooling/workflow checks.
Releases are manually dispatched on main and retain independent quality gates.
Semantic Release plans versions from the existing release tags and publishes only
after artifact/source verification. During 0.x, breaking
changes/features increment minor and fixes/maintenance increment patch. Stable
1.0.0 requires explicit policy graduation. Publish native archives with README,
LICENSE and checksums. Artifacts must match the tested
version/source. Normal CI is read-only; only release planning/publishing may
request write permission. Actions are pinned. See [release policy](docs/releasing.md).

Please Behave packaging uses `please-vVERSION-TARGET.tar.gz`, containing a
matching directory with `please`, README and LICENSE, plus release SHA256SUMS.
Workflows use matching `please-*` artifact labels to transfer candidate archives
and metadata. The four native CLI assets are published for v0.1.0.

Repository self-integration uses an installed, previously published Please Behave
binary outside the checkout; local candidate builds never govern the source being
checked. `mise.toml` is the sole exact governor pin for local deployment and
Ubuntu CI. Native release builds verify candidates independently on all four
platforms. Renovate proposes updates without automerge.
Pinned installation downloads the exact release's native archive and SHA256SUMS
directly, without a GitHub Releases API lookup or fallback to latest. Only latest
discovery uses the API, with optional GITHUB_TOKEN Bearer authentication; asset
requests carry no token. Missing releases/assets, network/API failures and missing,
malformed or mismatched checksums fail installation and preserve the installed copy.
Acceptance: `scripts/test_deploy.py` covers direct pinned URLs, optional API
authentication, safe diagnostics and installation failures with mocked downloads.
The governor uses the exact published release in mise.toml. Repository rules use the unversioned,
self-contained Rule model and fixed Git-hook execution points.

The three Git-backed built-in points serve Git hooks. Normal CI uses one check
point for formatting, documentation, tooling, native validation, minimum Rust,
dependency audits and security. Local Rules declare their execution points
directly, without local Rule Sets. Pre-commit runs formatting and documentation
validation; pre-push runs the complete local suite, keeping all Rust tests in
one Rule. Python helper tests use one unittest discovery Rule and JavaScript
release tests use one npm Rule. All three test suites, workflow validation and
minimum-Rust checks run at pre-push and again in CI. The repository has 14 local
Checks and one conditional document Review; dependency audits run at check with advisory access. Agents use Git
commit/push feedback instead of manually running checks,
tests, linters or builds. They may read saved reports without rerunning Rules.
The manual mise check task delegates to published Please Behave at pre-push.
All three external Rule Set imports and the independent Gemini Flash-Lite provider
import pin the same reviewed full Git commit.
The three existing imports preserve their execution-point and parameter semantics.
Release workflow jobs own planning, exact-source CI prerequisites, candidate
builds, artifact transport, provenance, integrity and publication. The only custom
execution point is `release`: an independent local release-smoke Rule tests CLI
behavior against an already-built native candidate. It has no Rule dependencies.
GitHub Actions builds first, captures the binary hash, invokes that quality gate,
then checks the unchanged hash and records a smoke receipt. Packaging, archive
verification, complete-set evidence and the final source/version publication
guard run directly as workflow scripts, outside Please Behave.
All four supported native targets and existing archive/receipt guarantees remain.
Source checks, unit tests and audits are reused from exact-source CI, including
successful partial CI reruns; incomplete or untrusted evidence blocks publication.
Release artifacts must belong to the current workflow run and attempt.
The published governor remains pinned in mise.toml and separate from candidates.
Acceptance: `scripts/test_release.py::test_release_job_contract_permissions_and_selection`.

The repository-conventions set (rulesets/multi-point/repository-conventions) is imported at a full Git commit
pin for pre-commit, check and, through reference overrides, commit-msg. Its agents-size Check limits AGENTS.md
to 1500 approximate tokens at pre-commit and check; its scopes_file parameter uses .please/commit-scopes.txt. The external Check
validates syntax, types and scopes; an advisory Gemini Flash-Lite Review evaluates
only the header for specific English imperative wording, without the diff. Its
provider is imported independently from providers/gemini-flash-lite. The hook
inherits GEMINI_API_KEY; missing credentials and provider errors remain visible
as advisory ERROR. No local commit policy, commit-range or PR-title check remains.
The check and pre-push points also import rulesets/multi-point/security at a full commit
pin. Its sensitive-files and secret-scan checks inspect the working tree at check
and HEAD contents at pre-push, without history. The CI quality job explicitly
runs check with Python 3 and a checksum-verified Gitleaks binary available. Rules
and Please Behave never install these dependencies automatically. The same points import
rulesets/multi-point/open-source-software at a full Git commit pin. It currently
contains only the blocking license Rule, with target = "LICENSE". The license
must match an SPDX reference byte-for-byte: Apache-2.0,
PolyForm-Noncommercial-1.0.0 or Unlicense. This repository uses the exact
PolyForm-Noncommercial-1.0.0 reference text.
The Rule reads the working tree at check and HEAD at pre-push. Missing or
nonmatching licenses produce FAIL; technical errors produce ERROR.
Checks remain mandatory. The agents-quality Review runs at check only when its
file trigger matches. Deterministic errors block success; this advisory Review
keeps FAIL and ERROR visible without blocking. Setup, dependency installation and release publication remain
outside rule execution.

Hooks are managed by please sync at Git's effective hook path, including forwarding
commit-message arguments and preserving compatible unmanaged content. A setup
shim delegates to please sync without changing core.hooksPath; custom hook paths
are preserved.
Check subprocesses retain the inherited environment and recursion guard. Local
Rules use string commands for installed tools and consumer-root Python script
paths, without per-rule shell or Python launchers. Cargo build/test/Clippy/MSRV
commands and npm tests explicitly select offline mode. The execution-point wrapper additionally sets Cargo/npm
offline mode and disables Rustup auto-install for Git-backed invocations.
CI invokes please sync --yes once before execution. Execution never repairs
hooks noninteractively. Publish installs no governor or managed hooks and runs no
governance checks. It requires successful readiness and native candidate jobs, then verifies the
complete artifact set and checksums through the direct workflow step. The
publication guard additionally checks the planned version and tested source SHA
before tagging. Native builds stamp Cargo.toml/Cargo.lock only in their disposable
checkouts before testing and packaging; committed versions remain unchanged.
Semantic Release creates tags, Git notes, GitHub Releases and release notes,
without generating a changelog or release commit. Release tags target the tested
source revision. Publication refuses an advanced main branch or invalid artifacts.
Reports use schema 4. Local wrapper invocations also write a companion execution
record with governor version/digest, execution point, checked commit and exit code.
CI uses the published Action Summary and CLI report instead of that record.
See [repository self-integration](docs/self-governance.md) for coverage and setup.

Git hooks required by enabled effective assignments are derived automatically
and reconciled only after explicit confirmation (or sync --yes). A bypassable
local hook verifies working files, not necessarily the pushed commit. Hosted
provider authentication is not provisioned and personal tokens are not copied to CI.
No hosted service, database, autonomous development, universal policy engine,
result cache or additional input/provider implementation is in current scope.

## REC-001: Declarative reconciliation and Git hooks

Successful `sync` prints `Synchronization completed successfully.` to stdout when
import resolution state, cached repositories/revisions or Git hooks changed;
otherwise it prints `Already synchronized.`. Refreshing unchanged imports alone
does not count as a change. Both outcomes exit 0 and execute no checks or reviews.
Hook plans use `Planned changes:` on stderr, retaining their individual operation
descriptions and `Apply these changes? [y/N]` prompt. `--yes` skips the prompt
without printing an auto-approval message. Import refresh still precedes hook
approval and is not rolled back when approval is declined.

Configuration validation, desired-state reconciliation and quality-check execution
are separate responsibilities. `sync` and direct execution of Git-backed
points through `run` use Validate -> Inspect -> Plan -> Confirm -> Apply ->
Verify before any checks. Direct execution of `check` and custom points validates
and resolves Rules, then executes without inspecting or reconciling Git hooks.
Missing, outdated or unsafe hook paths do not block these explicit points; they
never prompt for hook synchronization or modify hook files. Repository recursion
protection still applies. Managed hooks accept only their identically named
built-in execution point.
No discrepancy means no prompt or write. Rejection, unavailable terminal, conflict
or failure exits 2 and preserves the previous report. `--plan` remains read-only
and does not reconcile. No provider is called by reconciliation.

`please sync --yes` explicitly approves the displayed reconciliation
plan without reading stdin or requiring a terminal. It is scoped to sync and
defaults to false. Validation, conflict detection, recursion protection, apply-time
state checks, rollback and final verification still apply. It creates, updates or
removes only declared managed integration state, never governed project files,
and executes no checks or reviews. Repeating it on synchronized state performs no
writes. Without the flag, noninteractive drift still exits 2. CI can run this
command once before Git-backed execution; explicit points such as `check` need
no hook setup in CI. Execution itself has no auto-approval.
Acceptance: `tests/reconciliation.rs::auto_approve_reconciles_noninteractively_without_executing_rules`.

Please Behave supports exactly three Git hooks: pre-commit, commit-msg and pre-push.
Each invokes its identically named built-in execution point. This relationship is
fixed and cannot be configured. Enabled effective assignments determine required
hooks after Rule Set inheritance and external imports resolve. Removing those
assignments retires the corresponding managed hook content.
Project configuration accepts no hooks section; obsolete hook declarations fail
as unknown fields, with the configuration path and supported fields in the error.
check and custom execution points have no Git hook and are invoked explicitly.
External import mappings still apply only to custom points and cannot map built-ins.
check has no workflow discovery or CI validation; consumers choose where to invoke
it, including multiple workflows. Please Behave generates no CI workflows.
Acceptance: tests/reconciliation.rs covers fixed event invocation, automatic
installation, removal, obsolete configuration rejection and explicit-point isolation;
tests/external_rule_sets.rs covers imported hook Rules and per-import mappings.

Git resolves the effective hook path, including worktrees and `core.hooksPath`.
Default common-repository hooks are shared by its worktrees; custom paths must
remain within this working tree. Symlink components, nonregular files, malformed
or duplicate ownership markers and foreign repository ownership are conflicts.
No Git configuration is changed. Only the three supported event files are inspected.

Managed sections have explicit markers and repository ownership. New hooks use
POSIX sh. Existing executable POSIX sh hooks can be composed by inserting a
managed section immediately after the shebang, preserving the remaining bytes
and checking Please Behave failure before existing commands. Other layouts require manual
integration. Removal preserves unmanaged bytes and permissions; whole-file deletion
requires exact ownership of the wrapper. Plans are deterministic and revalidated
before writes. Files are staged, synced and replaced individually, with best-effort
rollback on failure; a multi-file operation is not an atomic transaction. An
interrupted process may leave drift; rerun `sync` and inspect the reported paths.

Generated wrappers pass explicit event, wrapper path and original arguments to
`run`. Please Behave validates this context against the repository, identically
named execution point and hook state, verifies reconciliation read-only and never prompts or edits hooks. Drift
instructs the user to run `please sync` manually. Nested execution from checks
is rejected using a repository-scoped guard, including checks started by direct
execution of Git-backed, check and custom points. The guard also blocks nested
reconciliation in that repository. Acceptance:
`tests/reconciliation.rs::direct_execution_blocks_nested_execution_and_reconciliation`.
Checks receive `PLEASE_GIT_HOOK`,
`PLEASE_GIT_HOOK_ARG_COUNT` and one-based `PLEASE_GIT_HOOK_ARG_1`, etc.; a dedicated
checker script reads these without shell expansion by Please Behave. `commit-msg` supplies
the message file in argument 1, unlike pre-commit; no commit policy is hardcoded.
Pre-push stdin is not forwarded to checks. All execution points still verify working files,
not the exact staged index or pushed commit.

## ARC-001: Shared configuration contract

Defaults, resource bounds and the configuration directory have one source in
Rust. Initialization serializes the actual configuration types rather than
maintaining independent TOML templates. README defaults, limits and scaffold
examples are checked against those values. Validation, planning and execution
reuse the same bounds; this does not add configurable fields or change defaults.

## DOC-001: Documentation contract

Every editable configuration field, required/optional marker and default must
match the README reference. README configuration examples must validate and plan
offline. Local links/anchors and specification acceptance references must resolve.
Repository Git hooks run link/specification checks at pre-commit and pre-push;
pre-push also runs the Rust documentation acceptance tests.
CI runs them through check. The manual `mise docs:check` and `mise check` tasks
remain available to developers; agents rely on Git hooks. Automated checks
complement editorial review; they do not prove prose understandable.

## Acceptance coverage and change process

Each essential requirement has a stable ID and representative acceptance tests.
The table is a navigation aid, not a claim that one test proves every clause.
Add or update the contract and acceptance cases before implementing behavior;
update the README/examples and relevant design decisions in the same coherent
change. Keep one coherent configuration model and run applicable checks. Follow the user's Git instructions.
Public documentation explains supported behavior; proposals remain in the roadmap.

| Requirement | Representative acceptance test |
| --- | --- |
| TRIG-001 | `tests/review_triggers.rs::preview_and_execution_agree_and_nonmatches_need_no_provider_or_context` |
| PROV-002 | `tests/cli_providers.rs::stdin_results_and_provider_isolation` |
| HDR-001 | `tests/commit_header.rs::commit_header_hook_and_adapter_contract` |
| PROV-001 | `tests/named_providers.rs::named_references_load_without_changing_legacy_defaults` |
| EXT-002 | `tests/provider_imports.rs::separate_provider_and_ruleset_repositories_run_offline` |
| EXT-001 | `tests/external_rule_sets.rs::shared_rules_are_reused_across_sets_and_points_offline` |
| RULE-001 | `tests/execution_points.rs::resolution_obeys_membership_override_union_and_disabled_rules` |
| REC-001 | `tests/reconciliation.rs::approval_updates_removals_and_idempotency` |
| ARC-001 | `tests/documentation.rs::documented_adapter_configuration_is_valid` |
| DOC-001 | `tests/documentation.rs::every_configuration_field_required_marker_and_default_is_documented` |
| CLI-001 | `tests/acceptance.rs::all_command_paths_and_standard_help` |
| PARAM-001 | `tests/parameters.rs::local_parameters_deliver_strings_and_consumer_files_without_changing_rule_resources` |
| CFG-001 | `tests/configuration.rs::strict_configuration_and_implementation_validation` |
| SEL-001 | `tests/configuration.rs::checks_only_requires_complete_selection` |
| CHK-001 | `tests/configuration.rs::direct_and_interpreter_execution_keep_consumer_and_resources` |
| REV-001 | `tests/cli_providers.rs::stdin_results_and_provider_isolation` |
| PREP-001 | `tests/configuration.rs::prepared_input_is_shared_without_rereading_resources` |
| OUT-001 | `tests/review_contract.rs::review_contract_enforces_boundaries_and_semantic_evidence_locally` |
| INP-001 | `tests/acceptance.rs::changing_files_during_checks_blocks_pass` |
| RPT-001 | `tests/configuration.rs::saved_reports_reject_inconsistent_metadata` |
| REL-001 | `scripts/test_release.py::test_only_root_version_changes_and_invalid_versions_leave_files_untouched` |

## Runtime namespace isolation

Product-owned runtime variables use `PLEASE_*`, including parameter, Rule
directory, execution-point, change-base, hook context and recursion guard.
Hannes variables are not aliases or fallback inputs.
Third-party credentials and standard tool variables keep their upstream names.
The product uses `.please/` and its own cache and managed-hook markers; it never
loads `.hannes/`. Repository-owned adapters live under `.please/adapters/` and belong
to development governance, not the product.

Rule and provider IDs reserve `please-`. Generated integrity results use
`please-plan`, `please-snapshot`, `please-tree-stability` and
`please-commit-header-stability`, with report schema 4
and existing execution semantics unchanged. Public library phase callbacks use
English messages. Acceptance: `tests/runtime_namespace.rs` covers namespace
isolation, reserved IDs, phase messages and integrity report IDs.
