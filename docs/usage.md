# Usage

The [specification](../SPECIFICATION.md) is the contract for behavior described here;
the [configuration reference](configuration.md) defines every field.

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
selected reviews if no blocking check failed. Blocking failures and execution errors
prevent review token spending; those reviews are `NOT_RUN`. Disabled rules never
enter results.

`please run --checks-only` runs the checks of an execution point without calling any
provider. Points that require reviews reject it. Reports and `status` identify the
limited scope: PASS then means the selected checks passed, not that reviews did.

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

A valid run writes `.please/reports/last.json` atomically, including on FAIL or ERROR.
Invalid configuration leaves the previous report untouched. Reports are ignored and
stay local.

Progress appears on stderr: each rule shows its display name (optional `name` in
`rule.toml`, default the rule ID), then its result and duration. `--progress=auto`
redraws in an interactive terminal and appends plain logs in CI, with `TERM=dumb` or
with redirected stderr; `plain` and `tty` select a mode explicitly. Reviews sharing a
provider reference and parameter environment share one provider call. A final line
summarizes status, counts and duration, for example
`FAIL · 3 rules · 1 failed · 1 not executed · 2.0 s`. Colors follow `NO_COLOR`,
`--no-color` and each stream's terminal state.

`please status` reads the saved report without executing anything and reports
CURRENT, STALE or UNKNOWN by comparing the saved snapshot with the working tree.
Line deltas are unavailable because only hashes are stored. `--details` adds full
findings, captured output (bounded per stream and not redacted, so avoid printing
secrets), selection, required rule IDs, external sources and token usage.

## Configure checks and reviews

A Check lives in `.please/checks/<name>/rule.toml`. Prefer a command string such as
`[check] command = "python3 scripts/check-docs.py"`; quoting follows the configuration
reference and no shell is involved. The consumer root is the working directory and
`PLEASE_RULE_DIRECTORY` locates bundled files. Exit 0 is PASS, 1 is FAIL and 2 is
ERROR; signals, missing scripts and launch failures are ERROR. A completed non-blocking
FAIL permits success. Checks are trusted code without sandboxing, and changing
evaluated files during a run prevents PASS.

For an optional AI review, configure an external adapter, customize the disabled
example review, then enable it. Please Behave ships no provider and supplies no
default. See [provider configuration](configuration.md#configure-the-ai-provider)
and the [adapter protocol](adapter-protocol.md).

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Initialization/validation/help succeeded, or all required results PASS and status CURRENT |
| 1 | Verification FAIL, or status STALE |
| 2 | Configuration/reconciliation/execution/provider/report ERROR, declined or unavailable confirmation, status UNKNOWN, or invalid CLI arguments |
| 3 | Status requested before any report exists |

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

For unattended setup, `please sync --yes` approves the displayed plan without a
terminal, manages declared integration state only and is a no-op when synchronized:

```sh
please sync --yes
please run --at pre-push
```

Checks receive `PLEASE_GIT_HOOK`, `PLEASE_GIT_HOOK_ARG_COUNT` and one-based
`PLEASE_GIT_HOOK_ARG_1`, `PLEASE_GIT_HOOK_ARG_2` and so on, without shell expansion.
Stdin stays null, so pre-push reference data is not forwarded. For `commit-msg`,
create `.please/checks/commit-message/rule.toml` with
`[check] command = "sh .please/checks/commit-message/index.sh"` and read the message
path in `index.sh`:

```sh
#!/bin/sh
set -eu
test "$PLEASE_GIT_HOOK" = commit-msg
test "$PLEASE_GIT_HOOK_ARG_COUNT" = 1
exec my-message-checker "$PLEASE_GIT_HOOK_ARG_1"
```

A checker must not invoke Please Behave again; a repository-scoped guard blocks nested
execution.

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
