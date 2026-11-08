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

[execution_points]
custom = ["release"]
```

| Field | Required / default | Meaning |
| --- | --- | --- |
| `project` | Required | Nonblank display name. |
| `max_display_files` | Optional; `5` | Integer `1..5`: changed paths shown in status. |
| `execution` | Optional table | Check scheduling settings. |
| `execution.max_parallel` | Optional; `4` | Positive integer: maximum running checks; `1` runs serially. |
| `execution_points` | Optional table | Additional execution contexts. |
| `execution_points.custom` | Optional; `[]` | Unique names of 1–80 lowercase ASCII letters, digits, `-`, `_`, starting with a letter/digit. Built-in names are reserved. |
| `imports` | Optional; `[]` | External references, declared only here as `[[imports]]`. |

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

### Fields shared by checks and reviews

| Field | Required / default | Meaning |
| --- | --- | --- |
| `id` | Required | Case-sensitive ID unique across checks/reviews in its source scope; 1–80 ASCII letters, digits, `-`, `_`; `please-` is reserved. |
| `name` | Optional; `null` | Short display label; defaults to the original rule ID. 1–80 characters, no control characters or surrounding whitespace. |
| `description` | Required | Nonblank explanation. |
| `enabled` | Optional; `true` | False rules are validated but never executed or included in results. Plans list them under `disabled_rules`. |
| `blocking` | Optional; `true` | FAIL and provider/review ERROR prevent success when true. Check, parameter and missing-prompt errors are always fatal. |
| `run_at` | Optional; `[]` | Own execution points; overridden by membership in Rule Sets. Enabled rules need an effective assignment. |
| `parameters` | Optional; `{}` | Named parameter declarations; see below. |
| `check` | Optional; `null` | Check implementation; required for a check rule and mutually exclusive with `review`. |
| `review` | Optional; `null` | Review implementation; required for a review rule and mutually exclusive with `check`. |

Names are display labels, not references: Rule Sets, parameters and providers
continue to use IDs. Imported names default to their source-local ID. Duplicate
names receive a short origin suffix and an occurrence number when necessary.
Plans and reports save labels so status remains readable after configuration changes.
Full technical IDs and import provenance are available in JSON and details.

### Rule parameters

Checks and reviews use the same parameters. Declare each parameter in its rule's
`rule.toml`. For a local commit-message check, for example:

```toml
id = "commit-message"
description = "Validate Conventional Commits"
run_at = ["commit-msg"]

[check]
command = "sh .please/checks/commit-message/index.sh"

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

Parameter names use ASCII letters, digits and underscores, starting with a letter.
Uppercase names must be unique within a rule. Values are strings without NUL;
there are no default values. Omitted optional parameters remain absent.

A `file` value resolves from the consuming Git repository root: `.commit-scopes`
and `conventions/commit-scopes.txt` both work. The target must be an existing
regular file. Absolute paths, parent traversal and symlink components are rejected.
Rule scripts, prompts and bundled assets still resolve inside the rule directory.

Please Behave delivers values as environment variables: `scopes_file` becomes
`PLEASE_PARAM_SCOPES_FILE`. File values become absolute paths; strings stay unchanged.
A check can read `"$PLEASE_PARAM_SCOPES_FILE"` in shell or
`os.environ["PLEASE_PARAM_SCOPES_FILE"]` in Python. Reviews receive the same variables
in their provider subprocess, and their criteria data lists this environment.
Inherited `PLEASE_PARAM_*` variables are cleared before applying declared values.
Reviews sharing identical resolved environments share a call; different environments
use separate calls with the same prepared context. Provider timeout applies per call.

Missing required values, unsupported types, undeclared supplied names and invalid
files produce **ERROR**, identifying the rule and parameter. Parameter errors block success even with `blocking = false`. Disabled rules do not resolve parameter values or
require their implementations to exist.

### Add a check

Create `.please/checks/readme/rule.toml`:

```toml
id = "repository-readme"
description = "Require a nonempty README"
run_at = ["pre-push", "check"]

[check]
command = "sh .please/checks/readme/index.sh"
```

Create `.please/checks/readme/index.sh`:

```sh
#!/bin/sh
test -s README.md
```

String-based `check.command` is preferred for new checks. For example:

```toml
[check]
command = "python3 scripts/check-docs.py"
```

Array commands and file entrypoints with optional interpreters are deprecated
but remain fully supported, without runtime warnings. The legacy file form uses
`entrypoint = "index.sh"` instead of `command`. On Unix, direct file execution
requires executable permission (`chmod +x .please/checks/readme/index.sh`);
the OS handles shebangs. Please Behave does not infer languages or install runtimes.
Unix scripts may need an explicitly installed interpreter on Windows.

For example, the deprecated Python file form uses:

```toml
[check]
entrypoint = "index.py"
interpreter = "python3"
timeout_seconds = 30
```

Create `index.py` in that rule directory containing:

```python
from pathlib import Path
raise SystemExit(0 if Path("README.md").is_file() else 1)
```

This runs `python3 <absolute-entrypoint>` with no implicit shell. The interpreter
is one executable name or path, with no argument parsing. The entrypoint needs
neither a shebang nor executable permission in this mode. Missing interpreters
produce a clear startup ERROR.

| Field | Required / default | Meaning |
| --- | --- | --- |
| `check.entrypoint` | Optional; `null`; required unless `command` is set | Deprecated file syntax: filename in this Rule directory, or path containing `/` relative to the rule-source root (see below). |
| `check.command` | Optional; `null`; required unless `entrypoint` is set | Preferred: string containing executable and arguments. Deprecated arrays remain supported. Mutually exclusive with `entrypoint` and `interpreter`. |
| `check.interpreter` | Optional; `null` | Deprecated file syntax: executable used to run an entrypoint. Omitted means direct execution; forbidden with `command`. |
| `check.resource` | Optional; `null` | Exclusive, case-sensitive resource name; nonempty, no control characters or surrounding whitespace. |
| `check.timeout_seconds` | Optional; `30` | Integer `1..3600`; deadline including output capture. |

For an installed tool, use a direct command instead of a script wrapper:

```toml
[check]
command = "cargo +stable test --locked"
timeout_seconds = 600
```

Set exactly one of `command` and `entrypoint`. The first argument is an executable
name resolved through `PATH`, or an explicit executable path. Relative command
paths use the consuming root, including for imported commands; only explicit
path references resolve against Rule or source directories.

Explicit path references are supported in string commands after tokenization,
in the executable position and in arguments:

| Prefix | Root |
| --- | --- |
| `@rule/` | Directory containing this Rule's `rule.toml`. |
| `@repo/` | Consuming repository root. |
| `@source/` | Rule source root: the consuming repository root for local Rules (not `.please/` or a Rule Set directory), or the cached imported Git repository root. |

```toml
[check]
command = "sh @rule/index.sh"
```

```toml
[check]
command = "python3 @source/scripts/file-reference.py"
```

```toml
[check]
command = "python3 @rule/check.py @repo/pyproject.toml"
```

Only a token beginning with a supported prefix is resolved; there is no embedded
substitution (`--file=@repo/data` stays literal). Quote a reference containing
spaces, such as `"@rule/my script.py"`; its resolved absolute path remains one
argument and is never tokenized again. Ordinary relative paths still use the
consumer root. No shell is invoked and the working directory stays that root.

At the start of a string-command token, `@` is reserved. Use `@@` to pass one
literal leading `@`: `@@rule/file` becomes `@rule/file`, and `@@name` becomes
`@name`. Quoting or backslash escaping an `@` does not disable resolution,
because references are recognized after tokenization. Array commands retain all
arguments literally, including `@` and `@@`.

Reference suffixes must contain nonempty relative components separated by `/`
on Linux, macOS and Windows. Empty suffixes, repeated/trailing separators, `.`,
`..`, backslashes and colons (including drive prefixes) are rejected during
configuration loading, as are unknown prefixes or bare `@` tokens. Use `@@` for
literal arguments instead. Native absolute paths are produced for the host;
this does not make an unavailable executable or interpreter portable.

Immediately before execution, referenced paths must exist under the chosen root.
Symlinks in the root or suffix, including links pointing inside the root, are
rejected for all three prefixes. Paths are checked with the existing source
containment checks and canonicalized before being passed to the process. Missing
or unsafe references produce per-rule ERROR with the token and root; there is no
fallback search. Files and directories may be referenced; the OS still enforces
executable permissions. Offline plans validate syntax without requiring referenced
paths to exist or launching commands. Checks are not a filesystem sandbox, and
these checks do not eliminate concurrent filesystem replacement races.

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
- Please Behave invokes no shell and implements no pipelines, redirection, variable
  expansion, command chaining, comments or globbing. Wildcards and `~` are literal.

TOML literal strings avoid a second layer of backslash escaping:

```toml
[check]
command = '"tools/my checker" "path with spaces" "" path\ with\ spaces'
```

This passes three arguments to `tools/my checker`: `path with spaces`, an empty
string, and `path with spaces`. With TOML double-quoted strings, escape backslashes
and embedded double quotes according to TOML first.

Legacy arrays such as `command = ["cargo", "+stable", "test", "--locked"]`
bypass this grammar; every element remains literal, including spaces, empty
arguments and shell punctuation. Both forms require a nonblank executable and
forbid NUL. Missing tools produce per-rule execution ERRORs; validation and offline
plans do not launch commands. Parameters, working directory, Rule-directory
context, timeout, output capture and exit codes are shared with file entrypoints.
`interpreter` is only available with `entrypoint`.

A filename such as `index.sh` resolves inside the declaring Rule directory.
A path containing `/`, such as `scripts/check-files.py`, resolves from the
rule-source root: the consumer repository root for local Rules, or the imported
Git repository root, regardless of the selected Rule Set directory. Use `/` as
the separator. Please Behave chooses the base from the syntax and never searches other
locations. Entrypoints must be regular files within that source; absolute paths,
parent traversal, backslashes, drive prefixes and symlink components are rejected.

Several Rules can set `entrypoint = "scripts/check-files.py"` to share one script.
Each Rule still executes separately with its own parameters, timeout, blocking
behavior and PASS/FAIL/ERROR result. Git imports support shared scripts offline
from their resolved cache. Review prompts remain filenames in their own Rule
directories.

Checks inherit the environment, receive no stdin and keep the consuming project's
root as their working directory. `PLEASE_RULE_DIRECTORY` is the absolute directory
of the declaring Rule, even when its entrypoint is shared, for bundled resources
such as a scanner configuration or helper.
For example, Python can read `Path(os.environ["PLEASE_RULE_DIRECTORY"]) / "config.json"`.

Exit **0 = PASS**, **1 = FAIL**, **2 = ERROR**. Unexpected numeric codes produce
FAIL with an explicit diagnostic. Signal termination, missing entrypoints,
process-launch, capture and timeout failures are ERROR. Wrap external tools to
translate their own exit-code contracts: a scanner's exit 2 may mean a violation,
so map that to 1 and map actual execution problems to 2. Please Behave does not parse
shell error messages to classify startup failures. A non-blocking
FAIL remains visible and permits reviews and overall success; an ERROR remains
fatal regardless of blocking. Output capture and process cleanup are shared
across direct and interpreter execution.

Resource confinement is not process sandboxing. Checks execute with Please Behave's
permissions and can access files, credentials and the network. Only import
executable checks from trusted sources. Changes to evaluated files during
execution prevent PASS.

### Configure the AI provider


The **provider** runs AI requests through built-in **Codex CLI** or a generic
external CLI. The default Codex provider uses its existing local login. Install
and authenticate Codex before enabling a Codex review. In the same terminal or environment that will run Please Behave, check:

```sh
command -v codex
codex --version
codex login
```

Please Behave starts the executable named `codex` through its inherited PATH, from the
project root. Shell aliases, functions and startup files are not loaded. The built-in Codex executable path is fixed. Generic CLI providers configure their
own command. IDEs and Git hooks must also have the Codex
installation directory on PATH.

Generated `.please/provider.toml`:

```toml
provider = "codex-cli"
timeout_seconds = 180
model = "gpt-6.1-sol"
mcp_servers = []

[review_profile]
reasoning_effort = "medium"
tool_output_token_limit = 2000
```

| Field | Required / default | Meaning |
| --- | --- | --- |
| `provider` | Required | `"codex-cli"` or `"cli"`. |
| `command` | Optional; `null` | Required for `cli`: executable name or path; nonblank, no NUL. Unsupported for Codex. |
| `args` | Optional; `[]` | Literal CLI arguments, including empty arguments; no NUL. Unsupported for Codex. |
| `requirements` | Optional table | Runtime prerequisites for this provider; omitted lists are empty. |
| `requirements.env` | Optional; `[]` | Nonempty child environment variables required at execution. Names use ASCII letters, digits and underscores, starting with a letter or underscore. |
| `requirements.executables` | Optional; `[]` | Required executable names or paths; nonblank, no NUL. |
| `timeout_seconds` | Required | Integer `1..3600` (CLI also accepts `0` for no overall deadline): seconds per invocation, including stdin delivery. Codex includes MCP discovery, itself capped at 10 seconds. |
| `model` | Optional; Codex default | Model identifier passed as `--model` for Codex or child environment variable `PLEASE_MODEL` for CLI; nonblank, no NUL. Generated files select GPT-6.1 Sol (`gpt-6.1-sol`); alternatives must be supported by your CLI/account. |
| `mcp_servers` | Optional; `[]` | Names of Codex MCP servers allowed for reviews. Empty or omitted disables all of them. See below. |
| `review_profile` | Optional table | Settings for batches using this provider configuration. |
| `review_profile.reasoning_effort` | Optional; Codex default | `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max` or `ultra`. Actual support depends on the model/CLI. |
| `review_profile.tool_output_token_limit` | Optional; `2000` | Integer `256..16384`: tokens retained per tool output in Codex history. This is not a limit on total tokens or cost. |

Omitting `model` or `reasoning_effort` uses the corresponding Codex default.
Unsupported models or settings produce ERROR; Please Behave does not silently substitute
another model. Each review batch uses its resolved provider configuration.

**MCP integrations.** MCP (Model Context Protocol) servers give Codex access to
additional tools or data, such as a project knowledge graph. Define server commands,
URLs, transports and credentials in Codex configuration. Please Behave only chooses which
already configured servers a review call may use. To allow two, replace the
top-level empty list in `provider.toml` with:

```toml
mcp_servers = ["project-memory", "browser"]
```

Use actual configured names: at most 32 unique names, each 1–256 ASCII letters,
digits, `_` or `-`; wildcards are not supported. Reviews using this configuration receive the same list.

Before the AI call, Please Behave reads `codex mcp list --json` without starting a model
or server. It disables all unlisted servers for the child process. An unknown
allowed name, failed discovery, more than 256 discovered servers or unsafe server
names in that listing blocks the review. An empty list never inherits all of
your personal integrations.

**Provider execution.** Please Behave uses noninteractive, ephemeral Codex sessions with
structured output, a read-only sandbox and no approvals. Automatic skill instructions,
bundled skills, skill search/installation, apps/plugins, subagents and web search
are disabled. Project AGENTS.md and ordinary Codex safety settings still apply.
Allowed MCPs are trusted capabilities; they are not an additional permission sandbox.

Please Behave does not rewrite your Codex configuration. It removes `OPENAI_API_KEY` and
`CODEX_API_KEY` from Codex child processes; login-backed reviews still consume your
account allowance. Missing CLI/login, incompatible CLI options or exhausted allowance
produce ERROR. No credentials are copied to CI.


`provider.toml` is optional; omission uses the generated provider settings above.
When the file is present its required fields must be supplied.

**Named configurations.** A complete executable setup can also live in
`.please/providers/quality.toml`, with exactly the same fields and defaults as
`provider.toml` above. The filename stem is its ID: 1–80 ASCII letters, digits,
`-` or `_`, excluding the reserved `please-` prefix. Up to 256 named configurations
are supported across the project and imports. Each file has the same 64 KiB
metadata limit as other TOML configuration. Add `provider = "quality"` inside a
review's `[review]` table to select it. Models cannot be overridden in reviews.

For migration, copy your existing `provider.toml` into `providers/quality.toml`
and add references to the reviews that should use it. Reviews without a reference
continue using `provider.toml`; removing that file uses the generated defaults.
Adding named configurations never changes the default provider.

External rule sets distribute configurations in `providers/<id>.toml` beside the
selected set's `config.toml`. Every such configuration is loaded and validated,
including unused ones. IDs are global across local configuration and explicit
imports. Reviews may reference configurations from another explicitly imported
set, regardless of import order. Missing references fail, including in disabled
reviews. Distinct definitions with the same ID fail; identical settings do not
permit local overrides. Repeated imports of the same immutable provider file
share its definition. No provider dependencies are downloaded automatically.
Existing offline cache integrity and atomic resolution guarantees apply.

Codex reviews batch together only when their provider reference and resolved
parameter environment match. Generic CLI reviews execute individually. Different references use
separate invocations, even if their settings match. Plans and saved reports record
each explicit reference and its effective configuration. Provider files accept
no credential fields. Codex uses its existing login; generic tools inherit
credentials from the execution environment. Please Behave never records their values.

**Generic CLI.** The repository's Gemini REST adapter uses Python's standard
library and the existing CLI transport:

```toml
# .please/providers/gemini.toml
provider = "cli"
command = "python3"
args = ["scripts/please-gemini-review.py"]
model = "gemini-3.8-flash"
timeout_seconds = 0

[requirements]
env = ["GEMINI_API_KEY"]
executables = ["python3"]
```

The adapter reads the complete prompt from stdin and selects the configured model
through `PLEASE_MODEL`. The same adapter serves repository governance.
It sends a generateContent request and extracts the first completed candidate's
text. Please Behave validates the unchanged review JSON. Requests time out after
600 seconds. Transient connection failures and HTTP 429/500/502/503/504 retry up
to five total attempts, with delays of 30/60/120/240 seconds; Retry-After can
increase a delay up to 240 seconds. Timeouts and invalid responses are not retried.
Diagnostics suppress raw request and response content. No Gemini CLI is needed.
For tools that already accept stdin and return Please Behave JSON, configure their
executable directly and put model flags in `args` instead of using `model`.

Commands run directly from the consumer root. Arguments are literal: no shell
operators, variable expansion, globbing, interpolation or substitution. Use an
explicit executable path or inherited PATH. Prompt delivery uses a pipe, without
a temporary prompt file. The prompt includes review instructions, prepared project
context, evidence rules and the JSON response schema. Return one verdict for the
requested review ID, with no fences, extra text or envelope:

```json
{"reviews":[{"id":"architecture-review","status":"PASS","reason":"Criteria satisfied","findings":[{"message":"Boundary is explicit","path":"src/main.rs","line":1}]}]}
```

The ID must match the prompt. PASS and FAIL require real in-manifest file/line
evidence; ERROR expresses unavailable evidence or uncertainty. The existing
[response contract](https://github.com/please-behave/cli/blob/main/SPECIFICATION.md#out-001-structured-review-outcomes) applies.
Malformed, incomplete or ungrounded responses are ERROR. Nonzero exits, missing
commands and timeouts are ERROR regardless of output. Stdout and stderr are
drained with bounded captures; stdout beyond 64 KiB is rejected. Raw output and
invalid response details are excluded from diagnostics. Generic token usage is
unavailable. Process trees use the same cleanup as checks; this is not a sandbox.

Requirements are syntax-checked even for unused providers. Availability is checked
only after point selection, deterministic checks and review parameter resolution,
for the providers that will execute. Missing/empty variables or missing executables
produce review ERROR; an unused provider never requires its credentials. Offline
plans and `sync` do not check runtime availability. Executable requirements
use the child's PATH or explicit paths relative to the consumer root. Environment
values stay outside TOML and must be supplied to the terminal, CI or hook process.

Generic providers cannot use Codex MCP controls or custom review profiles; omit
those settings. They inherit the execution environment, rule parameters and
`PLEASE_EXECUTION_POINT`; omitted `model` clears inherited `PLEASE_MODEL`.
Codex keeps its MCP allow list, model/effort, login policy and batching unchanged.

Trust executable providers as you trust checks. Imported provider commands follow
the existing [external source trust model](https://github.com/please-behave/cli/blob/main/docs/external-rule-sets.md#source-resources-and-trust):
explicitly imported, validated, immutable cached definitions, without local
overrides. Imported programs execute with your permissions. Please Behave supplies no
generic tool sandbox; read-only settings and authentication belong to the chosen
tool or wrapper. The repository Gemini adapter owns HTTP transport and retries; Please Behave retains
selection, prompt assembly and verdict validation.

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
Report concrete contradictions as FAIL, missing essential evidence as ERROR,
and PASS only with supporting file paths and line numbers.
```

The Markdown contains instructions only, with no frontmatter or repeated metadata.
Prompts resolve exclusively inside their review directory. Reviews evaluate the
consumer through the existing shared provider call, after deterministic checks.

| Field | Required / default | Meaning |
| --- | --- | --- |
| `review.provider` | Optional; `null` (omitted in TOML) | Named provider ID; omission uses the legacy project provider. |
| `review.trigger` | Optional; `null` (omitted in TOML) | Select this review only for matching changed files. |
| `review.trigger.changed_files` | Required | 1–64 repository-relative patterns, each at most 1024 UTF-8 bytes; any match selects the review. |
| `review.include_diff` | Optional; `true` | Include the Git orientation diff in the prompt; independent of file triggers and context. Different settings use separate provider batches. |
| `review.prompt` | Required | Nonblank UTF-8 `.md` filename in the review directory. |
| `review.files` | Optional; `[]` | Up to 64 unique existing consumer-relative source files; exact paths, not globs. Starting points, not an evidence boundary. |
| `review.context` | Optional; `[]` | Up to 32 document selections as `[[review.context]]`. |
| `review.context.path` | Required | Existing UTF-8 consumer-relative file. |
| `review.context.headings` | Optional; `[]` | Up to 32 nonblank exact ATX Markdown headings; empty selects the whole document. |

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
staged and unstaged net changes, nonignored untracked files and ignored `.please/`
files except reports. Renames match old and new paths. Changes reverted back to
the baseline do not trigger a review. Locally, choose the intended starting commit;
use `HEAD` explicitly when only uncommitted changes matter. In CI, fetch the intended
baseline and provide its commit ID. For a multi-commit PR, the caller can compute
and supply its merge base with the target branch; Please Behave never assumes `HEAD^` or
infers GitHub context. It does not fetch missing refs. Shallow clones work when
the supplied commit and tree are available locally. Index conflicts and
assume-unchanged/skip-worktree flags (including sparse checkouts) block comparison;
clear them before using triggers.

Missing/invalid baselines, unavailable trees, incomplete snapshots, unsupported
filename encoding, timeouts and Git output above 64 KiB block execution with ERROR,
never a successful skip. Disabled reviews and reviews at other points need no
baseline. Reviews without triggers keep their existing behavior.

Nonmatches appear in preview and saved report plans under `omitted` with reason
`trigger_not_matched`; they have no result or required ID, need no provider runtime
credentials/executables, and contribute no context or planned model calls. Scheduled
reviews still wait for the global deterministic-check gate. Snapshot and context
limits remain unchanged.

The provider must return exactly one PASS/FAIL/ERROR per requested ID. PASS/FAIL
need an existing in-snapshot file and positive line reference. Malformed JSON,
missing/duplicate verdicts, invalid evidence and provider failures produce ERROR.
Summaries are single lines up to 120 characters; at most 20 findings per review,
2000 bytes each. Non-blocking FAIL is visible; incomplete verification stays fatal.

### Group rules into rule sets

Create `.please/rule-sets/repository.toml`:

```toml
id = "repository"
description = "General repository governance"
run_at = ["pre-push", "check"]
rules = ["repository-readme", "architecture-review"]
```

| Field | Required / default | Meaning |
| --- | --- | --- |
| `id` | Required | Unique set ID: 1–80 lowercase ASCII letters, digits, `-`, `_`, starting with a letter/digit. Independent of rule IDs. |
| `description` | Required | Nonblank explanation. |
| `run_at` | Required | Nonempty list of known execution points. |
| `rules` | Required | Local: Rule IDs in the same scope. External: repository-relative Rule directories (see imports below). Repeated references execute once per binding. Empty is allowed; set references are unsupported. |

Present sets always apply; they have no enabled, blocking or source attributes.
Remove a set or its rule references to stop using it. Set membership overrides a
rule's own assignments; multiple memberships contribute their union. Each rule
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
must execute; `--checks-only` cannot omit required reviews. An offline plan
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

### Reports and other files

Reports use schema 4 and plans use schema 3. These output schema numbers describe
machine-readable results, not configuration variants. Results record blocking,
status, evidence, durations and captured output. ERROR always takes precedence;
blocking FAIL follows; unexecuted selected rules prevent success. Disabled rules
are listed in plans and stay out of results. JSON records FAIL even when the
overall outcome is PASS because that violation is non-blocking.

`reports/last.json` is replaced atomically after a run. Invalid configuration
preserves a previous report. Keep reports and credentials local. Stdout/stderr
retain the first 64 KiB each with full byte totals/truncation indicators; output
is not automatically redacted. `--details` displays retained output and findings.
Status compares the saved working-tree snapshot without executing rules again.
