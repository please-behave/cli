# Knowledge Model

This directory contains the authoritative knowledge about Please Behave.

The knowledge model records **agreed-upon product knowledge**, including retained historical knowledge. It describes the product specification, not implementation progress or validation results. It is neither a backlog nor a roadmap.

## Artifacts

| Artifact | Purpose |
|---|---|
| **Capability** | Describes what the product is intended to be capable of. |
| **Use Case** | Describes an actor's goal and interaction with the product. |
| **Requirement** | Defines a binding, verifiable condition the product must satisfy. |
| **Decision** | Records an architectural choice and its rationale. |

Requirements form the **verifiable contract between specification and implementation**. Implementation and test results are evaluated against this contract; they do not determine its validity.

## Principles

- Only agreed-upon knowledge is recorded. No drafts, proposals, roadmaps, or implementation statuses.
- Artifacts complement rather than duplicate one another.
- Requirements describe observable behavior or properties, not unnecessary implementation details.
- Relationships are recorded in one direction; reverse relationships are derived.
- Capabilities and requirements are authoritative unless marked `obsolete: true`.
- Decisions remain authoritative unless superseded by a newer decision referencing them through `supersedes`.
- Historical artifacts may be retained for traceability. Artifacts withdrawn without replacement may be deleted; Git preserves their history.
- Schemas define the permitted structure of each artifact.
- 
