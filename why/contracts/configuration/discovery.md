# Configuration discovery and validation

Producer: the project author or an explicitly imported source author. Consumer:
Please Behave. [api.yaml](api.yaml) identifies the Governance Configuration API;
[project configuration](project.md) defines its local project entry point.
Configuration documents, Rule implementations and supporting resources serve
different purposes even when stored beside one another.

## Local discovery

Configuration lives under the lowercase `.please/` directory at the Git
working-tree root. Commands that operate on a project require that root rather
than silently selecting a parent or accepting a subdirectory. Discovery recognizes
these locations only:

| Location | Meaning |
| --- | --- |
| `.please/config.toml` | Required project document. |
| `.please/adapter.toml` | Optional default adapter profile. |
| `.please/adapters/*.toml` | Named adapter profiles, identified by filename stem. |
| `.please/checks/*/rule.toml` | Check definition in each immediate Rule directory. |
| `.please/reviews/*/rule.toml` | Review definition in each immediate Rule directory. |
| `.please/rule-sets/*.toml` | Local Rule Set definitions. |

Adapter profile, Rule and Rule Set directories are optional. Each entry in `checks/`
or `reviews/` must be a Rule directory containing its own `rule.toml`; each entry
in `rule-sets/` must be a TOML file. In `adapters/`, TOML files are definitions
and directories may hold supporting resources without recursive discovery.
Supporting files inside a Rule directory, such as a tool's own TOML configuration,
are data: Please never interprets them as governance configuration. Arbitrary or
nested files cannot introduce extra definitions.

Explicit Git imports select source directories under the
[external Rule Set reference](https://github.com/please-behave/docs/blob/main/src/external-rule-sets.md). Each selected
directory defines exactly one Rule Set in its `ruleset.toml`, with optional named
adapter profiles in `adapters/*.toml` beside it; a directory without
`ruleset.toml` fails validation. Imports do not extend local recursive
discovery. Named adapter profile packages follow the
[independent adapter profile import reference](https://github.com/please-behave/docs/blob/main/src/external-rule-sets.md#independent-profile-packages).
These definitions belong to this same configuration compatibility boundary.

## Rule Set display names

Local and imported Rule Sets may declare a `name` TOML string. It must contain
1 to 80 Unicode characters, with no control characters or surrounding whitespace,
using the same bounds as a [Rule's display label](rule.md#identity-and-labels).
An omitted name leaves only the technical ID available as a label. Invalid names
fail configuration validation. Names are display labels only and have no effect
on identities, references, selection, ordering, execution or canonical rule
digests.

## Documents and safe resources

Governance documents are TOML encoded as UTF-8. Each metadata document is bounded
to 64 KiB. Each scanned directory is bounded to 256 entries and the combined local
Rule definitions to 256 Rules; limits cannot be bypassed by adding another directory.
Required text is nonblank and contains no NUL. TOML syntax and type errors,
duplicate keys, unknown fields, duplicate identities, invalid references and
unsafe resources fail validation, including in disabled Rules. Identity uniqueness
is checked when the configuration is loaded, within the source scope (the local
project or one import binding) and independent of the rules a run selects;
import bindings stay separate. Rule Set IDs are independent of Rule IDs.

Configuration directories and definitions reject symlink traversal and must stay
within their source boundary. Review prompt resources are resolved without
following symlink components. A review prompt is a
Markdown filename directly inside its Rule directory, not an absolute path,
parent traversal or a path containing separators. Existing prompt resources
must be regular files, not symlinks or special files.

A missing implementation file or prompt is an execution prerequisite, not a
structural exemption: unsafe prompt names and existing prompt resource kinds are
rejected while loading, but absence becomes an error only for an enabled Rule
selected by that run.
Inactive Rules need no runtime implementation. A selected review's missing prompt
is a configuration error that prevents success even when the review is advisory.
Executables, credentials and adapter profile requirements are late provider
prerequisites, checked only when their Rule runs; they are ordinary ERROR and
no configuration error. Detailed Rule, adapter profile and import validation follows the
[configuration reference](https://github.com/please-behave/docs/blob/main/src/configuration.md).

## Validation and reconciliation

Loading and validation execute no check or adapter and change no Git
objects, index or configuration. `please sync` validates the configuration and
reconciles its declared external state after confirmation; `--yes` supplies
unattended confirmation. Hook requirements derive from effective assignments,
with each managed hook invoking its identically named execution point. Hook
reconciliation and import refresh are distinct from executing Rules. Offline plan
preview uses previously validated imports without fetching or reconciling state,
as defined by the [Execution Plan API](../plan/api.yaml).
