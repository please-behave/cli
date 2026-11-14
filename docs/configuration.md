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

`checks/`, `reviews/` and `rule-sets/` are optional. Each immediate rule directory
requires `rule.toml`. Only `config.toml`, `provider.toml`, `providers/*.toml`, `checks/*/rule.toml`,
`reviews/*/rule.toml` and `rule-sets/*.toml` are governance configuration. Supporting
files such as `checks/secrets/gitleaks.toml` are data and are never parsed by Please Behave.
Definitions cannot be placed arbitrarily or nested in other directories.

Unknown fields, duplicate keys/IDs and invalid references fail validation, even
for disabled rules. Missing implementations become execution errors only for
enabled selected rules. Text uses UTF-8; required text must be nonblank and contain
no NUL. Review prompt filenames must name regular files directly inside their own rule
directory: no empty names, separators, absolute paths, parent traversal or symlinks.
Rule directories, definitions and configuration paths also reject symlinks.

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

| Field | Required / default | Meaning |
| --- | --- | --- |
| `project` | Required | Nonblank display name. |
| `max_display_files` | Optional; `5` | Integer `1..5`: changed paths shown in status. |
| `execution` | Optional table | Check scheduling settings. |
| `execution.max_parallel` | Optional; `4` | Positive integer: maximum running checks; `1` runs serially. |
| `execution.fail_fast_terminate_running_instances` | Optional; `false` | Terminate running checks when invocation-level fail-fast triggers; requires Unix process groups. |
| `execution.fail_fast_termination_grace_seconds` | Optional; `5` | Integer `1..3600`: grace period between SIGTERM and forced termination, validated even when termination is disabled. |
| `context` | Optional table | Run context settings; see [Run context](#run-context). |
| `context.retain_runs` | Optional; `100` | Integer `1..10000`: stored run contexts kept; older ones are removed at the start of a run. |
| `context.max_section_bytes` | Optional; `10485760` | Integer `1024..1073741824`: byte limit for each patch file, each list and each message; larger sections are marked truncated. |
| `execution_points` | Optional table | Additional execution contexts. |
| `execution_points.custom` | Optional; `[]` | Unique names of 1–80 lowercase ASCII letters, digits, `-`, `_`, starting with a letter/digit. Built-in names are reserved. |
| `imports` | Optional; `[]` | External Rule Set references, declared only here as `[[imports]]`. |
| `provider_imports` | Optional; `[]` | Independent external named provider packages, declared as `[[provider_imports]]`. |

Built-in points are `pre-commit`, `commit-msg`, `pre-push` and `check`. The first
three use Git hooks. `check` is invoked explicitly with `please run --at check`
and never triggered automatically. It needs no declaration or CI workflow.
Consumers decide where to invoke it, including multiple workflows; Please Behave does
not generate workflows, discover them or validate CI integration. Declare custom
points in the consumer, including unmapped custom points used by imports.
`release` is an ordinary custom point. The default execution point is `pre-push`.

Checks run concurrently by default. `please run --at check --max-parallel 6`
overrides configuration for that invocation; otherwise `execution.max_parallel`
applies, defaulting to 4. Set it to 1 for serial execution. Zero, negative,
noninteger and overflowing limits are rejected. Concurrency does not change the
semantic plan hash; saved snapshots still detect configuration file changes.

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

Fail-fast termination is optional under `[execution]` and takes effect only with
`run --fail-fast`. See the [fail-fast behavior](#project-settings) for
process groups, cancellation, grace periods and interrupted-build limitations.

### Fields shared by checks and reviews

| Field | Required / default | Meaning |
| --- | --- | --- |
| `id` | Required | Case-sensitive ID unique across checks/reviews in its source scope; 1–80 ASCII letters, digits, `-`, `_`; `please-` is reserved. |
| `name` | Optional; `null` | Short display label; defaults to the original rule ID. 1–80 characters, no control characters or surrounding whitespace. |
| `description` | Required | Nonblank explanation. |
| `enabled` | Optional; `true` | False rules are validated but never executed or included in results. Plans list them under `disabled_rules`. |
| `blocking` | Optional; `true` | Reviews only: FAIL and provider/review ERROR prevent success when true. A check must not declare it: checks are always blocking and a declaration is a configuration error. Parameter and missing-prompt errors are always fatal. |
| `run_at` | Optional; `[]` | Own execution points; overridden by membership in Rule Sets. Enabled rules need an effective assignment. |
| `parameters` | Optional; `{}` | Named parameter declarations; see below. |
| `check` | Optional; `null` | Check implementation; required for a check rule and mutually exclusive with `review`. |
| `review` | Optional; `null` | Review implementation; required for a review rule and mutually exclusive with `check`. |

Names are display labels, not references: Rule Sets, parameters and providers
continue to use IDs. Imported names default to their source-local ID. Duplicate
names receive a short origin suffix and an occurrence number when necessary.
Plans and reports save labels so status remains readable after configuration changes.
Full technical IDs and import provenance are available in JSON and details.

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

| Field | Required / default | Meaning |
| --- | --- | --- |
| `parameters.<name>.type` | Required | `"string"` for an arbitrary string or `"file"` for an existing consumer file. |
| `parameters.<name>.required` | Optional; `false` | Whether a value must be supplied before execution. |
| `parameters.<name>.description` | Optional; `null` | Explanation for the consumer. |
| `parameters.<name>.value` | Optional; `null` | Local value. Imported declarations must omit this field; the consumer supplies it per import. |

Parameter names use ASCII letters, digits and underscores, starting with a letter and
are case-sensitive. Values are strings without NUL; there are no default values.
Omitted optional parameters remain absent; referencing one is an execution ERROR.

A `file` value resolves from the consuming Git repository root: `.commit-scopes`
and `conventions/commit-scopes.txt` both work. The target must be an existing
regular file. Absolute paths, parent traversal and symlink components are rejected.
Rule scripts, prompts and bundled assets still resolve inside the rule directory.

A check passes a value on explicitly with `@param/<name>`: `@param/scopes_file`
becomes the absolute path of the file, and strings stay unchanged. Reviews receive all
their resolved values as `rule_parameters` in the adapter request. Every review is
one adapter call with the same prepared context. Provider timeout applies per call.

Missing required values, unsupported types, undeclared supplied names and invalid
files produce **ERROR**, identifying the rule and parameter. Parameter errors block success even for a review with `blocking = false`. Disabled rules do not resolve parameter values or
require their implementations to exist.

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
executable checks from trusted sources. Changes to evaluated files during
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
`config.toml`. Existing import boundaries, immutable caches and offline execution
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
produce review ERROR. Raw adapter stdout/stderr are never stored as diagnostics.
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
| `review.trigger` | Optional; `null` (omitted in TOML) | Select this review only for matching changed files. |
| `review.trigger.changed_files` | Required | 1–64 repository-relative patterns, each at most 1024 UTF-8 bytes; any match selects the review. |
| `review.include_diff` | Optional; `true` | Include the Git orientation diff in the prompt; a change set above 32 KiB ends the review as ERROR before any adapter call instead of being truncated, so use smaller commits or `false`. Independent of file triggers and context. |
| `review.prompt` | Required | Nonblank UTF-8 `.md` filename in the review directory. |
| `review.files` | Optional; `[]` | Up to 64 unique existing consumer-relative source files; exact paths, not globs. Starting points, not an evidence boundary. |
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
include_diff = false
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
files can trigger reviews. A trigger selects execution; it does not supply context.

Set `PLEASE_CHANGE_BASE` to an explicit locally available Git commit or reference:

```sh
PLEASE_CHANGE_BASE=main please run --at check --plan --json
PLEASE_CHANGE_BASE=main please run --at check
```

Please Behave resolves the reference once and records its commit ID as `change_base` in
the plan. It compares that commit directly with current working files: committed,
staged and unstaged net changes, nonignored untracked files and declared governance
inputs even when ignored, except reports. Unreferenced ignored artifacts are excluded.
Renames match old and new paths. Changes reverted back to
the baseline do not trigger a review. Locally, choose the intended starting commit;
use `HEAD` explicitly when only uncommitted changes matter. In CI, fetch the intended
baseline and provide its commit ID. For a multi-commit PR, the caller can compute
and supply its merge base with the target branch; Please Behave never assumes `HEAD^` or
infers GitHub context. It does not fetch missing refs. Shallow clones work when
the supplied commit and tree are available locally. Index conflicts and
assume-unchanged/skip-worktree flags (including sparse checkouts) block comparison;
clear them before using triggers.

Missing/invalid baselines, unavailable trees, incomplete snapshots, unsupported
filename encoding, timeouts and Git path-list output above 128 MiB block execution with ERROR,
never a successful skip. Disabled reviews and reviews at other points need no
baseline. Reviews without triggers keep their existing behavior.

Nonmatches appear in preview and saved report plans under `omitted` with reason
`trigger_not_matched`; they have no result or required ID, need no provider runtime
credentials/executables, and contribute no context or planned model calls. Scheduled
reviews still wait for the global deterministic-check gate. Snapshot and context
limits remain unchanged.

The provider must return exactly one PASS/FAIL/ERROR per requested ID. PASS/FAIL
need an existing in-snapshot file and positive line reference for worktree reviews.
At commit-msg, `path = null` with a `line` cites that line of the commit message. Malformed JSON,
missing/duplicate verdicts, invalid evidence and provider failures produce ERROR.
Summaries are single lines up to 120 characters; at most 20 findings per review,
2000 bytes each. Non-blocking FAIL is visible; incomplete verification stays fatal.

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
stop using it. A selected directory defines one set in `config.toml`, with the
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

Each run writes one JSON document that describes the Git state the rules judge, so rules need
not call Git themselves. A check receives its absolute path as the `@context` argument and
an adapter as `context_file` in its request; never hardcode the location. The format is specified by the
[JSON Schema](schemas/context.schema.json) (`schema_version` 1); additive changes extend that
schema and removals need a new schema version. The files live in the worktree-local Git
directory (`please/context/<run-id>/`), are readable by the owner only, and are not part of the
working tree or the snapshot. Rules must treat the context as read-only. File contents are never
included: a rule reads files from disk and uses the context to confirm that they are clean.

| Section | Present | Content |
| --- | --- | --- |
| `repository` | Always | Git directory, common directory, `linked_worktree` and `shallow`. |
| `head` | Always | `commit` (null in an unborn repository), `branch` (null when detached) and the HEAD `message`. |
| `workspace` | Always | `clean`, and `staged`, `unstaged`, `untracked` and `conflicted` paths with their status and, for renames, `old_path`. `staged_patch` (index against HEAD) and `unstaged_patch` (working tree against index) point to patch files. |
| `commit_message` | `commit-msg` | Message file and text. The message does not exist at `pre-commit`. |
| `push` | `pre-push` | The remote name and one update per pushed reference: `kind` (`create`, `update` or `delete`), `is_head` (the pushed tip is the checked-out HEAD), the new `commits`, `changed_files` and a `patch` file. |
| `changes` | `PLEASE_CHANGE_BASE` names an available commit | The resolved base commit and the net changed files and patch from that commit to the working tree, including untracked files. |

`workspace.clean` is true only when nothing is staged, modified, untracked or conflicted;
ignored files do not count. A path listed both as staged and unstaged is partially staged, so
the file on disk differs from what a commit records. For a push, `clean` together with `is_head`
tells a rule that the working tree equals what is being pushed. For a new reference, the commits
are those not yet reachable from the remote's tracking references. When the remote tip is
unknown locally, `range_available` is false and the rule must use Git itself.

Patch objects have `file` (a name relative to the context directory, or null when there is no
change), `bytes` and `truncated`. Paths that are not valid UTF-8 are counted in
`unrepresentable_paths` instead of being listed. A section above `context.max_section_bytes`
is cut, flagged `truncated` and, for lists, counted in `omitted_*`; a rule then fails or reads the
data from Git. Pre-push reads the reference list from standard input, so commands placed after
the managed section of a composed hook do not receive it.

### Reports and other files

Reports use schema 4 and plans use schema 3. These output schema numbers describe
machine-readable results, not configuration variants. Results record blocking,
status, evidence, durations and captured output. ERROR always takes precedence;
blocking FAIL follows; fail-fast skips and cancellations prevent success. Dependency
skips inherit the originating outcome. Disabled rules
are listed in plans and stay out of results. JSON records FAIL even when the
overall outcome is PASS because that review is non-blocking.

`reports/last.json` is replaced atomically after a run. Invalid configuration
preserves a previous report. Keep reports and credentials local. Stdout/stderr
retain the first 64 KiB each with full byte totals/truncation indicators; output
is not automatically redacted. `--details` displays retained output and findings.
Status compares the saved working-tree snapshot without executing rules again.
