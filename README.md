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
| **Provider** | The integration that executes AI reviews. An external executable adapter. | A project-owned review adapter. |
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
- **An adapter only if you enable reviews.** Configure an external executable
  implementing the [adapter protocol](docs/adapter-protocol.md). Checks work without
  any provider. See [provider setup](docs/configuration.md#configure-the-ai-provider).

The Please Behave executable needs no Rust, Python or Node runtime. Rust is needed to
build it from source; Python and Node are used by Please Behave's development tooling.

### Install a release

Download the archive matching the release version and `SHA256SUMS` from
[GitHub Releases](https://github.com/please-behave/cli/releases):

| Platform | Target in the archive name |
| --- | --- |
| macOS Apple Silicon | `aarch64-apple-darwin` |
| macOS Intel | `x86_64-apple-darwin` |
| Linux x64 | `x86_64-unknown-linux-gnu` |
| Linux ARM64 | `aarch64-unknown-linux-gnu` |

From this source checkout, `mise deploy` installs the published Please Behave
release pinned in `mise.toml`, with checksum and executable verification.
See [native downloads](docs/releasing.md#native-downloads).

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
mise install                  # Install development tools and the published governor.
mise build                    # Build target/release/please.
```

`mise install` installs development tools and the Please Behave governor.
Use the source installation above only when you need a candidate build.
Repository governance uses the published release selected by `mise exec` or shell activation.
See [Development](#development) for checks and release tasks.

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
On success it prints `Initialized .please/ (9 files created).`, using the actual
created file count. It does not install hooks or make a Git commit.

`sync` validates configuration and reconciles declared external state after
confirmation. It runs no checks or reviews. The first
`run` should pass without an adapter because the example review is disabled.
This confirms that Please Behave works; it does not yet verify your application's quality.

Next, [add a real check](docs/configuration.md#add-a-check) and replace or remove the example check.
When you need AI judgment, [configure an adapter](docs/configuration.md#configure-the-ai-provider),
[write a review](docs/configuration.md#add-a-review), validate again, then run `please run`.
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
| `--fail-fast` | `run` | Stop scheduling after a blocking failure or fatal execution error; defaults to off. |
| `--max-parallel N` | `run` | Positive maximum concurrent checks; overrides configuration (default `4`). |
| `--at NAME` | `run` | Select a known execution point (default `pre-push`). |
| `--progress=auto`, `--progress=plain`, `--progress=tty` | `run` | Progress display; defaults to `auto`. Terminal rows update in place; CI and redirected streams use append-only output. |
| `--plan` | `run` | Preview what would run without executing rules or calling the provider. |
| `--json` | `run --plan` | Print the plan or plan errors as JSON. |
| `--details` | `status` | Show full findings and captured check output. |
| `--no-color` | Any command | Disable colors. `NO_COLOR` and redirected output also disable colors. |
| `-h`, `--help` | Any command | Show help. |
| `-V`, `--version` | `please` | Show the version. |

At the selected execution point, enabled checks run first, followed by reviews.
Checks dispatch by estimated remaining dependency-path duration and may finish out of order.
Reports retain ID order within each kind. Disabled rules are validated but are not
executed or included in results. Selected reviews with identical parameter
environments share an adapter invocation.

If neither checks nor reviews are defined, Please Behave additionally prints
`No rules defined yet.`. A successful empty run does not
establish that your project passed any quality rules.

During execution, each rule shows its short name, description, status and duration.
An interactive terminal shows pending, resource-waiting, running and completed
rows, running elapsed times and completed/total and running counts. Rule numbers
follow actual start order and stay with the rule through completion; they are
separate from the completed count. Started rows appear first in start order.
Pending, waiting and never-started `NOT_RUN` rules have no start number. Plain
output uses the same numbering and reports counts after each final result.
Plans and saved results retain their canonical order. Refreshes are limited to
ten per second; short terminals show selected rows with hidden counts. In CI,
GitHub Actions, redirected stderr or `--progress=plain`, named starts, resource
waits and results are appended without cursor controls, even with `--progress=tty`.
`TERM=dumb` and very narrow or short terminals also use plain output. Colors are
controlled separately. Execution ends with one overall result line;
use `please status --details` for saved output and full diagnostics.

### Run checks without AI

```sh
please run --checks-only
```

This runs the selected enabled checks only: no adapter processes or
review-token usage. The report is explicitly marked as checks-only and lists every
selected review as `NOT_RUN`; they do not affect the overall result. A checks-only PASS
says nothing about your review criteria. It is meant for local development, not for
workflows.
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
| `NOT_RUN` | A rule was not started because a prerequisite did not pass, the check phase failed, or fail-fast stopped scheduling. The reason explains why. |
| `CANCELLED` | A running check was interrupted after fail-fast triggered; captured partial output is retained. |

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
`Adapter tokens`. Details add `Selection`, `Required rules` and external rule
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
[rule sets and execution points](docs/configuration.md#group-rules-into-rule-sets) for assignments.

### Use a Git hook or CI

The public Please Behave Action is maintained separately in
[please-behave/action](https://github.com/please-behave/action). Its README defines
usage, inputs and supported platforms. It installs an explicitly selected CLI
release, verifies its archive checksum and optionally runs `please run --at` in
the consumer checkout. The Action revision and CLI release version are independent.
The CLI repository pins the Action to a full commit in CI and native release workflows.
Mise installs the governor pinned in `mise.toml`; one Action step with
`install: 'false'` and `sync: 'true'` synchronizes governance and executes the gate.
The Action owns GitHub review baselines and generic review summaries. Workflows run
the full execution point; `--checks-only` and the Action's `checks-only` input are for local development.
CI uploads the CLI report; local wrapper invocations also retain separate execution metadata.
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
Checks-only runs need no AI credentials. Please Behave does not provision provider authentication on
hosted runners, and personal login credentials should not be copied into CI.

## Configuration

Configuration lives in `.please/`: `config.toml` for project settings, one
`rule.toml` per Check or Review, optional Rule Sets and imports, and `reports/` for
saved results. The [configuration reference](docs/configuration.md) is the single
source for every field, default, limit and example. `please init` writes the same
guide to `.please/README.md`.

A minimal Check, `.please/checks/readme/rule.toml`, runs at pre-push and in CI:

```toml
id = "repository-readme"
description = "Require a nonempty README"
run_at = ["pre-push", "check"]

[check]
command = "test -s README.md"
```

Reference topics:

- [Files and validation](docs/configuration.md#files-and-validation)
- [Project settings](docs/configuration.md#project-settings)
- [Fields shared by Checks and Reviews](docs/configuration.md#fields-shared-by-checks-and-reviews) and [Rule parameters](docs/configuration.md#rule-parameters)
- [Add a Check](docs/configuration.md#add-a-check)
- [Configure the AI provider](docs/configuration.md#configure-the-ai-provider) and [add a Review](docs/configuration.md#add-a-review)
- [Group Rules into Rule Sets](docs/configuration.md#group-rules-into-rule-sets)
- [Import Rule Sets](docs/configuration.md#import-rule-sets-from-git) and [providers](docs/configuration.md#import-providers-from-git) from Git
- [Reports and other files](docs/configuration.md#reports-and-other-files)

## Limits and unsupported features

These limits are built in, not additional configuration fields:

| Area | Limit or behavior |
| --- | --- |
| Configuration metadata | 64 KiB per metadata file; at most 256 rules and 256 Rule Sets including imports. |
| External Git sources | 64 KiB resolution state; cached repositories: 8 MiB/file, 128 MiB total, 100,000 entries, 128 directory levels. Fetch timeout 60 seconds, local Git operations and lock waits 30 seconds. |
| Review criteria | 32 KiB per prompt file; 256 KiB combined, including disabled reviews. |
| Initial AI hints | Up to 200 file-list entries and 32 KiB of diff, with truncation indicated. Necessary evidence still needs inspection. |
| Snapshot | 8 MiB per file, 128 MiB total, 100,000 paths; Git path-list output capped at 128 MiB; `.please` nesting limited to 16 levels. |
| Snapshot coverage | Tracked and nonignored untracked files, plus declared governance inputs even if ignored. Unreferenced ignored artifacts, `.git` and `.please/reports` are excluded. Unsupported symlinks, submodules, paths or exceeded limits prevent PASS. |
| Input stability | Working files and HEAD must match before/after execution and before every AI call. Staging and pushed commits are not separately verified. |
| AI response | 64 KiB adapter stdout; summaries and findings have the limits described under reviews. |
| Scheduling history | 1 MiB local cache; oversized or invalid input uses defaults, and oversized updates are not saved. |
| Saved report input | 128 MiB maximum when reading a saved report. |
| Token usage | Available counters are summed across review batches. Optional reasoning/cache-write counters are subsets, not additions to the total. Missing telemetry remains unavailable. |

Declared governance inputs include local configuration documents, rule prompts and
entrypoints, review entry/context files, and existing local files named by check
commands, provider commands/arguments or parameter values. They remain covered even
when ignored, including disabled definitions. Check path references retain their
declared boundaries. Transitive script dependencies are not inferred; ignored
caches such as `.please/adapters/__pycache__/` are excluded unless explicitly named.

Not currently supported: reviews restricted to a diff, staged/index or commit input, configurable run-wide
token budgets, monetary cost estimates and cached results. A token-efficient
shared call is not a result cache or a guaranteed prompt-cache hit. Planned work
is listed in the [TODOs](docs/todo/README.md).

## Development

Please Behave governs its own development using a separately installed release.
`mise.toml` pins the governor, and `.please/` defines its rules.
`mise install` installs that exact release; builds under `target/` are candidates.
See [repository self-integration](docs/self-governance.md) for setup and coverage.
The `rulesets/multi-point/open-source-software` Rule Set is pinned to a full Git commit and checks
LICENSE against its permitted reference texts at `pre-push` and `check`.

The imported repository-conventions Rule Set combines a blocking AGENTS.md size limit with a
conditional, non-blocking Gemini quality review at `check`; provider errors remain visible
in reports and Git hooks need no Gemini credentials.

This repository's deterministic quality checks run through an installed published
Please Behave release. Run `mise install` for the configured rule tools (including
Gitleaks, actionlint, the published governor and both Rust toolchains), then
`mise run ci:prepare` (alias `setup`) to fetch locked dependencies. Git and a
native compiler/linker remain host prerequisites. All mise tool versions, including
the Rust toolchain, are pinned and updated by Renovate; Rust 1.85.0 independently
verifies minimum compatibility.
See the [setup guide](docs/self-governance.md#local-setup).

Use `mise exec -- please run --at pre-commit` or `mise exec -- please run --at pre-push`
for manual execution without shell activation. Use `mise exec -- git commit` and
`mise exec -- git push` to give hooks the same tools. Plain commands require mise
shell activation or equivalent PATH setup; `mise install` alone does not activate
tools. The separately installed governor must still be on PATH.

Install the pre-commit, commit-msg and pre-push hooks explicitly with
`please sync --yes`. Commit and push then invoke Please Behave automatically: pre-commit
checks formatting and documentation, and pre-push runs the complete local suite.
Agents rely on this feedback and must not manually run checks, tests, linters or
builds. They may format source and inspect saved results with
`please status --details`.

For developers who want an explicit manual run, `please run --at pre-push` runs the
pinned published governor at the `pre-push` execution point:
formatting, documentation validation, Clippy, Rust tests, Python helper tests,
JavaScript release tests, workflow validation, minimum Rust, an optimized
candidate build, smoke test and the imported
Security and open-source-software Rule Sets against `HEAD`. No reviews or real provider calls run. The governor never comes from the candidate build.

`mise docs:check` checks links, specification references, configuration fields and
executable documentation examples. Individual tasks remain available: `format`,
`format:check`, `lint`, `test`, `smoke` for manual development use.
Local hooks verify working files; CI checks clean checkouts.

See [repository quality gates](docs/self-governance.md) for setup, every execution point,
the Renovate-managed CI version pin, reports and hook limitations. Accepted commit
types, scopes and release effects are in [Contributing](CONTRIBUTING.md#commits).
`mise run TASK` works for every task; `mise format` formats Rust, whereas the
built-in `mise fmt` formats `mise.toml`.

CI uses `mise install` through the mise Action and `mise run ci:prepare`; preparation
only fetches locked dependencies. Release jobs install only their required tools.
`mise run build --offline` builds a prepared native candidate; `mise run release:package`
packages it after smoke validation. Actions owns stamping, receipts, archive checks
and publication.

`mise release` uses an authenticated `gh` CLI to start the release workflow for
**remote main**, then returns. It does not commit/push local work or wait for
publication unless `--install` is given (see [Releasing](docs/releasing.md)). Please Behave starts at 0.1.0; features and breaking changes increment minor
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
