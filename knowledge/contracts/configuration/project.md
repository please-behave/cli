# Project configuration

Producer: the consuming project's author. Consumer: Please Behave.
`.please/config.toml` is the project entry point.
[project.schema.json](project.schema.json) defines its supported fields, types,
required markers, defaults and representable bounds as a JSON-compatible parsed
TOML tree. [Discovery](discovery.md) defines locations and document validation.
Schema defaults describe behavior; validation does not rewrite configuration.

## Compatibility and representation

All governance configuration shares the compatibility version in [api.yaml](api.yaml),
including project settings, Rules, Rule Sets, provider declarations and imports.
Separate TOML documents do not have separate API versions. Version 0 is experimental
under [DEC-0026](../../decisions/DEC-0026.yaml), independently of CLI releases.
The API version is contract metadata: configuration declares no version field.
Adding `version` or `schema_version` is an unknown-field error, not a request for
another parser. No version negotiation or fallback interpretation occurs.

The schema describes TOML's strings, booleans, integers, arrays and tables using
JSON types. TOML parsing and semantic validation remain necessary: null is not a
TOML configuration value, integers must fit TOML and the target representation,
and floating-point or date/time values do not substitute for the declared types.
Object-field order has no meaning. Unknown fields are rejected, including inside
optional tables. An omitted table applies the defaults of its child fields;
an empty table behaves the same way. A missing field is not an explicit null.

## Project label and presentation

`project` is the configured display label, preserved in plans and reports. It is
not a Rule identity, import namespace or provider reference. `max_display_files`
bounds the changed paths shown by status. This presentation setting does not
weaken snapshot coverage or turn incomplete comparisons into exact totals; the
[Snapshot and Freshness contract](../report/snapshot.md) defines those guarantees.

## Execution settings

`execution.max_parallel` limits checks running simultaneously in one invocation.
The default permits concurrency; a value of one runs serially. Invocation-level
`--max-parallel` overrides the setting without changing configuration. Resource and
dependency waits use no execution slot; reviews run only after the check phase.
The scheduling limit, local duration history and fail-fast execution strategy do
not change semantic [plan identity](../plan/prepared.md).

`execution.fail_fast_terminate_running_instances` permits termination of running
checks only when invocation-level `--fail-fast` triggers. It does not enable
fail-fast on its own. Fail-fast has no persistent configuration field. Without
termination, already running checks finish; with it, Unix process groups receive
graceful termination followed by forced termination after
`execution.fail_fast_termination_grace_seconds`. The grace value is validated even
when termination is disabled. Platforms without process-group termination reject
enabled termination rather than silently ignoring it. Interrupted checks retain
CANCELLED results and partial output under the [Report API](../report/document.md).

Checks that share mutable state use the same `check.resource` declaration; resource
names coordinate checks within an invocation, including imports, rather than
locking other Please invocations. Rule-level declarations follow the
[check reference](../../../docs/configuration.md#add-a-check).

## Run context settings

`context.retain_runs` controls retention of run-context directories.
`context.max_section_bytes` bounds each context list, patch and message; it is not
a limit on the complete JSON document or the project's snapshot. Larger sections
remain visibly incomplete rather than silently trusted. The
[Run Context API](../run-context/api.yaml) defines delivery, lifetime and truncation.

## Execution points and explicit imports

`execution_points.custom` declares custom point names in the consuming project.
The built-in names are reserved. `pre-commit`, `commit-msg` and `pre-push` correspond
to their fixed Git hooks. `check` and custom points run only by explicit request,
without requiring current hooks or any CI workflow. Please generates, discovers
and validates no CI integration. Custom names such as `release` carry no special
behavior. A run without `--at` selects `pre-push`.

`imports` declares explicit external Rule Set sources and their integration
settings only at project level. `provider_imports` independently selects named
provider packages; these imports select no Rules or execution points. Both arrays
are empty by default; Please adds no implicit sources or providers. Their fields
are represented in the project schema, with source selection, parameter binding,
authentication, integrity and refresh semantics defined by the
[Rule Set import reference](../../../docs/configuration.md#import-rule-sets-from-git)
and [provider import reference](../../../docs/configuration.md#import-providers-from-git).
