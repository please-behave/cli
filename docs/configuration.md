# Please Behave configuration

Use `please init` to create `.please/`, `please sync` to validate and synchronize
it, `please run` to execute rules and `please status` to inspect saved results.
`please help` lists the English commands and options. This guide is generated as
`.please/README.md` during initialization.

Please Behave has three governance concepts: a **Rule** defines an expectation, a
**Rule Set** groups rules and assigns them to **Execution Points**. Checks and
Reviews are the two rule implementations. An import references a source Rule Set.

## Configuration

### Files and validation

Use this fixed layout at the Git working-tree root:

```text
.please/
  config.toml
  provider.toml
  providers/
    quality.toml
  checks/readme/
    rule.toml
    index.sh
  reviews/architecture/
    rule.toml
    prompt.md
  rule-sets/repository.toml
  reports/last.json
```

Start with `config.toml` and add Rule, Rule Set and provider documents as needed.
Supporting files belong beside the Rules that use them. The binding
[Configuration Discovery and Validation contract](../knowledge/contracts/configuration/discovery.md)
defines the recognized locations, safe paths, document limits and validation rules.

Run `please sync` after editing. Validation runs no checks or providers.
Required Git hooks are derived from effective Rule assignments and reconciled
after confirmation; use `--yes` for unattended reconciliation. New projects initialized by `please init` include
one enabled passing check, one disabled failing check and one disabled review.
Initialization requires `.please/` to be absent and installs no hooks.

### Project settings

`.please/config.toml`:

```toml
project = "My_App"

[execution]
max_parallel = 4
fail_fast_terminate_running_instances = false
fail_fast_termination_grace_seconds = 5

[execution_points]
custom = ["release"]
```

The [Project Configuration contract](../knowledge/contracts/configuration/project.md)
explains every project option. Its [schema](../knowledge/contracts/configuration/project.schema.json)
defines fields, defaults and bounds; the TOML format has no version field.

Use `execution.max_parallel` to cap concurrent checks, or `--max-parallel` for one
invocation. Set the limit to one when serial execution is useful. Enable
`execution.fail_fast_terminate_running_instances` only when interrupted checks can
leave their work in an acceptable state; it takes effect with `run --fail-fast`.

With `run --fail-fast` and an effective limit of at least two, one slot always
picks the runnable check with the shortest estimated duration; the other slots
prioritize the longest remaining dependency path. The fast lane accepts any
duration and never holds capacity idle. Ties use canonical IDs. Estimates come
from local history, with a uniform fallback for checks without history, and
never override dependencies, resource locks or the concurrency limit. Complete
runs and serial runs keep remaining-path priority; reviews are unaffected.
There is no fast-lane setting.

Declare additional execution points under `execution_points.custom`. Invoke
`check` and custom points explicitly, including in CI; they need no Git hooks.
Use `context` settings to control stored run contexts and section sizes, and
`max_display_files` to limit changed paths shown by status. Add external sources
through [Rule Set imports](#import-rule-sets-from-git) and
[provider imports](#import-providers-from-git).

Checks sharing mutable state must declare the same `resource` inside `[check]`:

```toml
[check]
command = "cargo test --locked"
resource = "cargo"
```

Resource names are exact, case-sensitive opaque identifiers. They coordinate
local and imported checks within one invocation, without namespacing by import.
They do not lock other Please invocations. Resource-waiting checks occupy no
execution slots; later independent checks can start first. Failures and timeouts
do not stop remaining checks. Results retain canonical ID order. Existing rule
files remain valid, but checks relying on serial execution need resources or a
limit of 1. Older CLI releases reject these new configuration fields; upgrade
pinned consumers before adding them to shared rulesets.

The [execution settings contract](../knowledge/contracts/configuration/project.md#execution-settings)
defines fail-fast termination, grace periods and platform requirements. Inspect
partial output after cancellation and clean up interrupted builds before retrying.

### Fields shared by checks and reviews

The binding [Rule Definition contract](../knowledge/contracts/configuration/rule.md)
and its schema define shared fields, identity, labels, enabled state, enforcement
and execution-point assignment. [Rule Parameters](../knowledge/contracts/configuration/parameters.md)
defines declared inputs and their validation and delivery.

Checks can declare top-level prerequisites independently of execution points:

```toml
id = "smoke"
description = "Smoke test after a successful build"
run_at = ["check"]
depends_on = ["build"]

[check]
command = "./scripts/smoke.sh"
```

Explicitly select an enabled `build` check at the same point. Dependencies never
select extra rules. Use exact IDs, not names: imported bare references stay within
that import binding; consumer references to imports require the full namespaced
ID from a JSON plan. Cross-import short-name lookup is unsupported. Missing,
unselected, ambiguous, self or cyclic dependencies fail before execution. Disabled
checks validate dependency syntax without resolving targets. Duplicate entries
are ignored and ordering does not matter. AI reviews cannot declare dependencies
(even `[]`) or serve as prerequisites.

Only PASS satisfies a prerequisite. Dependency waits
use no execution slots; resource exclusions remain in force. Failed, errored or
skipped prerequisites propagate `NOT_RUN` transitively, with `blocked_by` and a
reason identifying the immediate prerequisite and outcome. Independent checks
continue by default. Dependency skips inherit the originating failure's aggregate
effect: check failures, execution errors and incomplete fail-fast execution
prevent reviews. Check and
review phases never overlap. Gated reviews report `NOT_RUN` with a check-phase reason.

### Rule parameters

Checks and reviews use the same parameters. Declare each parameter in its rule's
`rule.toml`. For a local commit-message check, for example:

```toml
id = "commit-message"
description = "Validate Conventional Commits"
run_at = ["commit-msg"]

[check]
command = "sh @rule/index.sh @param/scopes_file"

[parameters.scopes_file]
type = "file"
required = true
description = "Allowed commit scopes"
value = ".commit-scopes"
```

The [Rule Parameters contract](../knowledge/contracts/configuration/parameters.md)
and its schema define declaration fields, local and imported binding, resolution
and delivery. This is separate from provider/review adapter settings.

### Add a check

Create `.please/checks/readme/rule.toml`:

```toml
id = "repository-readme"
description = "Require a nonempty README"
run_at = ["pre-push", "check"]

[check]
command = "sh @rule/index.sh"
```

Create `.please/checks/readme/index.sh`:

```sh
#!/bin/sh
test -s README.md
```

A command is one string: an executable and its arguments, run directly without a
shell. For an installed tool, name it directly:

```toml
[check]
command = "cargo +stable test --locked"
timeout_seconds = 600
```

| Field | Required / default | Meaning |
| --- | --- | --- |
| `check.command` | Required | String containing the executable and its arguments. |
| `depends_on` | Optional; `null` (equivalent to `[]` for checks) | Top-level exact prerequisite check IDs. Checks only; forbidden on reviews. |
| `check.resource` | Optional; `null` | Exclusive, case-sensitive resource name; nonempty, no control characters or surrounding whitespace. |
| `check.timeout_seconds` | Optional; `30` | Integer `1..3600`; deadline including output capture. |

The first argument is an executable name resolved through `PATH`, or an explicit
executable path. Relative command paths use the consuming root, including for imported
commands. On Unix, a script run directly needs executable permission; `sh script.sh`
does not. Please Behave does not infer languages or install runtimes.

#### Rule inputs

A rule receives its inputs only as arguments, through references that are resolved
after tokenization, in the executable position or in any argument:

| Token | Resolves to |
| --- | --- |
| `@rule/<path>` | A path inside the directory containing this Rule's `rule.toml`. |
| `@repo/<path>` | A path inside the consuming repository. |
| `@source/<path>` | A path inside the rule source root: the consuming repository for local Rules (not `.please/` or a Rule Set directory), or the cached imported Git repository. |
| `@param/<name>` | The value of a declared [rule parameter](#rule-parameters); a `file` parameter becomes an absolute path. |
| `@context` | The absolute path of the [run context](#run-context) file. |

```toml
[check]
command = "python3 @rule/check.py @context @param/scopes_file @repo/pyproject.toml"
```

A reference is recognized only as a whole token (`--file=@repo/data` stays literal).
Quote a token containing spaces, such as `"@rule/my script.py"`; its resolved path
stays one argument and is never tokenized again. `@` at the start of a token is
reserved: write `@@` for one literal leading `@`. Quoting or escaping an `@` does not
disable resolution.

Path suffixes use nonempty relative components separated by `/`; empty suffixes, `.`,
`..`, backslashes and colons are rejected when loading, as are unknown prefixes, a bare
`@`, a malformed `@param/` name or an `@param/` reference to an undeclared parameter.
Errors for unknown prefixes, malformed reserved references, malformed parameter
names and undeclared parameters list the supported tokens and the `@@` literal escape,
including for disabled checks.
Immediately before execution, referenced paths must exist, symlinks anywhere in the
root or suffix are rejected, and the canonical path is passed on. A missing path, a
missing `@context` or a parameter without a value is a per-rule ERROR with no fallback.
Offline plans validate syntax without requiring paths to exist or launching commands.

**Rules receive no environment from Please Behave.** Every inherited `PLEASE_*`
variable is removed before a check or adapter starts, and none is set, so nothing leaks
into the tools a script starts. A script that needs an input for a subprocess passes it
on explicitly. The only exception is a reserved internal variable that stops a
`please` started inside a rule from running recursively; never read or set it.

String commands use this grammar **after TOML decoding**:

- Spaces and tabs separate arguments outside quotes. Single or double quotes
  group text and are removed. Adjacent segments concatenate (`pre"two words"post`
  becomes one argument); `''` or `""` preserves an empty argument.
- Backslash escapes the next character outside single quotes, including inside
  double quotes. Inside single quotes, backslash is literal. Unclosed quotes,
  trailing backslashes and control characters other than tab are rejected.
- Unquoted `|`, `&`, `;`, `<`, `>`, `(`, `)` and `#` are rejected. Unescaped `$`
  and backticks are rejected outside single quotes, including in double quotes.
  Quote or escape these characters when they are literal argument data.
- No shell is invoked: there are no pipelines, redirection, variable expansion,
  command chaining, comments or globbing. Wildcards and `~` are literal.

TOML literal strings avoid a second layer of backslash escaping:

```toml
[check]
command = '"tools/my checker" "path with spaces" "" path\ with\ spaces'
```

This passes three arguments to `tools/my checker`: `path with spaces`, an empty
string, and `path with spaces`.

Several Rules can run one script, for example
`command = "python3 @source/scripts/check-files.py @rule/config.json"`, and each still
executes separately with its own parameters, timeout, blocking behavior and result.
Git imports support shared scripts offline from their resolved cache. Missing tools
produce per-rule execution ERRORs; validation and offline plans do not launch commands.

Checks receive no stdin and keep the consuming project's root as their working
directory.

Exit **0 = PASS**, **1 = FAIL**, every other status = **ERROR**. FAIL is a verdict
about the checked subject; a script's own technical failure must exit with any other
status, conventionally 2. Interpreters report an uncaught exception with their default
code (Python traceback 1), which reads as FAIL, so wrap the entry point in a catch-all
that exits 2. The ERROR diagnostic names the exit status. Signal termination,
process-launch, capture and timeout failures are ERROR. Wrap external tools to
translate their own exit-code contracts: a linter that reports violations with 101 or a
scanner that uses 2 for a violation must be mapped to 1, and its actual execution
problems to 2. Please Behave does not parse shell error messages to classify startup
failures. A check FAIL or ERROR prevents reviews and overall success.

Resource confinement is not process sandboxing. Checks execute with Please Behave's
permissions and can access files, credentials and the network. Only import
executable checks from [trusted repositories](external-rule-sets.md#trusting-repositories). Changes to evaluated files during
execution prevent PASS.

### Result statuses

| Status | Meaning | Assigned by | Effect on success |
| --- | --- | --- | --- |
| `PASS` | The rule ran and its expectation holds. | Check exit 0 or review verdict | None |
| `FAIL` | The rule ran and found a violation. | Check exit 1 or review verdict | Always blocks for a check; blocks for a review only when the review is blocking |
| `ERROR` | The rule could not complete or produce a trustworthy result, such as a crash, missing tool, timeout or invalid provider response. | Any other check exit status or Please Behave | A check ERROR always blocks; a review ERROR blocks only when the review is blocking |
| `NOT_RUN` | The selected rule was not executed because of a failed prerequisite, fail-fast or check-phase gating. | Please Behave | A dependency skip inherits the originating outcome; fail-fast and gating skips follow the originating failure |
| `CANCELLED` | A running check was interrupted by fail-fast termination. | Please Behave | Blocks success |

### Configure the AI provider

Please Behave executes external adapters through a versioned JSON protocol. It
ships no providers and chooses none implicitly. Install or write an adapter, then
configure `.please/provider.toml` as the project default, or
`.please/providers/quality.toml` as a named configuration:

```toml
command = "python3"
args = ["tools/review-adapter.py"]
timeout_seconds = 180
```

| Field | Required / default | Meaning |
| --- | --- | --- |
| `command` | Required | Executable name or path; nonblank, no NUL. |
| `args` | Optional; `[]` | Literal arguments, including empty arguments; no NUL. |
| `timeout_seconds` | Required | `1..3600` seconds per invocation, including stdin delivery; `0` disables the deadline. |
| `requirements` | Optional table | Runtime prerequisites; omitted lists are empty. |
| `requirements.env` | Optional; `[]` | Required nonempty child environment variables; names only, never values. |
| `requirements.executables` | Optional; `[]` | Required executables, resolved through child PATH or paths relative to the consumer root. |
| `parameters` | Optional; `{}` | Arbitrary structured adapter settings; Please does not interpret their meaning. |

No shell, interpolation or argument expansion occurs. Adapters run in the consumer
root and inherit the environment except `PLEASE_*` variables, which are removed. They
get the execution point, the Rule parameters and the context file path in the request
JSON. Configure authentication and tool permissions in the
adapter. Trust adapters as you trust checks: they have Please Behave's permissions.
Please installs no adapter dependencies and provides no provider sandbox. The
prompt carries no safety, permission or role instructions; protecting the project
from an adapter's actions is the responsibility of the adapter and its provider.

A named configuration uses its filename stem as its ID: 1–80 ASCII letters,
digits, hyphens or underscores, excluding the reserved `please-` prefix. Select it
with `provider = "quality"` in `[review]`. Omission uses the explicit project
default; a selected enabled review without either is blocked. Disabled reviews
need no default. Unknown explicit references fail even when disabled. Adding named
configurations never changes the default.

External Rule Sets can distribute `providers/<id>.toml` beside their selected
`ruleset.toml`. Existing import boundaries, immutable caches and offline execution
apply. Repeated imports of the same immutable file share one definition; distinct
definitions with the same ID fail. Local overrides are unsupported.

Adapter settings may be supplied in `[parameters]` in a provider definition and
`[review.parameters]` in a Rule. Review values override matching provider keys;
nested tables and arrays replace the entire value at that key. Settings are
JSON-compatible TOML values, with no provider-specific validation in Please.
This is separate from typed Rule parameters, which adapters receive as `rule_parameters`.
Every review carries its own merged settings.

Runtime requirements are checked only for selected reviews, after deterministic
checks and immediately before invocation. Plans do not execute adapters or check
availability. Missing prerequisites, nonzero exits, timeouts and invalid responses
produce review ERROR. An adapter call that ends as ERROR retains its stdout/stderr unmasked
within the output bounds of checks.
Nonblocking review errors remain visible and follow the normal enforcement rules.

See the [adapter protocol](adapter-protocol.md) for request/response formats,
versioning, migration and a [working dummy adapter](../examples/adapters/dummy.py).
Adapters can wrap CLI tools, HTTP APIs or local models without changes to Please.

### Add a review

Create `.please/reviews/architecture/rule.toml`:

```toml
id = "architecture-review"
description = "Review architectural consistency"
blocking = false
run_at = ["pre-push"]

[review]
prompt = "prompt.md"
```

Create `.please/reviews/architecture/prompt.md`:

```markdown
Compare the implementation with the project's documented architectural boundaries.
Report concrete contradictions and missing essential evidence as FAIL,
and PASS only with supporting file paths and line numbers.
```

The Markdown contains instructions only, with no frontmatter or repeated metadata.
Prompts resolve exclusively inside their review directory. Reviews evaluate the
consumer through the adapter, one call per review, after deterministic checks.

| Field | Required / default | Meaning |
| --- | --- | --- |
| `review.provider` | Optional; `null` (omitted in TOML) | Named provider ID; omission requires an explicit default for selected reviews. |
| `review.parameters` | Optional; `{}` | Structured adapter settings overriding provider keys. |
| `review.trigger` | Optional; `null` (omitted in TOML) | Select this review only for matching changed files; the matching files are its scope. Without it any change selects the review and all changed files are in scope. |
| `review.trigger.changed_files` | Required | 1–64 repository-relative patterns, each at most 1024 UTF-8 bytes; any match selects the review. |
| `review.include_content` | Optional; `true` | Include the current content of the files in scope. Binary and deleted files have none. Content above 256 KiB ends the review as ERROR before any adapter call instead of being truncated. |
| `review.include_change_set` | Optional; `false` | Include the change set of the files in scope for the execution point. A change set above 32 KiB ends the review as ERROR before any adapter call instead of being truncated. |
| `review.prompt` | Required | Nonblank UTF-8 `.md` filename in the review directory. |
| `review.context` | Optional; `[]` | Up to 32 document selections as `[[review.context]]`. |
| `review.context.path` | Required | Existing UTF-8 consumer-relative file. |
| `review.context.headings` | Optional; `[]` | Up to 32 nonblank exact ATX Markdown headings; empty selects the whole document. |

At `commit-msg` the whole commit message (up to 64 KiB of UTF-8 without control
characters other than tab and line breaks) is part of every review's input with line
numbers, and the prompt decides which part counts. An empty message is valid. To review
only the summary line:

```toml
id = "commit-wording"
description = "Require a specific English imperative commit summary"
run_at = ["commit-msg"]

[review]
provider = "commit-quality"
prompt = "prompt.md"
include_content = false
```

A review at `commit-msg` blocks direct `please run --at commit-msg` and offline
preview because those invocations have no validated message-file argument.
`please sync` installs the managed hook; Git supplies the message when committing
or amending. The message is captured during planning, included in plan identity
and saved as `plan.input.commit_message`. Changes to it during checks or review
invalidate verification. See the [adapter protocol](adapter-protocol.md).

Context includes subsections and preserves original lines. Code-block headings
are ignored; missing or ambiguous headings are errors. Overlapping selections
are merged. All context is bounded to 32 distinct files, 1 MiB per file and 64 KiB
selected text; disabled definitions are validated too. Entry/context paths must
stay within the consumer. Supporting review files require no resource-import system.

To run a review only for relevant changes, add this to its `rule.toml`:

```toml
[review.trigger]
changed_files = ["src/**", "docs/design.md", ".please/reviews/architecture/prompt.md"]
```

Patterns match consumer-repository paths, including imported reviews. Matching
is case-sensitive and uses `/`: `*` matches zero or more characters within one
component, `?` matches one character, and a whole component `**` matches zero or
more directories. Hidden files match normally. Absolute paths, empty/dot/parent
components, backslashes, colons, controls, brackets, braces and embedded `**`
(such as `src/**.rs`) are invalid. Exact paths need not currently exist, so deleted
files can trigger reviews. A trigger selects the review and limits its scope; it does not supply context documents.

The changed files follow the execution point, and so does the change set of a review that
includes it:

| Execution point | Changed files and change set |
| --- | --- |
| `pre-commit`, `commit-msg` | The staged changes, as if the current index were committed. |
| `pre-push` | The pushed commits. Without Git's reference list, as in a direct or preview run, the commits of `HEAD` that are on no remote-tracking reference of the upstream remote, else `origin`, else any remote. |
| `check` and custom points | The net changes from `PLEASE_CHANGE_BASE` to the working tree. |

Renames count as a deletion of the old path and an addition of the new one, so a trigger
can match either. Content is read from the working files, also at `pre-commit`, so a
partly staged file shows its working state. Untracked files appear as additions at
`check` but have no change set. An unmerged path blocks the plan.

Set `PLEASE_CHANGE_BASE` to an explicit locally available Git commit or reference:

```sh
PLEASE_CHANGE_BASE=main please run --at check --plan --json
PLEASE_CHANGE_BASE=main please run --at check
```

Please Behave resolves the reference once and records its commit ID as `change_base` in
the plan. It compares that commit directly with current working files: committed,
staged and unstaged net changes, nonignored untracked files and declared governance
inputs even when ignored, except reports. Unreferenced ignored artifacts are excluded.
Changes reverted back to
the baseline do not select a review. Locally, choose the intended starting commit;
use `HEAD` explicitly when only uncommitted changes matter. In CI, fetch the intended
baseline and provide its commit ID. For a multi-commit PR, the caller can compute
and supply its merge base with the target branch; Please Behave never assumes `HEAD^` or
infers GitHub context. It does not fetch missing refs. Shallow clones work when
the supplied commit and tree are available locally. Index conflicts and
assume-unchanged/skip-worktree flags (including sparse checkouts) block comparison;
clear them before using `PLEASE_CHANGE_BASE`.

Missing/invalid baselines, unavailable trees, incomplete snapshots, unsupported
filename encoding, timeouts and Git path-list output above 128 MiB block execution with ERROR,
never a successful skip. Disabled reviews, checks-only runs and reviews at other points need no
changed files. At most 200 files may be in the scope of one review.

Reviews without matching changed files appear in preview and saved report plans under `omitted` with reason
`trigger_not_matched`, or `no_changes` when the review has no triggers and nothing changed; they have no result or required ID, need no provider runtime
credentials/executables, and contribute no context or planned model calls. Scheduled
reviews still wait for the global deterministic-check gate. Snapshot and context
limits remain unchanged.

The adapter's exit status is the verdict: 0 is PASS and 1 is FAIL, each only with a
valid response. A cited file must exist in the snapshot and a cited line be positive
for worktree reviews. At commit-msg, `path = null` with a `line` cites that
line of the commit message. Any other exit status, malformed JSON, invalid evidence
and provider failures produce ERROR. Summaries are single lines up to 120 characters;
an optional longer explanation has up to 8000 bytes; at most 20 findings per review,
2000 bytes each. FAIL and ERROR of a non-blocking review stay visible; configuration errors and
damaged imports still prevent success.

### Group rules into rule sets

Create `.please/rule-sets/repository.toml`:

```toml
id = "repository"
description = "General repository governance"
run_at = ["pre-push", "check"]
rules = [
  "repository-readme",
  { id = "architecture-review", run_at = ["check"] },
]
```

| Field | Required / default | Meaning |
| --- | --- | --- |
| `id` | Required | Unique set ID: 1–80 lowercase ASCII letters, digits, `-`, `_`, starting with a letter/digit. Independent of rule IDs. |
| `description` | Required | Nonblank explanation. |
| `run_at` | Required | Nonempty list of known execution points. |
| `rules` | Required | Strings or reference tables (below). Local: Rule IDs in the same scope. External: repository-relative Rule directories. Empty is allowed; set references are unsupported. |

Each reference may be a string or table, for Checks and Reviews alike.
Here the Check inherits both points while the Review runs only at `check`.

| Reference field | Required / default | Meaning |
| --- | --- | --- |
| `id` | Required | Same local Rule ID or external Rule directory path as a string reference. |
| `run_at` | Optional; inherits the set's `run_at` | Nonempty list of known execution points, replacing the set list for this reference. |

String references and tables without `run_at` keep the existing inheritance.
Overrides never merge with the set list. Unknown or duplicate points and empty
overrides fail, including on disabled rules. The set's own list remains required
and validated. Repeated references contribute the union of their effective lists;
each rule still executes once per point within a binding.

Present sets always apply; they have no enabled, blocking or source attributes.
Remove a set or its rule references to stop using it. Set membership overrides a
rule's own assignments; multiple memberships contribute the union of their
effective reference assignments. Each rule
runs at most once at a point within each import binding. Standalone enabled rules
use their own `run_at`.
Disabled rules never execute. All references, even overridden ones, are validated.

```sh
please sync
please run --at pre-commit
please run --at commit-msg
please run --at pre-push
please run --at check
please run --at release --plan --json
```

Checks finish before reviews; reports retain ID order within each kind. Empty points succeed. Every scheduled rule
must execute unless dependencies or fail-fast prevent it; `--checks-only` reports selected reviews as `NOT_RUN` without calling a provider. An offline plan
prepares the same selection without rules, provider calls or reconciliation.

### Import rule sets from Git

Add imports only to `.please/config.toml`:

```toml
project = "My_App"

[execution_points]
custom = ["verify", "audit"]

[[imports]]
git = "https://github.com/example/governance.git"
path = "repository"
ref = "v1.0.0"

[[imports]]
git = "https://github.com/example/governance.git"
path = "software-quality"
ref = "v1.0.0"

[imports.execution_points]
quality = "verify"

[[imports]]
git = "https://github.com/example/governance.git"
path = "security"
ref = "v2.1.0"

[imports.execution_points]
scan = "audit"
report = "verify"
```

| Field | Required / default | Meaning |
| --- | --- | --- |
| `git` | Required | Repository selector using host Git authentication: HTTP(S), SSH, SCP-style SSH, Git protocol, file URL or consumer-relative local repository. |
| `path` | Required | Selected directory, or `"."` for repository root; no parents or symlinks. |
| `ref` | Required | Exact branch, tag or full commit hash, 1–1024 bytes. |
| `execution_points` | Optional; `{}` | Per-import table mapping external custom names to consumer-declared custom names. |
| `rule_parameters` | Optional; `{}` | Per-import tables of rule IDs to parameter names and string values. |

The table belongs only to the immediately preceding `[[imports]]`. Sources must
be custom names used by that imported Rule Set; targets must already appear in
the consumer's `execution_points.custom`. Names follow the same syntax and limits
as custom declarations. Built-ins are forbidden on both sides. Unknown names,
empty/invalid names and structures other than a table of strings fail validation.
Unmapped points keep their names and must be known to the consumer.

Supply imported parameter values in tables belonging to that specific import.
The external rule declares `parameters.scopes_file` as above, omitting `value`:

```toml
[[imports]]
git = "https://github.com/example/governance.git"
path = "repository"
ref = "v1.0.0"

[imports.rule_parameters.commit-message]
scopes_file = "frontend/scopes.txt"

[[imports]]
git = "https://github.com/example/governance.git"
path = "repository"
ref = "v1.0.0"

[imports.rule_parameters.commit-message]
scopes_file = "backend/scopes.txt"
```

These imports execute as two distinct bindings. Values never merge, and there is
no global parameter table. Results use distinct imported IDs and retain source
provenance. Repeated references to a rule within one binding execute it once per
point, including when multiple source points map to the same target. Repeating
an identical import also creates a separate binding.

Mappings apply after scope-local Rule Set inheritance. Multiple source points may
map to one target; a Rule runs once there. Separate imports may map the same source
name differently or share targets. No global or individual Rule mapping exists.
Mapping preserves Rule identity, enabled/blocking state, resources and provenance;
it changes neither cached files nor revision identity and works offline without
fetching. No other import fields or Rule overrides are accepted. Remove the import to
stop using it. A selected directory defines one set in `ruleset.toml`, with the
same four fields as a local set. External `rules` entries name Rule directories
relative to the external repository root, for example:

```toml
id = "security"
description = "Check for secrets before push and in CI"
run_at = ["pre-push", "check"]
rules = ["rules/checks/secret-scan"]
```

Each referenced directory must be directly under a `checks/` or `reviews/`
directory and contain `rule.toml` plus its resources. Several sets can reference
the same directory. Only referenced Rules are loaded; repeated paths load once.
Distinct referenced directories must have unique Rule IDs within each import.
Imported parameter tables continue to use those IDs, not directory paths.
Each import remains a separate execution and parameter binding.
Absolute paths, parent traversal and symlinks are rejected. Sets cannot reference
other sets or consumer definitions, and accept no project settings or imports.
Local sets continue to reference Rule IDs. To migrate an existing external set,
replace each ID in `rules` with its repository-relative Rule directory; for
example, `"repository-readme"` becomes `"repository/checks/readme"`.
Identical rule IDs across independent imports do not collide; imported identities
and commit provenance remain deterministic.

`sync` resolves refs and validates all sources before atomically publishing
resolution state. A failed refresh preserves prior state. Execution, hooks and
plans reuse the validated immutable cache offline; missing/changed state or corrupt
contents require `please sync`. Multiple sets at one repository/commit share
one checkout. Full hashes pin content; branches/tags can move when refreshed.

See [external source structure, cache and trust](https://github.com/please-behave/cli/blob/main/docs/external-rule-sets.md).

### Import providers from Git

Providers can be distributed independently of Rule Sets. Add an explicit,
independently pinned import to the consumer's `.please/config.toml`:

```toml
[[provider_imports]]
id = "quality"
git = "https://github.com/company/governance-providers.git"
path = "providers/quality"
ref = "v1.2.0"
```

| Field | Required / default | Meaning |
| --- | --- | --- |
| `id` | Required | Named provider exposed to reviews; the ordinary provider ID rules apply. |
| `git` | Required | Git repository selector, with the same authentication and trust rules as Rule Set imports. |
| `path` | Required | Repository-relative directory containing `provider.toml` and adapter resources. |
| `ref` | Required | Exact branch, tag or full commit hash; full hashes pin reviewed content. |

The selected directory contains a normal `provider.toml`, for example:

```toml
command = "python3"
args = ["@provider/adapter.py"]
timeout_seconds = 70

[requirements]
executables = ["python3"]
env = ["QUALITY_API_KEY"]
```

The consumer needs no matching `.please/providers/quality.toml` or adapter copy.
A local or imported Review selects it using `provider = "quality"`. Rule Set
and provider imports may name the same repository or separate repositories,
with independently selected paths/revisions. No imports are inferred, nested
or fetched during execution. Provider packages need no Rule Set or execution point.
All imports share the 256-import bound; named provider IDs are global. Duplicate
IDs, including local/imported collisions, fail; there is no implicit override.
Existing providers distributed inside Rule Sets remain supported.

Only independent provider packages resolve explicit tokens in `command`, `args`
and `requirements.executables`: `@provider/` selects the package directory,
`@source/` its repository root and `@repo/` the consumer root. `@@` escapes one
leading @; other unknown @ prefixes fail. Referenced paths must exist and cannot
use absolute suffixes, parent traversal or symlinks. Resolved paths remain single
literal arguments, including spaces. Ordinary tokens still use PATH or the
consumer root. Local and legacy Rule Set-provider arguments keep their literal
behavior. There is no shell expansion, dependency installation or sandbox.

Run `please sync` to resolve and validate both import kinds, without executing
adapters. A failed sync preserves the previous combined state. Later plans,
hooks and execution use verified immutable cached revisions offline; corrupt or
missing resources fail. Cache integrity failures are fatal even for advisory
reviews. Runtime PATH/environment requirements are checked only for selected
reviews, after deterministic checks. Plans/reports retain `provider_origin`
(repository hash, selected path/ref and resolved commit), which participates in
plan identity. Provider resources stay in the cache; removing the import stops
using them without deleting the shared cache.

### Run context

Each executed run supplies Git facts to checks through the `@context` argument
and to adapters through `context_file` in the request. Use that supplied absolute
path and treat the context and its patch files as read-only.

The [Run Context API](../knowledge/contracts/run-context/api.yaml) defines
[delivery and lifetime](../knowledge/contracts/run-context/delivery.md) and the
[document and patches](../knowledge/contracts/run-context/document.md), including
their structural schema. Inspect its truncation indicators before relying on
complete evidence; obtain missing facts from Git or fail the rule.
Configure retention and section limits through the `context` settings above.
At the start of each run, `context.retain_runs` keeps the newest earlier run
contexts (default: 10), plus the current run. Earlier runs count even if they did
not finish. For example, `retain_runs = 3` keeps three earlier contexts and the
current one.

Pre-push reads the reference list from standard input, so commands placed after
the managed section of a composed hook do not receive it.

### Input snapshot

A saved report identifies the evaluated working files, including declared local
governance inputs even when ignored. Files that cannot be covered completely
prevent PASS. See the binding [Snapshot and Freshness contract](../knowledge/contracts/report/snapshot.md)
and its schema for coverage, exclusions and limits. Script dependencies are not
inferred; declare the inputs your Rules rely on.

### Reports and other files

Runs save local evidence in `.please/reports/last.json`. Use `please status` to
compare the saved input with your current files, and `please status --details`
to inspect results, findings and captured output. Keep reports and credentials
local, and avoid printing secrets: captured output is not automatically redacted.

The [Verification Report API](../knowledge/contracts/report/api.yaml) defines the
[document and lifecycle](../knowledge/contracts/report/document.md), including
versions, statuses, enforcement, output, usage and reader validation. Its schema
references the independent [Execution Plan API](../knowledge/contracts/plan/api.yaml).
