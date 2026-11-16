# Please Behave specification

Status: current implemented contract. Changes to behavior start here and in the
acceptance criteria, before implementation. Explicit user decisions take precedence.

The [product vision](docs/product-vision.md) defines direction and architectural
boundaries: Please Behave makes development expectations explicit, executable and
verifiable, independently of who writes the code. This document defines supported
behavior. The [roadmap](docs/todo/README.md) contains planned extensions; they are
not implemented merely because they have a proposal.

The [configuration reference](docs/configuration.md) defines every public
configuration field, default and limit. The [execution-point contract](docs/execution-points-and-plans.md)
extends this specification for plan schema 1 and report schema 1. These documents must agree; a contradiction must be resolved before work
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

The [requirements](knowledge/requirements/README.md) REQ-0134 to REQ-0150, REQ-0181 and REQ-0182 specify rules, rule sets, display names, execution points and their assignment. They replace the text of
this section. The [components](knowledge/components/README.md) name the
requirements they implement.

## EXT-001: External Git Rule Sets

The [requirements](knowledge/requirements/README.md) REQ-0185 to REQ-0207
specify external rule set imports, their bindings, execution point mapping, rule
references, provenance, synchronization and offline execution. They replace the
text of this section. The [components](knowledge/components/README.md) name the
requirements they implement.

## EXT-002: Independent external providers

The [requirements](knowledge/requirements/README.md) REQ-0208 to REQ-0213
specify independent provider imports, their identifiers, reference tokens and
provenance. They replace the text of this section. The
[components](knowledge/components/README.md) name the requirements they
implement.

## CFG-001: Configuration and initialization

Projects use `.please/` exclusively.

One unversioned configuration model uses lowercase .please/config.toml, optional
provider.toml, providers/*.toml, checks/*/rule.toml, reviews/*/rule.toml and rule-sets/*.toml.
There is no configuration version field. Unknown fields, duplicate keys/IDs,
invalid references and unsafe rule resources fail, including disabled Rules.
Missing referenced check files or review prompts are execution errors only when
selected and enabled.
No recursive TOML discovery is supported. External sets explicitly select Rule
directories as defined in EXT-001; local definitions use only the paths above.
Supporting files inside rule directories are data, never governance documents.

Project settings are project, max_display_files (default 5), execution_points,
execution, imports and provider_imports. The configuration reference defines all fields and bounds. Review prompt filenames
must be single nonempty names directly in a Rule directory: no separators,
absolute paths, parent traversal, symlinks or special files. Review prompts are
UTF-8 Markdown without frontmatter requirements. Check commands follow CHK-001.
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
## HDR-001: Commit message input

The [requirements](knowledge/requirements/README.md) REQ-0099 and REQ-0130 to REQ-0132 specify what a review receives and how its verdict is reported; reviews at commit-msg receive the whole commit message like any other declared input.
They replace the text of this section. The [components](knowledge/components/README.md) name the
requirements they implement.


## PROV-001: Named adapter configurations

The [requirements](knowledge/requirements/README.md) REQ-0214 to REQ-0221
specify named and default provider configurations, their definitions and the
adapter command. They replace the text of this section. The
[components](knowledge/components/README.md) name the requirements they
implement.

## PROV-002: Adapter protocol

The [requirements](knowledge/requirements/README.md) REQ-0222 to REQ-0228
specify the adapter protocol, its request, parameters and timeout. They replace
the text of this section. The [components](knowledge/components/README.md) name
the requirements they implement.

## SEL-001: Selection and offline plans

The [requirements](knowledge/requirements/README.md) REQ-0151 to REQ-0162 and REQ-0183 specify selection of an execution point, offline plans, checks-only runs and the environment of adapters. They replace the text of
this section. The [components](knowledge/components/README.md) name the
requirements they implement.

## CHK-001: Deterministic checks

The [requirements](knowledge/requirements/README.md) REQ-0001 to REQ-0088 specify
deterministic checks: commands and references, results, parallel execution,
scheduling, dependencies, review gating and fail-fast. They replace the text of
this section. The [components](knowledge/components/README.md) name the
requirements they implement.

## PARAM-001: Rule parameters

The [requirements](knowledge/requirements/README.md) REQ-0163 to REQ-0167 and REQ-0169 specify rule parameters. They replace the text of
this section. The [components](knowledge/components/README.md) name the
requirements they implement.

## REV-001: Reviews and provider isolation

The [requirements](knowledge/requirements/README.md) REQ-0092 to REQ-0104 and REQ-0124 to REQ-0129 specify reviews, adapter isolation, review input and review enforcement. They replace the text of
this section. The [components](knowledge/components/README.md) name the
requirements they implement.


## TRIG-001: File-change review selection

The [requirements](knowledge/requirements/README.md) REQ-0170 to REQ-0179 specify file-change triggers and the change base of reviews. They replace the text of
this section. The [components](knowledge/components/README.md) name the
requirements they implement.

## PREP-001: Shared review input

The [requirements](knowledge/requirements/README.md) REQ-0180 specify the review input prepared once. They replace the text of
this section. The [components](knowledge/components/README.md) name the
requirements they implement.

## OUT-001: Structured review outcomes

The [requirements](knowledge/requirements/README.md) REQ-0116 to REQ-0123 and REQ-0130 to REQ-0132 specify structured review outcomes and token usage. They replace the text of
this section. The [components](knowledge/components/README.md) name the
requirements they implement.


## INP-001: Snapshots and stability

The [requirements](knowledge/requirements/README.md) REQ-0231 to REQ-0240 specify
snapshots, stability checks and freshness. They replace the text of
this section. The [components](knowledge/components/README.md) name the
requirements they implement.

## CTX-001: Structured run context

The [requirements](knowledge/requirements/README.md) REQ-0241 to REQ-0250 specify
the structured run context. They replace the text of
this section. The [components](knowledge/components/README.md) name the
requirements they implement.

## RPT-001: Reporting and CLI outcomes

The [requirements](knowledge/requirements/README.md) REQ-0253 to REQ-0275 specify
reports, progress, terminal output and exit status. They replace the text of
this section. The [components](knowledge/components/README.md) name the
requirements they implement.

## REL-001: Distribution and integrations

Please Behave is distributed as native executables for macOS Apple Silicon, macOS
Intel and Linux x64 and ARM64 (GNU/glibc); Windows is excluded. It builds with stable
Rust at the declared minimum version. Git and configured external tools remain
runtime dependencies; no Python or Node runtime is bundled.

Archives are named `please-vVERSION-TARGET.tar.gz` and contain a matching directory
with `please`, README and LICENSE. Each release also publishes `SHA256SUMS`.
Artifacts must match the tested version and source.

The public GitHub Action is maintained independently in
[please-behave/action](https://github.com/please-behave/action), which owns its inputs,
supported runners and tests. The Action revision and the CLI release are independent.

Releases are manually dispatched on main and Semantic Release publishes only after
artifact and source verification. During 0.x, breaking changes and features increment
the minor version and fixes the patch version; 1.0.0 requires explicit policy
graduation. Normal CI is read-only and actions are pinned. See the
[release policy](docs/releasing.md).

The repository is governed by an installed, previously published release, never by
candidate builds of its own checkout; see [repository self-integration](docs/self-governance.md).
Hosted provider authentication is not provisioned and personal tokens are not copied
to CI. No hosted service, database, autonomous development, universal policy engine,
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
reconciliation in that repository.
Hook arguments are not exported. `commit-msg` supplies the message file through CTX-001; no commit
policy is hardcoded.
Pre-push stdin is read by Please Behave and not forwarded to checks; its reference list is
available through CTX-001. All execution points still verify working files,
not the exact staged index or pushed commit.

## ARC-001: Shared configuration contract

Defaults, resource bounds and the configuration directory have one source in
Rust. Initialization serializes the actual configuration types rather than
maintaining independent TOML templates. The configuration reference's defaults,
limits and examples are checked against those values. Validation, planning and execution
reuse the same bounds; this does not add configurable fields or change defaults.

## DOC-001: Documentation contract

Every editable configuration field, required/optional marker and default must
match the configuration reference. Its examples must validate and plan
offline. Local links/anchors and specification acceptance references must resolve.
Repository Git hooks run link/specification checks at pre-commit and pre-push;
pre-push also runs the Rust documentation acceptance tests.
CI runs them through check. `please run --at pre-push` remains available to developers; agents rely on Git hooks. Automated checks
complement editorial review; they do not prove prose understandable.

## Acceptance coverage and change process

Each essential requirement has a stable ID and representative acceptance tests.
The table is a navigation aid, not a claim that one test proves every clause.
Add or update the contract and acceptance cases before implementing behavior;
update the documentation and relevant design decisions in the same coherent
change. Keep one coherent configuration model and run applicable checks. Follow the user's Git instructions.
Public documentation explains supported behavior; proposals remain in the roadmap.

| Requirement | Representative acceptance test |
| --- | --- |
| TRIG-001 | `tests/review_triggers.rs::preview_and_execution_agree_and_nonmatches_need_no_provider_or_context` |
| PROV-002 | `tests/cli_providers.rs::stdin_results_and_provider_isolation` |
| HDR-001 | `tests/commit_message.rs::commit_message_hook_and_adapter_contract` |
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
| SEL-001 | `tests/configuration.rs::checks_only_reports_selected_reviews_as_not_run` |
| CHK-001 | `tests/configuration.rs::command_representations_execute_equivalently` |
| REV-001 | `tests/cli_providers.rs::stdin_results_and_provider_isolation` |
| PREP-001 | `tests/configuration.rs::prepared_input_is_shared_without_rereading_resources` |
| OUT-001 | `tests/review_contract.rs::review_contract_enforces_boundaries_and_cited_evidence_locally` |
| INP-001 | `tests/acceptance.rs::changing_files_during_checks_blocks_pass` |
| RPT-001 | `tests/configuration.rs::saved_reports_reject_inconsistent_metadata` |
| CTX-001 | `tests/context.rs::push_updates_describe_create_update_delete_and_unpushed_heads` |
| REL-001 | `.github/scripts/test_release.py::test_only_root_version_changes_and_invalid_versions_leave_files_untouched` |

## Runtime namespace isolation

The [requirements](knowledge/requirements/README.md) REQ-0091, REQ-0162, REQ-0251
and REQ-0252 specify the runtime namespace. They replace the text of
this section. The [components](knowledge/components/README.md) name the
requirements they implement.
