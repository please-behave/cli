# Adapter request

Producer: Please Behave. Consumer: an external adapter.
[request.schema.json](request.schema.json) defines the JSON structure and required
fields. It permits additional request fields so adapters can ignore extensions.
[api.yaml](api.yaml) identifies the shared API version and related requirements.

## Version and identity

New requests carry the API's experimental version in `protocol_version`.
During the version-zero transition adapters accept both supported labels in the
schema and echo the received label in their response. Unsupported versions must
be rejected; Please never retries with another label. Stable compatibility follows
[DEC-0026](../../decisions/DEC-0026.yaml), independently of CLI release numbers.

`execution_point` is the consumer's selected point, including the target of an
import mapping. `reviews` contains exactly one review. Its ID identifies the rule;
criteria are the configured Markdown prompt, and `context_files` names its selected
context documents. `rule_parameters` contains the resolved string values of that
Rule, with file parameters represented by absolute paths.

## Evidence

`input.excerpts` contains selected context text with its original repository path
and line range, including subsections. `end_line` cannot precede `start_line`.
`input.files` contains the changed files in this review's scope: those matching
its triggers, or all changed files without triggers. `content` is included only
when requested and available as current text; binary and deleted files have none.
A binary file has `binary: true`. `change_set`, when requested, describes the same
scope at the selected execution point. These are initial evidence, not a limit
on further repository inspection.

At `commit-msg`, `input.commit_message` contains the whole message with consecutive
one-based line numbers; an empty message has no lines. It is absent at other
points. The prompt determines which parts matter. There is no message-file
argument in the adapter invocation or top-level request.

`context_file` is the absolute native path to this run's read-only
[Git context](../run-context/document.md), with
[delivery and lifetime](../run-context/delivery.md) defined by the Run Context API.
It is a separate document, not the snapshot or the adapter request itself.
`prompt` contains the common review instructions, criteria and prepared evidence.
An adapter may use it directly or construct its own provider input while preserving
the configured criteria, supplied evidence and result obligations. Evidence is
prepared once and passed unchanged from planning to execution.

## Adapter settings and response instructions

`reviews[0].parameters` is the effective adapter settings table. Provider and
review values are JSON-compatible; review values replace matching top-level keys,
including nested tables or arrays as whole values. No deep merge occurs. Please
does not interpret model, reasoning, tools or MCP settings. They remain distinct
from `rule_parameters`. Credentials must not be stored in parameters, which can
appear in version-controlled configuration, plans and reports.

`response_schema` describes the response body required for generation, excluding
`protocol_version` and `usage`. It is derived from the
[response schema](response.schema.json). The adapter adds the protocol envelope
and optional telemetry after interpreting the provider's answer. Structural schema
compliance does not replace byte limits, evidence validation or input stability.
