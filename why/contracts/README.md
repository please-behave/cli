# API contracts

An API is a compatibility boundary between a producer and its consumers.
A contract defines one part of that boundary. Contracts in the same API share
one compatibility version; separate documents do not imply separate versions.

| API | Contracts |
| --- | --- |
| [Adapter Protocol](adapter/api.yaml) | [Invocation](adapter/invocation.md), [request](adapter/request.md), [response](adapter/response.md) |
| [Run Context](run-context/api.yaml) | [Delivery and lifetime](run-context/delivery.md), [document and patches](run-context/document.md) |
| [Execution Plan](plan/api.yaml) | [Preview and diagnostics](plan/preview.md), [prepared plan and identity](plan/prepared.md) |
| [Verification Report](report/api.yaml) | [Document and lifecycle](report/document.md), [snapshot and freshness](report/snapshot.md), [rule version registration](report/registration.md) |
| [Governance Configuration](configuration/api.yaml) | [Discovery and validation](configuration/discovery.md), [project configuration](configuration/project.md), [Rule definition](configuration/rule.md), [Rule parameters](configuration/parameters.md) |
| [Release Fact Delivery](release-facts/api.yaml) | [Release fact command](release-facts/record.md) |

[Rule knowledge](rule-knowledge/README.md) defines the experimental companion
documents for Rules and Rule Sets. The CLI reads rule knowledge to compute rule digests.

## Representation

Each API directory contains `api.yaml`, validated by the [Why standard](https://github.com/please-behave/why/tree/main/standard) through the `why` ruleset.
The index records its stable `API-NNNN` ID, name, compatibility version and
contracts. Each contract records a globally unique, stable `CON-NNNN` ID, name,
document, optional structural schema and the requirements it specifies.
IDs are never reused. Document and schema paths are relative to the index.

The index is the source for API membership and Contract-to-Requirement links.
Graph consumers can derive API and Contract nodes, API membership and requirement
references from it. Reverse relationships are derived, not maintained separately.
Contracts reach architectural decisions through their requirements. The glossary
is explanatory prose and needs no graph representation.

## Authority

Requirements state the product promises; contracts define their exact interfaces.
A structural schema defines fields, types and representable constraints. Its
Markdown document defines meaning, sequencing, errors and conditions that need
validation beyond the schema. Do not repeat field tables or numeric bounds in
both places. A valid JSON document alone does not establish a valid verdict.

Contracts and requirements must agree. Resolve contradictions with the owner;
neither existing code nor an explanatory guide silently changes the contract.
User documentation teaches usage and links to these definitions.

## Compatibility

[DEC-0026](../decisions/DEC-0026.yaml) defines compatibility versioning. The index
records a single version per API. A document split or move does not change it.
Each API specifies where its version appears and how unknown fields and versions
are handled. Recording metadata does not introduce a new wire or configuration field.

Only the CLI reads adapter responses and saved reports (with their embedded plan),
and it rejects unsupported versions and invalid version types. Run-context documents,
adapter requests and plan previews are produced by the CLI for external consumers,
who own their own version handling. Structural schemas accept the number `1.0` as an
integer version, so the CLI readers, not the schemas, reject it.

The `why` check validates index structure, unique IDs, requirement references
and document/schema paths. Contract tests check structural schemas against actual
requests, responses, run-context documents, plan previews, saved reports and parsed
project configuration. Semantic validation, byte limits and evidence checks remain
explicit obligations
even when a structural schema cannot express them.
