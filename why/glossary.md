# Glossary

The terms Please Behave uses with a specific meaning. Each entry explains the
concept and how it differs from its neighbors; the
[configuration reference](https://github.com/please-behave/docs/blob/main/src/configuration.md) holds fields, defaults and
behavior.

| Term | In short |
|---|---|
| [Project](#project) | The repository in which Please Behave runs and whose governance it evaluates. |
| [Rule](#rule) | A named expectation, implemented as a check or a review. |
| [Rule ID](#rule-id) | The identifier of a rule within its source. |
| [Rule Identity](#rule-identity) | The global identity of a rule across runs and revisions. |
| [Run-Scoped ID](#run-scoped-id) | The identifier used for a rule in plans and reports. |
| [Rule Version](#rule-version) | An optional A.B label assigned by the rule owner. |
| [Check](#check) | A deterministic rule that runs a command. |
| [Review](#review) | A rule that asks an AI to judge the project. |
| [Rule Set](#rule-set) | A named group of rules with shared execution points. |
| [Execution Point](#execution-point) | A moment at which rules run. |
| [Plan Preview](#plan-preview) | A preview of what would run at an execution point. |
| [Configuration Error](#configuration-error) | An invalid configuration or a missing prerequisite required for execution. |
| [Run Context](#run-context) | The Git state of a run, supplied to rules as a file. |
| [Input Snapshot](#input-snapshot) | A fingerprint of the files a run judged. |
| [Release Fact](#release-fact) | An authenticated report of a release version and its Git commit. |
| [Deployment Target](#deployment-target) | A named place where software runs, identified by its authenticated owner and name. |
| [Adapter Profile](#adapter-profile) | A named configuration entry that tells Please Behave which adapter to run and with what settings. |
| [Adapter](#adapter) | The executable that talks to the AI for an adapter profile. |
| [External Source](#external-source) | A Git repository that supplies rule sets or adapter profiles. |

## Project

The project is the Git repository in which Please Behave runs and whose
governance it evaluates. Its repository root is the base for project-relative
paths. It can import rules and adapter profiles from [external sources](#external-source),
whose repositories remain distinct from the project repository.

## Rule

A rule is one named expectation about a project, such as "the changelog is
current" or "the implementation respects the architecture". A rule is either a
[check](#check) or a [review](#review), never both. Rules are grouped in
[rule sets](#rule-set) and run at [execution points](#execution-point).

## Rule ID

A rule ID identifies a [rule](#rule) within its source. For an imported rule,
the ID alone does not distinguish which import supplies it.

## Rule Identity

A rule identity consists of the normalized Git repository source, the
repository-relative path of the rule directory and the [rule ID](#rule-id).
It follows the same [rule](#rule) across refs, commits, import bindings,
parameters, rule sets and execution points (why:DEC-0010).

## Run-Scoped ID

The run-scoped ID is the [rule ID](#rule-id) for a local rule and the
binding-qualified rule ID for an imported rule. That qualification
distinguishes bindings in references, plans and reports (DEC-0014).

## Rule Version

A rule version is an optional A.B label that the [rule](#rule) owner declares in
the [rule definition](contracts/configuration/rule.md#rule-version). A signals a
fundamental change and B any other change. The label communicates the owner's
judgment about the change. It carries no guarantee of reproducible behavior.

## Check

A check is a rule decided by a command. It passes or fails by the command's
exit status, so the same input always gives the same verdict. Checks are the
deterministic half of governance: they run before reviews, and a failing check
always blocks.

## Review

A review is a rule decided by an AI. It carries a prompt, is evaluated through
an [adapter profile](#adapter-profile) and, unlike a check, may be nonblocking. Its judgment
is probabilistic, so it runs only after the deterministic checks. An adapter profile
failure or an invalid answer is an error, never a pass.

## Rule Set

A rule set groups rules and assigns them execution points, so a team can adopt
a coherent set of expectations at once. One rule can belong to several sets.
Rule sets can be local or come from an [external source](#external-source).

## Execution Point

An execution point is a moment at which rules run, for example before a commit
or before a push. Each rule is selected for the points it is assigned to.

## Plan Preview

A plan preview shows what would run at an [execution point](#execution-point)
without running it. It is requested with `please run --at <point> --plan` and
works offline. Offline availability is a property of the preview, not a
separate kind of plan.

## Configuration Error

A configuration error means that the project's own configuration is wrong: a
structural error found when the configuration is loaded, or, for a selected,
enabled review, a parameter or prompt that cannot be resolved. It prevents
success even for an advisory review. What the environment must provide, such as
an executable or a credential, is no configuration error. FAIL and ERROR
outcomes of an advisory review remain nonblocking; they describe the review's
result rather than a configuration error.

## Run Context

The run context describes the Git state a run judges: the branch, the staged
and unstaged changes, the commit message or the pushed commits. Please Behave
writes it as one read-only JSON file per run and hands its path to every rule,
so rules do not need to query Git themselves.

## Input Snapshot

The input snapshot is a fingerprint of the files a run judges. It is captured
around execution, so a result is only trusted when the files did not change
while the rules ran. An incomplete or unstable snapshot never counts as a pass.

## Release Fact

A release fact associates an opaque version with a full Git object ID in a
repository. The ledger uses the repository and commit to find reported
verification evidence. The release fact carries the sender's claim about the
release and contributes no verification evidence of its own.

## Deployment Target

A deployment target belongs to the repository that deploys the software. The
owner declares the target in its [project configuration](contracts/configuration/project.md#deployment-targets)
and registers it from CI with [`please register`](contracts/report/registration.md#deployment-targets).
The authenticated owner and case-sensitive name identify a target (why:DEC-0022).

## Adapter Profile

An adapter profile is a named configuration entry: the command of an
[adapter](#adapter), a timeout and opaque parameters. It is not an AI vendor.
The adapter alone decides which service, model or endpoints it uses and may call
several. Please Behave ships no adapter profile and chooses none implicitly. A
project can explicitly configure a default adapter profile and named adapter
profiles, which a review selects by name.

## Adapter

An adapter is a trusted executable that translates between Please Behave and an
AI service. It receives one JSON request, returns a JSON response and signals
the verdict through its exit status, whether it calls a CLI, an HTTP API or a
local model. This keeps Please Behave independent of any particular AI vendor. See the
[adapter protocol](contracts/README.md).

## External Source

An external source is a Git repository, pinned to a revision, from which a
project imports [rule sets](#rule-set) or [adapter profiles](#adapter-profile) instead of
defining them locally. This lets several projects share the same governance.
Imports are fetched on `please sync` and verified before use. See
[external rule sets](https://github.com/please-behave/docs/blob/main/src/external-rule-sets.md).
