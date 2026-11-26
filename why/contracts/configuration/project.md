# Project configuration

Producer: the consuming project's author. Consumer: Please Behave.
`.please/config.toml` is the project entry point.
[project.schema.json](project.schema.json) defines its supported fields, types,
required markers, defaults and representable bounds as a JSON-compatible parsed
TOML tree. [Discovery](discovery.md) defines locations and document validation.
Schema defaults describe behavior; validation does not rewrite configuration.

## Compatibility and representation

All governance configuration shares the compatibility version in [api.yaml](api.yaml),
including project settings, Rules, Rule Sets, adapter profile declarations and imports.
Separate TOML documents do not have separate API versions. Version 0 is experimental
under [DEC-0026](../../decisions/DEC-0026.yaml), independently of CLI releases.
The API version is contract metadata. The project document rejects `version`
and `schema_version` as unknown fields. The optional [rule version](rule.md#rule-version)
describes the rule itself. Configuration parsing uses a single interpretation
without version negotiation.

The schema describes TOML's strings, booleans, integers, arrays and tables using
JSON types. TOML parsing and semantic validation remain necessary: null is not a
TOML configuration value, integers must fit TOML and the target representation,
and floating-point or date/time values do not substitute for the declared types.
Object-field order has no meaning. Unknown fields are rejected, including inside
optional tables. For tables with defaulted child fields, omission and an empty table behave
identically. An optional table with required fields must supply them when present. A missing field is not an explicit null.

## Project label and presentation

`project` is the configured display label, preserved in plans and reports. It is
not a Rule identity, import binding or adapter profile reference. `max_display_files`
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

Invocation-level `--fail-fast` with an effective limit of at least two gives
exactly one check slot shortest-estimated-duration priority. When free, that
slot dispatches first and chooses the eligible check with the smallest own
estimate, breaking ties by canonical ID; other slots retain priority by the longest
remaining dependency path. It accepts any duration and reserves no idle capacity.
Eligibility, resource exclusion and the concurrency limit remain authoritative.
Uniform fallback estimates apply without history. Complete runs, serial runs
and reviews retain their usual order. This behavior has no configuration field.

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
[check reference](https://github.com/please-behave/docs/blob/main/src/configuration.md#add-a-check).

## Run context settings

`context.retain_runs` is the number of newest earlier run-context directories to
keep at the start of a run, regardless of whether those runs finished. The current
run is preserved in addition to that number. Its default and permitted values are
defined in [project.schema.json](project.schema.json).
`context.max_section_bytes` bounds each context list, patch and message; it is not
a limit on the complete JSON document or the project's snapshot. Larger sections
remain visibly incomplete rather than silently trusted. The
[Run Context API](../run-context/api.yaml) defines delivery, lifetime and truncation.

## Report settings

`report.retain_runs` is the number of newest saved reports to keep after each
run, counting the report just saved. It is independent of `context.retain_runs`.
Its default and permitted values are defined in [project.schema.json](project.schema.json).
The [Report API](../report/document.md) defines numbering and removal.

## Publish settings

The optional `[publish]` table supplies settings for
[`please publish`](../report/document.md#publishing-evidence),
[`please register`](../report/registration.md),
[`please record release`](../release-facts/record.md) and
[`please record deployment`](../release-facts/deployment.md). Each command takes the required
`--gateway-url` argument on the command line. A `gateway_url` entry in this
table is rejected as an unknown field.

`auth` selects `api-key` (the default) or `github-oidc`. Other values are
configuration errors. With `api-key`, `key_env` is required and names the
environment variable that supplies the API key. With `github-oidc`, no
`key_env` is required. Credentials are acquired only for delivery.

An optional `repository` overrides derivation of the repository identity from
Git's `origin` remote. With API-key authentication the override is used as
written. With GitHub OIDC the CLI lowercases the host and path of the selected
identity, and the gateway rejects delivery if the result differs from the
workflow's repository. Changing these fields changes a governed file and
requires a new run before publication.

Deployment delivery takes the subject repository from its required
`--repository` argument. The authenticated sender identifies the target owner.

These fields extend the experimental configuration API at version 0. They add
no version declaration in TOML.

## Deployment targets

Each top-level `[[targets]]` entry declares a deployment target owned by the
repository. The list defaults to empty and allows up to 100 entries. Replace
the initial `targets = []` line when adding the first entry. A name
contains 1 to 64 ASCII characters. It starts with a letter or digit and may
continue with letters, digits, dots, underscores or hyphens. Names are case
sensitive. An optional description contains 1 to 512 Unicode characters and
excludes NUL. The CLI preserves both values as supplied.

CI submits these declarations through [`please register`](../report/registration.md#deployment-targets).

## Execution points and explicit imports

`execution_points.custom` declares custom point names in the consuming project.
The built-in names are reserved. `pre-commit`, `commit-msg` and `pre-push` correspond
to their fixed Git hooks. `check` and custom points run only by explicit request,
without requiring current hooks or any CI workflow. Please generates, discovers
and validates no CI integration. Custom names such as `release` carry no special
behavior. A run without `--at` selects `pre-push`.

`imports` declares explicit external Rule Set sources and their integration
settings only at project level. `adapter_imports` independently selects named
adapter profile packages; these imports select no Rules or execution points. Both arrays
are empty by default; Please adds no implicit sources or adapter profiles. Their fields
are represented in the project schema, with source selection, parameter binding,
authentication, integrity and refresh semantics defined by the
[Rule Set import reference](https://github.com/please-behave/docs/blob/main/src/external-rule-sets.md#import-rule-sets)
and [adapter profile import reference](https://github.com/please-behave/docs/blob/main/src/external-rule-sets.md#independent-profile-packages).
