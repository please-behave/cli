# Please Behave – Product Vision and Architectural Direction

## Context

Please Behave is a source-available, agent-agnostic governance framework for software development,
licensed under [PolyForm Noncommercial License 1.0.0](../LICENSE).

The project originated from the observation that AI-assisted software development is becoming increasingly autonomous, while the responsibility for software quality, architectural consistency, maintainability, and adherence to requirements remains with developers and organizations.

Traditional development tools already provide deterministic quality gates through compilers, tests, linters, and static analysis.

However, many important expectations cannot be expressed reliably through deterministic checks alone.

Examples include:

- Does an implementation satisfy its documented requirements?
- Does a change respect established architectural decisions?
- Has unnecessary complexity been introduced?
- Does the implementation preserve the intended behavior of the product?
- Are important constraints being violated even though all tests pass?

LLMs can evaluate such questions, but their judgments are probabilistic, potentially inconsistent, and dependent on the evidence provided.

Please Behave combines these two approaches within a common, configurable governance model.

## Product Vision

**Please Behave makes software development governance explicit, executable, and verifiable – independently of who or what writes the code.**

Please Behave is not a coding agent.

Please Behave is not an agent orchestration framework.

Please Behave does not implement features, repair code, or decide how development tasks should be performed.

Instead, Please Behave defines and evaluates the conditions under which development results can be considered acceptable.

Its responsibility is to establish whether explicitly configured expectations have been satisfied and to provide actionable evidence when they have not.

## Fundamental Principles

### 1. Governance without orchestration

Please Behave governs outcomes without controlling implementation.

Developers and coding agents remain free to choose their workflows, tools, architectures, and implementation strategies.

Please Behave evaluates whether the resulting software satisfies the configured expectations.

It must not evolve into another coding agent or a general-purpose agent orchestration framework.

### 2. Agent-agnostic by design

Please Behave must remain independent of individual coding agents and LLM providers.

External executable adapters integrate provider CLIs, direct model APIs or local
models without provider-specific logic in the core. Please Behave supplies no
providers by default.

Provider-specific functionality must remain behind explicit integration boundaries.

### 3. Verification as Code

Governance definitions belong alongside the source code.

They should be:

- Declarative where practical.
- Version-controlled.
- Human-readable.
- Independently executable.
- Portable between development environments.
- Reviewable through ordinary software development practices.

The `.please/` directory is the project's governance configuration.

### 4. Deterministic and probabilistic verification

Please Behave distinguishes between two fundamental verification mechanisms.

**Checks** execute commands and interpret their results deterministically.

**Reviews** use LLMs to evaluate expectations that require interpretation.

Both contribute to a unified verification outcome.

However, their evidential characteristics differ.

A successful LLM review is not mathematical proof that a requirement has been satisfied.

Please Behave must preserve this distinction rather than presenting all verification results as equally reliable.

### 5. Explicit enforcement

Please Behave should support quality gates that prevent progression when required conditions have not been satisfied.

Every selected rule executes by default. Explicit invocation-level fail-fast may
stop scheduling after a blocking failure or fatal execution error; already
running checks finish by default, with optional graceful termination and explicit
cancellation results. Rules explicitly declare whether completed
violations block success; the default is blocking. An inability to complete a blocking review or any deterministic check prevents a
successful overall result; advisory review errors remain visible.

Please Behave must distinguish actual violations from technical verification failures.

Enforcement mechanisms may include local commands, Git hooks, CI pipelines, and future integrations with development platforms.

The strength of enforcement depends on the integration. A locally bypassable hook must not be presented as an unconditional guarantee.

### 6. Actionable and auditable results

Verification must produce structured, machine-readable results.

Every result should identify:

- What was evaluated.
- Whether verification succeeded.
- What evidence supports the result.
- Which violations were discovered.
- What prevented verification, if applicable.
- Which source state was evaluated.

Results should be understandable to humans and consumable by other tools or coding agents.

### 7. Token efficiency as an architectural concern

LLM verification consumes computational resources and potentially incurs financial costs.

Please Behave should minimize unnecessary model invocations without compromising verification quality.

Potential strategies include:

- Running deterministic checks before LLM reviews.
- Selecting only relevant reviews.
- Sharing common context between reviews.
- Avoiding unnecessary repository-wide context.
- Reusing valid verification results when appropriate.
- Measuring token consumption.
- Supporting configurable model capabilities.

Optimization must not silently reduce the coverage or reliability of mandatory verification.

### 8. Minimal complexity

Please Behave should remain a small, self-contained native executable.

Rust is the initial implementation language.

Avoid unnecessary runtime dependencies, databases, services, distributed infrastructure, and complex abstractions.

Do not implement speculative extensibility without a concrete use case.

A simple, understandable architecture is preferable to a theoretically universal framework.

## Current Product Scope

The initial implementation focuses exclusively on software development verification.

Its primary capabilities are:

- Repository-local configuration with reusable external Git rule sets.
- Deterministic checks.
- LLM-based reviews.
- A uniform, configuration-driven executable adapter interface.
- Configuration validation and confirmed reconciliation of declared external state.
- Rule selection through execution points and rule sets.
- Structured verification reports.
- Git working-tree change detection.
- CLI integration with Git hooks and CI pipelines.

The executable is `please`, with English commands and user-facing output:
`init`, `sync`, `run`, `status` and `help`.

## Long-Term Direction

Please Behave may eventually evolve beyond individual repository verification.

Potential future capabilities include:

- Organization-wide governance policies.
- Additional LLM providers.
- Lifecycle-specific verification.
- Integration with coding-agent hooks.
- Policy inheritance and controlled exceptions.
- Governance based on architectural decisions.
- Verification of different source states, including commits and pull requests.
- Historical verification evidence and audit trails.
- Additional software lifecycle phases.

These are possibilities, not commitments.

The architecture should avoid unnecessary barriers to such extensions without implementing them prematurely.

## Architectural Boundaries

Please Behave must not become:

- A coding agent.
- An autonomous software developer.
- A replacement for Git.
- A CI/CD platform.
- A general-purpose workflow engine.
- A mandatory development methodology.
- A proprietary wrapper around one LLM provider.
- A compliance platform that imposes requirements unrelated to the project.

Please Behave should allow projects to define their own expectations rather than imposing universal development rules.

## Competitive Positioning

Please Behave operates in an ecosystem that includes agent governance frameworks, AI code review tools, policy engines, and agent orchestration platforms.

Its intended differentiation is the combination of:

1. Independence from the coding agent.
2. Unified deterministic and LLM-based verification.
3. Repository-local, version-controlled governance.
4. Explicit and portable enforcement.
5. Structured, actionable verification evidence.
6. Token-conscious execution.
7. Minimal operational complexity.

The goal is not to replace existing coding agents or duplicate their capabilities.

Please Behave should complement them.

## Guiding Question

Every proposed feature or architectural change should be evaluated against this question:

**Does this make development expectations easier to define, verify, enforce, or understand without unnecessarily taking control of the development process?**

If the answer is no, the feature probably does not belong in Please Behave.
