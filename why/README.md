# Why

This directory holds the Why of the CLI: the authoritative requirements, decisions, components and contracts that say what the product is and why. It follows the [Why documentation approach](https://github.com/please-behave/why).

The Why records **agreed-upon product statements**, including retained history. It describes the product specification, not implementation progress or validation results. It is neither a backlog nor a roadmap; direction belongs in the vision.

## Artifacts

| Artifact | Purpose |
|---|---|
| **Use Case** | Describes an actor's goal and interaction with the product. |
| **[Requirement](requirements/README.md)** | Defines a binding, verifiable condition the product must satisfy. |
| **[API Contract](contracts/README.md)** | Defines an interface, its compatibility boundary and exact obligations. |
| **Decision** | Records an architectural choice and its rationale. |
| **[Component](components/README.md)** | Assigns every compiled file to one responsibility with explicit boundaries. |

Components are the only artifact that describes the current implementation structure rather than the specification. A deterministic rule keeps them complete and true to the code.

Requirements form the **verifiable contract between specification and implementation**. Implementation and test results are evaluated against this contract; they do not determine its validity.

The [glossary](glossary.md) explains the core terms these artifacts use.

## Detailed References

Requirements define the binding product promises. [API contracts](contracts/README.md)
define exact interfaces and reference the requirements they specify. Structural
schemas and semantic contracts complement one another. User guides explain how
to use these interfaces without redefining them.

The [configuration contracts](contracts/configuration/api.yaml) define discovery,
project settings, shared Rule definitions and parameters. The configuration and
external rule set references also contain detailed interface definitions. Their
definitions must remain consistent with the Why.

Requirements and detailed references must agree. Resolve any contradiction
explicitly before implementation and update the affected sources together.

## Principles

- Only agreed-upon content is recorded. No drafts, proposals, roadmaps, or implementation statuses.
- Artifacts complement rather than duplicate one another.
- Requirements describe observable behavior or properties, not unnecessary implementation details.
- Relationships are recorded in one direction; reverse relationships are derived.
- References point from the more volatile artifact to the more stable one: component to requirement, contract to requirement, requirement to decision. A decision names only the decisions it supersedes.
- Requirements are authoritative unless marked `obsolete: true`.
- Decisions remain authoritative unless superseded by a newer decision referencing them through `supersedes`.
- Historical artifacts may be retained for traceability. Artifacts withdrawn without replacement may be deleted; Git preserves their history.
- The [Why standard](https://github.com/please-behave/why/tree/main/standard) defines the permitted structure of each artifact. [`repositories.yaml`](repositories.yaml) declares the repositories these documents link.
