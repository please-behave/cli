# Plan preview and diagnostics

Producer: Please Behave. Consumers: people and tools inspecting an execution plan,
and report consumers reading the embedded prepared plan. Both contracts share
the compatibility version in [api.yaml](api.yaml).
[plan.schema.json](plan.schema.json) defines the JSON structure and permitted
values. [Prepared plan and identity](prepared.md) defines its semantic obligations.
The schema's `prepared` definition also describes the plan embedded in a report;
the report has its own version and envelope.

## Invocation and effects

`please run --plan` previews the named `--at` execution point, defaulting to
`pre-push`. Unknown points are rejected. Adding `--json` writes exactly one JSON
object to stdout. The CLI version announcement goes to stderr in JSON mode;
otherwise it precedes the human-readable plan on stdout. Human labels and
explanations are English; configured text and technical identifiers are preserved.
The human-readable rendering is not a machine-readable format.

Preview uses the same selection and prepared review input as execution at the
same point and mode. It runs no checks or adapters, checks no adapter availability,
uses no network, reconciles no hooks, prompts for no confirmation, changes no
project or Git state and does not replace the saved report. It writes no run
context. Previously synchronized imports are used offline; preview does not
refresh them. A preview describes the captured inputs, not a guarantee that a
later execution will see the same state.

## Output forms and failures

Every JSON preview supplies `plan_schema_version`, `status` and `blocking_reasons`.
A planned preview exits 0. A blocked preview exits 2 and contains nonblank
blocking reasons. Planned status requires no blocking reasons and complete input.
It says preparation permits execution, not that any rule has passed.

When configuration loading or selection fails before a prepared plan is available,
the diagnostic form contains only the common fields. It omits unavailable input,
selection and identity rather than inventing them. When preparation returns a
plan with blocking reasons, the complete prepared form is retained, including
known metadata and input limitations. An incomplete snapshot, unavailable review
comparison or failed review-input preparation prevents execution.

Consumers must inspect status and reasons before using plan details. Neither a
valid JSON structure nor an empty selection establishes a successful verification.
Error messages are explanatory strings, not stable error codes.

## Compatibility

New output uses the experimental API version in `plan_schema_version`. Readers
accept both transition labels permitted by the schema with identical structural
and semantic validation. Unsupported versions and non-integer JSON version
representations are rejected without retry or fallback. Unknown structural fields
are rejected; arbitrary keys inside adapter profile `parameters` are data. Optional
metadata can be absent in older documents: absent `disabled_rules` and
`depends_on` mean empty lists; an absent or null default `adapter` means none.

[DEC-0026](../../decisions/DEC-0026.yaml) governs compatibility independently of
product releases. The report's version does not replace the embedded plan version.
