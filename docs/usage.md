# Usage

[Why](../why/README.md) defines binding behavior and API contracts.
The [configuration guide](configuration.md) explains setup and links to exact field
definitions, defaults and bounds.

## First run

From the root of a Git repository (an unborn repository from `git init` works):

```sh
please init
please sync
please run
please status
please status --details
```

`init` creates `.please/` only when it does not exist. Any existing entry fails with
exit code 2 before any writes. It installs no hooks, stages no files and makes no
commits. The enabled example check passes without an adapter; the deliberately failing
check and the example review are disabled. To demonstrate failure, set
`enabled = true` in `.please/checks/deliberately-failing/rule.toml` and run `please run`.

Checks and reviews are collectively called **rules**. `sync` validates all files,
even disabled entries, and reconciles declared external state after confirmation,
without running rules. `run` validates, executes the selected enabled checks, then the
selected reviews if no check failed or errored. Check failures and execution errors
prevent review token spending; those reviews are `NOT_RUN`. Disabled rules never
enter results.

`please run --checks-only` runs the checks of an execution point without calling any
adapter. Selected reviews are reported `NOT_RUN`. Reports and `status` identify the
limited scope: PASS then means the selected checks passed, not that reviews did. Use it
for local development, never in workflows.

## Select and preview rules

```sh
please run --at pre-push --plan
please run --at pre-push --plan --json
please run --at pre-push
```

The default point is pre-push. All selected rules are mandatory, including reviews.
See [execution points and plans](execution-points-and-plans.md). Checks run
concurrently by default; configure `[execution] max_parallel`, override it with
`--max-parallel`, and declare a shared `check.resource` for checks that share mutable
state.

## Run results

A valid run atomically saves a new numbered report `.please/reports/<number>.json`, including on FAIL or ERROR, and names its number in the summary line. `report.retain_runs` (default 10) keeps the newest reports; numbering continues after removal.
The [Verification Report API](../why/contracts/report/api.yaml) defines its
format and [snapshot freshness](../why/contracts/report/snapshot.md).
Each review result names the adapter profile that produced it; reviews using the default
adapter profile or making no adapter call name none. Invalid configuration saves no report and leaves saved reports untouched. Reports are ignored and
stay local.

Progress appears on stderr: each rule shows its display name (optional `name` in
`rule.toml`, default the rule ID), then its result and duration. `--progress=auto`
redraws in an interactive terminal and appends plain logs in CI (`CI=true` or `1`, or GitHub Actions), with `TERM=dumb` or
with redirected stderr; `plain` and `tty` select a mode explicitly. Each review is one
adapter call. A final line
summarizes status, counts and duration, for example
`FAIL · 3 rules · 1 failed · 1 not executed · 2.0 s`. Unless the run passes, a short
failure summary follows: the number of rules that prevented success and up to three
lines of cause for the first one, from its review result or its captured error output
(standard output as fallback), with a pointer to `please status --details`. Colors follow
`NO_COLOR`, `--no-color` and each stream's terminal state.

`please status` reads the saved report without executing anything and reports
CURRENT, STALE or UNKNOWN by comparing the saved snapshot with the working tree.
Line deltas are unavailable because only hashes are stored. `--details` adds full
findings, captured output (bounded per stream and not redacted, so avoid printing
secrets), selection, required rule IDs, external sources and token usage.

## Publish a report

Set up [publishing](configuration.md#publish-settings) and supply the API key in
the configured environment variable through your secret store. From the
repository root, pass your gateway origin explicitly after a run.

```sh
please publish --gateway-url https://gateway.example.invalid
```

Replace the placeholder with an HTTP or HTTPS origin. Use HTTPS for a remote
gateway. An optional port or trailing slash is allowed. The origin must be
free of credentials, additional path segments, a query and a fragment.
`--gateway-url` is required on every invocation. The newest report must have
a commit and show CURRENT in `please status`. A current report from a failed
run can be published.

The gateway receives the repository identity, commit, execution point, report
schema version, overall verdict and each result's ID, kind and status. Captured
output, findings and explanations stay local. The command exits according to
delivery and preserves the report. In CI, run publication explicitly after
capturing the run's exit status, including when the run fails. Your workflow
decides how to handle a delivery failure.

## Configure checks and reviews

A Check lives in `.please/checks/<name>/rule.toml`. Prefer a command string such as
`[check] command = "python3 @rule/check.py"`; quoting follows the configuration
reference and no shell is involved. The consumer root is the working directory and
`@rule/` references locate bundled files. Exit 0 is PASS, 1 is FAIL and 2 is
ERROR; signals, missing scripts and launch failures are ERROR. A check FAIL prevents success. Checks are trusted code without sandboxing, and changing
evaluated files during a run prevents PASS.

For an optional AI review, configure an external adapter, customize the disabled
example review, then enable it. Please Behave ships no adapter profile and supplies no
default. See [adapter profile configuration](configuration.md#configure-the-adapter-profile)
and the [adapter protocol](adapter-protocol.md).

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Initialization/validation/help succeeded, plan preview is executable, or all required results PASS and status CURRENT |
| 1 | Verification FAIL, or status STALE |
| 2 | Configuration/reconciliation/execution/adapter/report ERROR, plan preview blocked by a configuration or selection error, declined or unavailable confirmation, status UNKNOWN, or invalid CLI arguments |
| 3 | Status requested for a report that does not exist |

For `please publish`, exit 0 means the gateway accepted the attestation and exit
2 means delivery did not succeed, including failed local prerequisites.

Status preserves a CURRENT report's original FAIL/ERROR exit code. UNKNOWN takes
precedence if snapshot coverage is incomplete.

## Optional Git hook

Please Behave supports three fixed Git hooks: `pre-commit`, `commit-msg` and
`pre-push`. Each invokes its identically named execution point. Required hooks are
derived automatically from enabled effective rule assignments, including Rule Set
inheritance and imports; run `please sync` to reconcile them. `check` and custom
points have no hook and run explicitly, without hook inspection, even in a fresh CI
checkout. All hook failures block Git, including a missing `please` on PATH.

Reconciliation follows **Validate -> Inspect -> Plan -> Confirm -> Apply -> Verify**.
`sync` and a direct `run` of a Git-backed point perform it before any rule runs.
Synchronized state needs no prompt and no write; changes are displayed and need
confirmation in a terminal. Declining, a missing terminal, conflicts or failures exit 2
and preserve the previous report. Inside a hook, reconciliation is read-only: drift
fails with an instruction to run `please sync`. `--plan` never synchronizes.

Importing from a Git repository additionally requires that you trust it with
`please trust REPOSITORY`; `--yes` never grants trust. See
[Trusting repositories](external-rule-sets.md#trusting-repositories).

For unattended setup, `please sync --yes` approves the displayed plan without a
terminal, manages declared integration state only and is a no-op when synchronized:

```sh
please sync --yes
please run --at pre-push
```

Hook arguments are not exported. Stdin stays null; the staged files, commit message and
pushed references are available in the [run context](configuration.md#run-context), whose
path a check receives as `@context`. For `commit-msg`, create
`.please/checks/commit-message/rule.toml` with
`[check] command = "sh @rule/index.sh @context"` and read the message path in `index.sh`:

```sh
#!/bin/sh
set -eu
message=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["commit_message"]["file"])' "$1")
exec my-message-checker "$message"
```

A checker must not invoke Please Behave again; a repository-scoped guard blocks nested
execution and synchronization before any writes, including import refreshes. Checks
may invoke Please Behave in other repositories.

### Ownership and recovery

Managed hooks use explicit `# BEGIN PLEASE MANAGED` / `# END PLEASE MANAGED` markers
and an ownership line bound to the common Git repository. An existing executable
`#!/bin/sh` hook receives a section after the shebang and keeps its remaining content.
Other shebangs, nonexecutable custom hooks, malformed or duplicate markers, symlinks
and foreign ownership are conflicts: Please Behave reports the file and stops before
any write. Resolve the conflict manually, then rerun `please sync`. Never copy managed
wrappers between repositories; each clone generates its own.

Git resolves the effective hook directory, including `core.hooksPath` and worktrees.
Custom directories must be inside the working tree. Linked worktrees share the common
hooks directory and must agree on mappings. Changing or removing mappings updates or
removes only managed sections. Multi-file changes are not one atomic transaction: after
an interruption, inspect the reported paths and rerun `please sync`, and remove leftover
`.please-hook-*` files once no other reconciliation is running.

Local hooks are bypassable (`--no-verify`) and verify the working tree, not the exact
staged index or pushed commit. CI can run the same execution points on a clean
checkout. Development of Please Behave itself uses a separately installed, published
governor; see [repository self-integration](self-governance.md).
