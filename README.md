# Please Behave

```text
Please Behave

Enforce development rules across coding agents and workflows.
```

Please Behave is a source-available, agent-agnostic
governance framework for software development: you define the expectations that
development results must satisfy as version-controlled rules alongside your code.
The same rules apply whether a person or a coding agent writes the code.
Today, its native CLI evaluates these rules in Git projects through deterministic
checks and AI reviews, recording verdicts and supporting evidence.

Use it when a successful build is only part of your definition of “ready”: tests
and linters can verify concrete conditions, while an AI review can compare your
implementation with written requirements and architectural decisions. Developers
and coding agents choose how to implement changes; Please Behave evaluates whether the
results meet the configured expectations. See the [product vision](docs/product-vision.md)
for its principles and long-term direction.

## Contents

- [How Please Behave works](#how-please-behave-works)
  - [Terms](#terms)
- [Getting started](#getting-started)
  - [Requirements](#requirements)
  - [Install a release](#install-a-release)
  - [Build locally](#build-locally)
  - [Initialize your project](#initialize-your-project)
- [Using Please Behave](#using-please-behave)
  - [Commands and options](#commands-and-options)
  - [Run checks without AI](#run-checks-without-ai)
  - [Read results](#read-results)
  - [Choose rules and preview a run](#choose-rules-and-preview-a-run)
  - [Use a Git hook or CI](#use-a-git-hook-or-ci)
- [Configuration](#configuration)
  - [Files and validation](#files-and-validation)
  - [Project settings](#project-settings)
  - [Fields shared by checks and reviews](#fields-shared-by-checks-and-reviews)
  - [Add a check](#add-a-check)
  - [Configure the AI provider](#configure-the-ai-provider)
  - [Add a review](#add-a-review)
  - [Group rules into rule sets](#group-rules-into-rule-sets)
  - [Import rule sets from Git](#import-rule-sets-from-git)
  - [Reports and other files](#reports-and-other-files)
- [Limits and unsupported features](#limits-and-unsupported-features)
- [Development](#development)
- [Frequently Asked Questions](#frequently-asked-questions)

## How Please Behave works

Your project's governance rules live in `.please/` alongside the source code,
where they can be reviewed and versioned together. Run `please run` to
evaluate the current working files against those expectations.

Please Behave validates the configuration, runs the selected checks, and then asks the
AI to perform the selected reviews. If a check has a blocking failure or an error,
reviews are skipped to avoid
spending tokens on a run that has already failed. All remaining checks still run.
The terminal shows progress as each rule starts and finishes, followed by a compact
report. Detailed results are saved locally for later inspection.

Start with checks for things your tools can establish directly, such as tests,
compilation or formatting. Add reviews for questions that need interpretation,
such as whether code follows requirements in a product document. AI judgments
can be wrong; a review PASS is evidence to inspect, not a guarantee of correctness.

### Terms

| Term | Meaning | Example |
| --- | --- | --- |
| **Rule** | One thing Please Behave verifies. Every rule is either a check or a review. | “Unit tests pass” or “The documented CLI matches the implementation.” |
| **Check** | A command whose exit status determines the result. It uses no AI provider and has no review-token cost. | Run `cargo test --locked`. |
| **Review** | An AI request to evaluate written criteria against project files and return a structured verdict with a short explanation. | Compare documented commands with their implementation. |
| **Provider** | The integration that executes AI reviews. Codex CLI or a configured external CLI. | Codex using `gpt-6.1-sol`. |
| **Report** | The saved results of a run, including explanations, detailed output and the files that were verified. | `.please/reports/last.json`. |

A check is deterministic in how Please Behave processes its exit status; its reliability
still depends on the command you choose. A review is an LLM (large language model)
judgment and can differ between runs. Use checks wherever a command can answer
the question reliably.

## Getting started

### Requirements

- **Git**, with your project already in a Git working tree. Run Please Behave from its
  root directory, not a subdirectory.
- The tools used by your checks, such as Cargo, npm or a linter, on your PATH.
- **An AI provider only if you enable reviews.** Codex reviews need Codex CLI
  and a local login; generic CLI reviews need their configured tool and credentials.
  Checks work without an AI provider. See [provider setup](#configure-the-ai-provider).

The Please Behave executable needs no Rust, Python or Node runtime. Rust is needed to
build it from source; Python and Node are used by Please Behave's development tooling.

### Install a release

When a compatible Please Behave release is available, download the matching
archive and `SHA256SUMS` from
[GitHub Releases](https://github.com/please-behave/cli/releases):

| Platform | Target in the archive name |
| --- | --- |
| macOS Apple Silicon | `aarch64-apple-darwin` |
| macOS Intel | `x86_64-apple-darwin` |
| Linux x64 | `x86_64-unknown-linux-gnu` |
| Linux ARM64 | `aarch64-unknown-linux-gnu` |

`mise deploy` installs the published Please Behave development governor.
The first Please Behave release is prepared as `0.1.0`; no compatible published
release is claimed here. Build `please` from source until its archives are
published. See [native downloads](docs/releasing.md#native-downloads).

For manual installation, verify the checksum, then extract and install it. Replace
`VERSION` and `TARGET` below with the downloaded archive's values:

```sh
# macOS; use sha256sum instead on Linux and compare with SHA256SUMS.
shasum -a 256 please-vVERSION-TARGET.tar.gz

tar -xzf please-vVERSION-TARGET.tar.gz
mkdir -p "$HOME/.local/bin"
install -m 755 please-vVERSION-TARGET/please "$HOME/.local/bin/please"
```

Ensure `~/.local/bin` is on PATH; for zsh, add
`export PATH="$HOME/.local/bin:$PATH"` to `~/.zshrc` and open a new terminal.
Check the installation with `please --version`.

Linux archives use glibc and are built on Ubuntu 24.04. Downloads are not signed
or notarized. Windows release builds are not provided.

### Build locally

Install **Git**, **stable Rust/Cargo 1.85+** (edition 2024) and a native compiler/linker:
Xcode Command Line Tools on macOS or GCC/Clang on Linux. From a source checkout:

```sh
cargo build --release --locked
./target/release/please --help
```

The executable is `target/release/please`. To install it on your Cargo PATH,
run `cargo install --path . --locked`.

With [mise](https://mise.jdx.dev/), product builds and governor installation are separate:

```sh
mise trust
mise install                  # Install development tools.
mise build                    # Build target/release/please.
mise deploy                   # Install the published Please Behave development governor.
```

`mise install` installs development tools; `mise deploy` installs the Please Behave governor.
Build and install Please Behave separately as shown above. See [Development](#development) for checks and release tasks.

### Initialize your project

In your application's Git root:

```sh
cd /path/to/My_App
please init
please sync
please run
please status
```

`init` creates `.please/` with a passing example check (`git --version`) and
a disabled example review. It requires `.please/` to be absent. Any existing
entry, including an empty directory, file or symlink, fails with exit code 2
before any files or directories are created or modified:
`Please Behave: ERROR: Already initialized (.please/ exists).`
On success it prints `Initialized .please/ (10 files created).`, using the actual
created file count. It does not install hooks or make a Git commit.

`sync` validates configuration and reconciles declared external state after
confirmation. It runs no checks or reviews. The first
`run` should pass without Codex because the example review is disabled.
This confirms that Please Behave works; it does not yet verify your application's quality.

Next, [add a real check](#add-a-check) and replace or remove the example check.
When you need AI judgment, [configure Codex](#configure-the-ai-provider),
[write a review](#add-a-review), validate again, then run `please run`.
Commit your rules and criteria so other users can run the same verification.

## Using Please Behave

### Commands and options

The supported commands are:

| Command | What it does |
| --- | --- |
| `please init` | Initialize a new `.please/` directory once. |
| `please sync` | Validate configuration and reconcile declared state after confirmation; run no checks or reviews. |
| `please run` | Validate, run selected rules and save results; Git-backed points reconcile hooks first. |
| `please status` | Show the last saved report and whether it still matches the working files. |
| `please help` | Show help. |

| Option | Used with | Meaning |
| --- | --- | --- |
| `--yes` | `sync` | Approve the displayed integration changes without a prompt, including in CI. Defaults to off. |
| `--checks-only` | `run` | Run only checks; do not call an AI provider. |
| `--at NAME` | `run` | Select a known execution point (default `pre-push`). |
| `--progress=auto`, `--progress=plain`, `--progress=tty` | `run` | Progress display; defaults to `auto`. Terminal rows update in place; CI and redirected streams use append-only output. |
| `--plan` | `run` | Preview what would run without executing rules or calling the provider. |
| `--json` | `run --plan` | Print the plan or plan errors as JSON. |
| `--details` | `status` | Show full findings and captured check output. |
| `--no-color` | Any command | Disable colors. `NO_COLOR` and redirected output also disable colors. |
| `-h`, `--help` | Any command | Show help. |
| `-V`, `--version` | `please` | Show the version. |

At the selected execution point, enabled checks run first, followed by reviews.
Within each kind, rules run in ID order. Disabled rules are validated but are not
executed or included in results. Selected reviews with identical parameter
environments share a Codex invocation.

If neither checks nor reviews are defined, Please Behave additionally prints
`No rules defined yet.`. A successful empty run does not
establish that your project passed any quality rules.

During execution, each rule shows its short name, description, status and duration.
An interactive terminal replaces running rows with their results. In CI, redirected
output or `--progress=plain`, start and finish messages are appended without cursor
controls. `auto` also uses plain output for `CI=true`, `CI=1` or `TERM=dumb`;
`tty` explicitly requests redraw. Colors are controlled separately. Very narrow
terminals and review batches larger than the screen fall back to plain output. Execution ends with one overall result line;
use `please status --details` for saved output and full diagnostics.

### Run checks without AI

```sh
please run --checks-only
```

The selected execution point must contain checks only. A required review makes
this invocation fail before any rule runs.

This runs selected enabled checks only: no Codex discovery, MCP connections or
review-token usage. The report is explicitly marked as checks-only and omits
reviews. A checks-only PASS says nothing about your review criteria.
The whole configuration is still validated, including disabled and omitted rules.

### Read results

Progress goes to stderr; the final result line goes to stdout. Redirecting
`please run > report.txt` keeps progress visible in the terminal. Raw command
output is saved instead of flooding the terminal.

| Rule result | Meaning |
| --- | --- |
| `PASS` | The check exited successfully, or the AI found the review criterion satisfied with evidence. |
| `FAIL` | The check failed, or the AI found a concrete violation of the criterion. |
| `ERROR` | Verification could not be completed, for example because of a timeout, missing executable, invalid AI output or missing evidence. |
| `NOT_RUN` | A review was skipped because checks failed. It has no review verdict. |

A run succeeds if no blocking failure exists and its input remains stable.
Non-blocking FAIL and provider/review ERROR remain visible and permit overall PASS.
Invalid parameters and missing review prompts still prevent success.
Check ERROR and integrity errors always prevent success. A review's one-line explanation
appears even when it passes. Read more without running anything again:

```sh
please status
please status --details
```

Status uses English labels, including `Overall`, `Mode: Checks only` and
`Codex tokens`. Details add `Selection`, `Required rules` and external rule
`Source` information where applicable.

Status compares the saved **snapshot** (file hashes and Git HEAD recorded during
the run) with the current working files. `CURRENT` means they match, `STALE` means
they changed, and `UNKNOWN` means the comparison could not be completed reliably.
CURRENT describes freshness, not success: a current report can still contain failures.
Please Behave checks the working files, including uncommitted changes; this does not
prove that the staged or pushed commit contains the same content.

For reviews, available input, cached-input and output token counts are recorded
once for the shared provider call. Total tokens are input plus output; cached
input is a subset of input. Missing usage stays unavailable. Please Behave does not
calculate money, remaining account quota or enforce a total token budget.

| Exit code | Meaning |
| --- | --- |
| `0` | Command succeeded, verification passed, or saved results passed and are current. An executable plan also exits 0. |
| `1` | Verification failed, or saved results are stale. |
| `2` | Configuration, execution, provider or report error; unknown freshness; invalid arguments; blocked/invalid plan; declined, unavailable or failed reconciliation. |
| `3` | No saved report exists for the status command. |

### Choose rules and preview a run

Execution points select the rules assigned to a named context. Use `--plan` for
an offline preview of the same selection and prepared review input. The
human-readable plan uses English labels and explanations:

```sh
please run --at pre-push --plan
please run --at pre-push --plan --json
please run --at pre-push
```

JSON previews use schema 3 and always include `status` and `blocking_reasons`.
Configuration or selection errors return `status: "blocked"`, nonempty reasons
and exit code 2; unavailable plan details are omitted. See
[rule sets and execution points](#group-rules-into-rule-sets) for assignments.

### Use a Git hook or CI

The public Please Behave Action is maintained separately in
[please-behave/action](https://github.com/please-behave/action). Its README defines
usage, inputs and supported platforms. It installs an explicitly selected CLI
release, verifies its archive checksum and optionally runs `please run --at` in
the consumer checkout. The Action revision and CLI release version are independent.
No Please Behave Action release is claimed here; a usable Action ref and matching
CLI release assets must exist before adopting it.

The CLI repository installs the pinned published CLI directly with its
checksum-verifying installer in CI; adoption of a released Action remains pending.
See [repository self-integration](docs/self-governance.md#ci-pinning-and-renovate).

Assign enabled Rules to `pre-commit`, `commit-msg` or `pre-push`. Please Behave derives
required hooks from effective assignments, including Rule Set inheritance and
external imports. No hook declaration is needed. Run `please sync` to review
and apply the hook changes.

The three supported Git hooks always invoke their identically named built-in
execution points. This relationship cannot be configured. `check` and custom
execution points are invoked explicitly and have no Git hook.
`commit-msg` receives the final message file, while `pre-commit` runs before it is
available. Please Behave delegates message validation to your configured checker.

`please sync` validates configuration, inspects actual state, plans changes,
asks for confirmation, applies the approved plan and verifies it. Direct execution
of `pre-commit`, `commit-msg` or `pre-push` through `run` does the same first.
Direct `check` and custom points execute without inspecting or synchronizing hooks,
even when required hooks are missing or outdated. Synchronized repositories need
no prompt or writes. Declining, conflicts, failed reconciliation or a required confirmation
without a terminal exit 2 before checks; run `please sync` interactively.
`run --plan` remains a read-only execution preview and does not synchronize.

For unattended setup, explicitly approve declared integration changes:

```sh
please sync --yes
please run --at pre-push
```

Successful `sync` prints `Synchronization completed successfully.` when import
resolution state, cached repositories/revisions or Git hooks changed, or
`Already synchronized.` otherwise. Both outcomes exit 0. Refreshing unchanged
imports alone does not count as a change. Hook changes appear under
`Planned changes:` on stderr, followed by `Apply these changes? [y/N]`.

`--yes` prints and approves the plan without reading stdin or requiring a terminal.
It prints no auto-approval message. Validation, conflict detection and state
verification still apply. It runs no checks or reviews and leaves an existing report
untouched. Synchronized hooks are left untouched. The option is available only on `sync`;
ordinary execution retains its existing confirmation behavior.

Please Behave marks its sections explicitly, preserves unmanaged commands, and refuses
ambiguous ownership or unsafe composition. Hooks no longer required by
effective Rule assignments lose only managed content. Generated hooks verify
synchronization read-only and direct you to
`please sync` on drift. Checks cannot start another Please Behave execution or run
`please sync` in the same repository; this also applies to direct CLI runs.
Local hooks are bypassable and verify working files,
which may differ from staged or pushed commits. See
[Git integration and recovery](docs/usage.md#optional-git-hook).
For explicit governance execution in a fresh CI checkout, no hook setup is needed:

```sh
please run --at check
```

Custom points such as `release` behave the same way. These commands work locally
or from any workflow; Please Behave neither generates nor validates CI workflows.
Checks-only runs need no AI credentials. Please Behave does not provision Codex login on
hosted runners, and personal login credentials should not be copied into CI.

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

[execution_points]
custom = ["release"]
```

| Field | Required / default | Meaning |
| --- | --- | --- |
| `project` | Required | Nonblank display name. |
| `max_display_files` | Optional; `5` | Integer `1..5`: changed paths shown in status. |
| `execution_points` | Optional table | Additional execution contexts. |
| `execution_points.custom` | Optional; `[]` | Unique names of 1–80 lowercase ASCII letters, digits, `-`, `_`, starting with a letter/digit. Built-in names are reserved. |
| `imports` | Optional; `[]` | External references, declared only here as `[[imports]]`. |

Built-in points are `pre-commit`, `commit-msg`, `pre-push` and `check`. The first
three use Git hooks. `check` is invoked explicitly with `please run --at check`
and never triggered automatically. It needs no declaration or CI workflow.
Consumers decide where to invoke it, including multiple workflows; Please Behave does
not generate workflows, discover them or validate CI integration. Declare custom
points in the consumer, including unmapped custom points used by imports.
`release` is an ordinary custom point. In this repository it runs formatting,
Clippy, Rust tests, a native build and an executable smoke test after version
stamping and before packaging. GitHub Actions handles packaging, collected
artifact verification and publication; see the [release sequence](docs/releasing.md#release-sequence).
The default execution point is `pre-push`.

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
Checks and AI review provider subprocesses also receive `PLEASE_EXECUTION_POINT`,
the effective execution point (for example, `pre-commit`, `commit-msg`, `pre-push`,
`check` or a configured custom point). Please Behave supplies this runtime context from
the validated execution plan and overrides inherited values. It is not a
configurable rule parameter and has no TOML field. Local and imported rules
receive the same context; mapped imports receive the consumer execution point.

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
[response contract](SPECIFICATION.md#out-001-structured-review-outcomes) applies.
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
the existing [external source trust model](docs/external-rule-sets.md#source-resources-and-trust):
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

Checks precede reviews, ordered by ID. Empty points succeed. Every scheduled rule
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

## Limits and unsupported features

These limits are built in, not additional configuration fields:

| Area | Limit or behavior |
| --- | --- |
| Configuration metadata | 64 KiB per metadata file; at most 256 rules and 256 Rule Sets including imports. |
| External Git sources | 64 KiB resolution state; cached repositories: 8 MiB/file, 128 MiB total, 100,000 entries, 128 directory levels. Fetch timeout 60 seconds, local Git operations and lock waits 30 seconds. |
| Review criteria | 32 KiB per prompt file; 256 KiB combined, including disabled reviews. |
| Initial AI hints | Up to 200 file-list entries and 32 KiB of diff, with truncation indicated. Necessary evidence still needs inspection. |
| Snapshot | 8 MiB per file, 128 MiB total, 100,000 paths; Git file enumeration capped at 64 KiB; `.please` nesting limited to 16 levels. |
| Snapshot coverage | Tracked and nonignored untracked files, plus `.please` even if ignored. `.git` and `.please/reports` are excluded. Unsupported symlinks, submodules, paths or exceeded limits prevent PASS. |
| Input stability | Working files and HEAD must match before/after execution and before every AI call. Staging and pushed commits are not separately verified. |
| AI response | 1 MiB final JSON document; summaries and findings have the limits described under reviews. |
| Saved report input | 128 MiB maximum when reading a saved report. |
| Token usage | Available counters are summed across review batches. Optional reasoning/cache-write counters are subsets, not additions to the total. Missing telemetry remains unavailable. |

Not currently supported: per-review model/MCP overrides,
reviews restricted to a diff, staged/index or commit input, configurable run-wide
token budgets, monetary cost estimates and cached results. A token-efficient
shared call is not a result cache or a guaranteed prompt-cache hit. Planned work
is listed in the [TODOs](docs/todo/README.md).

## Development

Please Behave governs its own development using a separately installed release.
`.please-version` pins the governor, and `.please/` defines its rules.
`mise deploy` installs that exact release; builds under `target/` are candidates.
See [repository self-integration](docs/self-governance.md) for setup and coverage.
The `repository-conventions` Rule Set is pinned to a full Git commit and checks
LICENSE against its permitted reference texts at `pre-push` and `check`.

The [AGENTS.md quality-review example](docs/agents-quality-review.md) combines a
blocking 1200-word limit with a conditional, non-blocking Gemini review at `check`.
It receives the complete document and rubric; provider errors remain visible in reports.
Git hooks need no Gemini credentials.

This repository's deterministic quality checks run through an installed published
Please Behave release. Install development tools with `mise install`, dependencies with
`npm ci --ignore-scripts` and `cargo +stable fetch --locked`, then the governor
with `mise deploy`. Install Gitleaks v8 with the `dir` command (tested with
8.30.1) explicitly for the imported Security Rule Set. Prepare Rust 1.85 and actionlint as described in the setup guide. Ensure Please Behave,
Gitleaks and actionlint are on PATH.

Install the pre-commit, commit-msg and pre-push hooks explicitly with
`mise hooks:install`. Commit and push then invoke Please Behave automatically: pre-commit
checks formatting and documentation, and pre-push runs the complete local suite.
Agents rely on this feedback and must not manually run checks, tests, linters or
builds. They may format source and inspect saved results with
`please status --details`.

For developers who want an explicit manual run, `mise check` delegates to the
pinned published governor at the `pre-push` execution point:
formatting, documentation validation, Clippy, Rust tests, Python helper tests,
JavaScript release tests, workflow validation, minimum Rust, an optimized
candidate build, smoke test and the imported
Security and repository-conventions Rule Sets against `HEAD`. No reviews or real provider calls run. The governor never comes from the candidate build.

`mise docs:check` checks links, specification references, configuration fields and
executable documentation examples. Individual tasks remain available: `format`,
`format:check`, `lint`, `test`, `smoke`, `hooks:test` for manual development use.
Local hooks verify working files; CI checks clean checkouts.

See [repository quality gates](docs/self-governance.md) for setup, every execution point,
the Renovate-managed CI version pin, reports and hook limitations. Accepted commit
types, scopes and release effects are in [Contributing](CONTRIBUTING.md#commits).
`mise run TASK` works for every task; `mise format` formats Rust, whereas the
built-in `mise fmt` formats `mise.toml`.

`mise release` uses an authenticated `gh` CLI to start the release workflow for
**remote main**, then returns. It does not commit/push local work or wait for
publication. Please Behave starts at 0.1.0; features and breaking changes increment minor
during 0.x, fixes and maintenance increment patch. Semantic Release handles
subsequent versions, tags, release notes and native archives;
see [Releasing](docs/releasing.md).

[Usage and Git hooks](docs/usage.md) · [Design and limits](docs/design.md) ·
[macOS application example](examples/my-app/README.md) ·
[Contributing](CONTRIBUTING.md) · [GitHub Releases](https://github.com/please-behave/cli/releases) · [PolyForm Noncommercial License 1.0.0](LICENSE)

## Frequently Asked Questions

### Is Please Behave available for Windows?

Windows release builds are not provided, and external Git rule-set imports are
unsupported on Windows because shared-cache locking currently requires Unix.
See [supported native targets](docs/releasing.md#native-downloads).
