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

The [requirements](knowledge/requirements/README.md) REQ-0276 to REQ-0282 specify
the commands, their boundaries and the project root. They replace the text of
this section. The [components](knowledge/components/README.md) name the
requirements they implement.

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

The [requirements](knowledge/requirements/README.md) REQ-0283 to REQ-0290 specify
the configuration files, their validation and initialization. They replace the text of
this section. The [components](knowledge/components/README.md) name the
requirements they implement.

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

The [requirements](knowledge/requirements/README.md) REQ-0315 to REQ-0319 specify
release artifacts and their distribution. They replace the text of
this section. The [components](knowledge/components/README.md) name the
requirements they implement.

## REC-001: Declarative reconciliation and Git hooks

The [requirements](knowledge/requirements/README.md) REQ-0291 to REQ-0314 specify
synchronization, reconciliation and Git hooks. They replace the text of
this section. The [components](knowledge/components/README.md) name the
requirements they implement.

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
