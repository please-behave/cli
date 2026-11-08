# Usage

## First run

From the root of a Git repository (an unborn repository from `git init` works):

```sh
please init
please sync
please run
please status
please status --details
please help
please --help
please --version
```

`init` creates `.please/` (lowercase) only when it does not exist. Any existing
entry, including an empty directory, file or symlink, fails with exit code 2 and
`Please Behave: ERROR: Already initialized (.please/ exists).` before any writes.
Success prints `Initialized .please/ (9 files created).` to stdout, using the
actual created file count. It never installs hooks, stages files or commits.
The enabled example uses `git --version`
and passes without an adapter. The deliberately failing check and example review are
disabled. To demonstrate failure, change `enabled = false` to `enabled = true`
in `.please/checks/deliberately-failing/rule.toml` and run `please run`.

Checks and reviews are collectively called **rules**. If neither is defined,
`run` prints `No rules defined yet.` alongside the
report. Defined but disabled rules do not trigger this message.

`sync` validates all files, even disabled entries, and reconciles declared
external state after confirmation, without running checks or reviews.
`run` validates, then executes selected enabled checks in ID order, then
selected active reviews in batches sharing provider references and parameter
environments if checks passed.
It selects rules assigned to the requested execution point. Direct Git-backed points reconcile hooks first;
`check` and custom points execute without hook synchronization. Disabled items
never enter results.
Any blocking failed check or execution error prevents spending review tokens; those reviews are `NOT_RUN`.

Use `please run --checks-only` for an execution point containing checks only.
Points requiring reviews reject this option before running any checks.
The full configuration is still validated, but no review or LLM provider is called;
Please Behave incurs no review-token usage. Progress counts only checks. Reviews are
excluded from results, and both the JSON (`checks_only: true`) and saved status
clearly identify the limited scope. PASS means the selected checks passed, not
that reviews passed. If no checks are enabled, the report explicitly says so.
The flag applies only to that invocation; an ordinary run still includes reviews.
No review result is cached or reused. Earlier reports default to the full scope.

## Select and preview rules

Use Rules and Rule Sets to assign named execution points:

```sh
please run --at pre-push --plan
please run --at pre-push --plan --json
please run --at pre-push
```

The default point is pre-push. All selected rules are mandatory, including
reviews. Checks-only rejects a point requiring reviews. Empty points succeed.
See [execution points and plans](execution-points-and-plans.md) and the
[configuration reference](configuration.md).

Checks run concurrently by default, up to four at a time. Configure
`[execution] max_parallel` or override it with `please run --at check --max-parallel 6`.
Use 1 for serial execution. Checks sharing mutable state must declare the same
`check.resource`; see the [configuration reference](configuration.md#project-settings).

## Run results

Execution errors and blocking failures prevent success. Non-blocking FAIL stays visible. A valid run writes `.please/reports/last.json`
atomically, including on FAIL/ERROR. Invalid configuration leaves the previous
report untouched. Reports are ignored and remain local.

During `run`, progress appears immediately on stderr. Each rule shows its
short display name, description, `RUNNING` state and then its result and duration.
Set optional `name` in rule.toml to choose a label; its default is the original
rule ID. Imported hashes and binding identities are confined to JSON and details.
Duplicate labels receive a short origin suffix and an occurrence number.

`--progress=auto` is the default. Interactive terminals replace running rows with
results, show pending and resource waits, refresh running elapsed times and show
completed/total counts. Short terminals show selected rows and hidden counts.
`CI=true`, `CI=1`, `GITHUB_ACTIONS=true`, `TERM=dumb` and redirected stderr use
append-only logs, even with `--progress=tty`. `--progress=plain` explicitly selects
logs. Very narrow or short terminals also use plain output. Color settings
are independent: `NO_COLOR` removes colors without disabling terminal redraw.
Disabled rules are excluded from the total. Skipped reviews show NOT_RUN without
a running message. Internal preparation and successful process-exit messages are
hidden. Failed rules show their reason; full findings are available with `status --details`; raw command output remains
in the saved report.

Reviews with the same provider reference and identical resolved parameter
environments share one provider call.
The number of calls is announced as `provider call` or `provider calls`;
review starts appear together and verdicts arrive when the review phase completes.
Numbering is rule order and does not
claim per-review model progress. Review verdict summaries remain in saved status
and details. Reports retain full reasons, findings, paths and captured output.

Execution ends with one status/count/duration line on stdout, without repeating
the results in another table. Counts use `rules`, `failed`, `errors` and
`not executed`, for example `FAIL · 3 rules · 1 failed · 1 not executed · 2.0 s`.
Empty execution points print `No rules executed`.
Integrity errors such as changing input files remain visible. `status`
lists saved results without rerunning them. Both streams respect `NO_COLOR`,
`--no-color` and their own TTY state.

Use `please status --details` to inspect full saved findings and captured
command output separately. It does not run adapters or any check again. Saved reports
remain readable; only `run` obtains new provider results. Status labels are
English: `Overall` summarizes results, `Mode: Checks only` identifies saved
scope, and `Adapter tokens` shows available usage. Details add selection, required
rule IDs, external sources and token counter breakdowns where available.

`status` reads that report without executing checks or adapters. It reports
CURRENT, STALE or UNKNOWN, with up to five paths and exact changed-file counts
when both snapshots are complete. Line deltas are unavailable: Please Behave stores
hashes, not prior file content. HEAD-relative diffs are never mislabeled as changes
since verification. See [snapshot design](design.md).

PASS and CURRENT are green, FAIL red, ERROR/STALE/UNKNOWN amber.
Help banners, commands and flags use cyan; headings are bold and secondary
metadata is muted. Piped streams, `NO_COLOR` (even empty) and `--no-color` disable
ANSI styling; stdout and stderr detect their terminal independently. Raw command
output is saved in local JSON and shown with `--details`, bounded to 64 KiB per
stream with byte totals/truncation indicators.
It is not automatically redacted; configure commands to avoid printing secrets.

## Configure checks and reviews

See the [configuration reference](configuration.md). A Check lives in
`.please/checks/<name>/` with rule.toml. Prefer a command string such as
`[check] command = "python3 scripts/check-docs.py"`; quoting and escaping follow
the configuration reference, without an implicit shell. Legacy command arrays and
entrypoint/interpreter forms are deprecated but remain supported. Filename entrypoints live in that
directory; paths containing `/`, such as `scripts/check-files.py`, resolve from
the consumer repository root (the source repository root for Git imports).
Multiple Rules may share a script and still execute independently. Direct execution uses
the OS; an explicit interpreter receives the absolute entrypoint. The consumer
root stays the working directory; PLEASE_RULE_DIRECTORY locates bundled files.
Exit 0 is PASS, 1 is FAIL and 2 is ERROR. Unexpected numeric codes are explicitly
reported as FAIL; signals, missing scripts and process-launch failures are ERROR.
A completed non-blocking FAIL permits success; execution errors always block. Checks are
trusted code, without process sandboxing. Changing evaluated files prevents PASS.

For an optional AI review, configure an external adapter, customize the disabled
example review, then enable it. Please Behave ships no provider and supplies no
implicit default. Authentication, model settings and tool permissions belong to
the adapter. See [provider configuration](configuration.md#configure-the-ai-provider)
and the [adapter protocol](adapter-protocol.md).

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Initialization/validation/help succeeded, or all required results PASS and status CURRENT |
| 1 | Verification FAIL, or status STALE |
| 2 | Configuration/reconciliation/execution/provider/report ERROR, declined or unavailable confirmation, status UNKNOWN, or invalid CLI arguments |
| 3 | Status requested before any report exists |

Status preserves a CURRENT report's original FAIL/ERROR exit code. UNKNOWN takes
precedence if snapshot coverage is incomplete, even when some differences are known.

## Optional Git hook

Please Behave supports three fixed Git hooks: `pre-commit`, `commit-msg` and `pre-push`.
Each invokes its identically named built-in execution point. Please Behave derives
required hooks automatically from enabled effective Rule assignments, including
Rule Set inheritance and imported Rules. Run `please sync` to reconcile them;
no hook configuration is needed. `check` and custom points are invoked explicitly
and have no Git hook. External import mappings apply only to custom points.

Assign Rules to the built-in points to choose their timing; check scripts implement
validation. Pre-commit runs before the message is finalized. Commit-msg receives
the final message file. Pre-push runs before references are transferred. All failures block Git with nonzero status,
including a missing `please` on PATH. Conventional Commits validation belongs in
a standalone checker, which must not invoke the hook again.

The lifecycle is **Validate -> Inspect -> Plan -> Confirm -> Apply -> Verify**.
`sync` performs validation and reconciliation without checks or provider calls.
Direct `run` execution of `pre-commit`, `commit-msg` or `pre-push` performs
the same prerequisite. `please run --at check` and direct custom points such
as `release` execute without hook inspection, confirmation or modification, even
with missing or outdated hooks. They need no hook setup in a fresh CI checkout
and can run independently from any CI workflow.
Synchronized state succeeds without prompting or writing. Changes are displayed
as creation, managed-section update or removal and require `y`/`yes` in a terminal.
Declining, missing a terminal, conflicts or failures exit 2 and preserve the
previous report. Invalid configuration always fails before hook modification.
`--plan` previews rule execution only and never synchronizes hooks.

For a fresh CI checkout or other unattended setup, run:

```sh
please sync --yes
please run --at pre-push
```

Successful `sync` prints `Synchronization completed successfully.` when import
resolution state, cached repositories/revisions or Git hooks changed, or
`Already synchronized.` otherwise. Both messages use stdout and exit 0.
Refreshing unchanged imports alone does not count as a change. Hook plans use
`Planned changes:` on stderr and retain `Apply these changes? [y/N]`.

The option explicitly approves the displayed integration plan without stdin or a
terminal and prints no auto-approval message. It defaults to off and is available
only on `sync`. Invalid configuration, ownership conflicts and apply/verification
failures still exit 2;
recursion protection remains active. No checks or reviews run during setup, and
the previous report is preserved. Repeated setup leaves synchronized hooks
untouched. Auto-approval manages declared integrations only; it does not create
or repair governed files such as README, LICENSE or `.editorconfig`.

Generated wrappers supply explicit event and wrapper-path arguments validated by
Please Behave against Git's effective directory and the identically named execution
point. Within hooks, reconciliation is read-only: drift fails with instructions to run `please sync`
manually. No prompt or hook write occurs. A repository-scoped execution guard
blocks nested Please Behave execution from checker processes.

Deterministic checks receive `PLEASE_GIT_HOOK`, `PLEASE_GIT_HOOK_ARG_COUNT` and
one-based `PLEASE_GIT_HOOK_ARG_1`, `PLEASE_GIT_HOOK_ARG_2`, etc. Values preserve
Git's arguments; relative paths are relative to the consuming repository root. For a commit-msg
execution point, create `.please/checks/commit-message/rule.toml` with
`[check] command = "sh .please/checks/commit-message/index.sh"`. Its index.sh reads
the supplied hook arguments:

```sh
#!/bin/sh
set -eu
test "$PLEASE_GIT_HOOK" = commit-msg
test "$PLEASE_GIT_HOOK_ARG_COUNT" = 1
exec my-message-checker "$PLEASE_GIT_HOOK_ARG_1"
```

The script reads environment variables itself; Please Behave passes them without expansion. Checks retain
null stdin; pre-push reference data from stdin is not forwarded. Original Git
arguments still reach any unmanaged hook commands.

### Ownership and recovery

New hooks use `#!/bin/sh`, executable permissions, explicit
`# BEGIN PLEASE MANAGED` / `# END PLEASE MANAGED` markers and an ownership line
bound to the common Git repository. Existing executable `#!/bin/sh` hooks receive
a section immediately after the shebang; their remaining bytes and permissions
are preserved. Please Behave runs first and propagates failure before custom commands.
Other shebangs, nonexecutable custom hooks, misplaced/malformed/duplicate markers
or foreign ownership are conflicts. Please Behave reports the file and stops before
confirmation or writes. Inspect and correct the conflict manually, preserving
custom behavior, then rerun `please sync`. Do not copy managed wrappers between
repositories; declare mappings and let each clone generate its own wrappers.

Changing mappings updates managed sections. Removing mappings removes those
sections, preserving unmanaged content; whole-file deletion requires the exact
owned wrapper with no other content. Repeated synchronized runs preserve file
timestamps. A moved clone needs manual review of old ownership markers.

Git resolves the effective directory, including `core.hooksPath` and worktrees;
Please Behave never changes Git configuration. Custom directories must be inside the
working-tree root, with no parent traversal or symlink components. External/shared
custom paths require manual integration or a repository-local `core.hooksPath`.
The default common-repository hooks directory is shared by linked worktrees;
all such worktrees must agree on mappings. Divergent worktree configurations cause
drift and must be reconciled deliberately. Symlinks and nonregular hooks fail
safely. Switching hook directories leaves the previous directory untouched;
remove its managed mappings before switching if you want Please Behave to clean it up.

The complete plan is rechecked before application. Replacements are staged and
synced, then atomically replaced one file at a time. Multiple files are not one
atomic transaction. Failures report the affected operation/path, earlier operations
and rollback result. Rollback preserves concurrent edits and reports any paths it
cannot restore. After interruption, inspect the hooks and rerun `please sync`;
leftover `.please-hook-*` temporary files can be removed after ensuring no other
reconciliation is running. Missing directories are created after approval.

Local hooks are bypassable (`git commit --no-verify`, `git push --no-verify`).
They verify the working tree, not the exact staged index or pushed commit.
Exact-index verification is separate future work. CI can execute the same execution points
against a clean checkout; hosted authentication and deployment remain separate.
Development of Please Behave itself uses a separately installed, published Please Behave
governor and `.please/` configuration. See [repository self-integration](self-governance.md).
