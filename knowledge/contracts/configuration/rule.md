# Rule definition

Producer: a local or imported Rule author. Consumer: Please Behave.
[rule.schema.json](rule.schema.json) defines the shared fields and the structure
of the check and review implementation tables. It references the
[parameter declarations schema](parameters.schema.json). These contracts share
the Governance Configuration API version in [api.yaml](api.yaml), with no version
field in `rule.toml`. [Discovery](discovery.md) defines locations and safe resources.

## One definition, one implementation

A Rule declares exactly one implementation: a check or a review. Its implementation
must match its directory kind, including for imported Rules. A check declares a
command even when disabled. A review declares a prompt filename even when disabled;
the prompt file itself need not exist until the Rule is selected. Neither missing
implementations nor disabled state permit unknown fields or ambiguous structure.
The schema describes parsed TOML, not null-filled internal serialization; an absent
optional field is not an explicit null configuration value.

The check and review table structures participate in the same configuration API.
Their command, resource, prompt, provider, trigger and context semantics are defined
by the [check reference](../../../docs/configuration.md#add-a-check) and
[review reference](../../../docs/configuration.md#add-a-review). Structural schema
validation alone does not validate a command grammar, file path, trigger or evidence.

## Identity and labels

`id` is the technical Rule identity, case-sensitive and independent of its directory
name. Its syntax, bounds and reserved prefix are defined by the schema. Local Rule
IDs are unique across checks and reviews in their source scope; imported Rules
retain distinct, binding-qualified identities. Every reference resolves to one exact
identity, and duplicate identities are configuration errors even for identical
definitions. Rule Set IDs use their own uniqueness scope.

`name` is an optional display label. Without it, the Rule's source-local ID is the
label, including for imported Rules. Labels do not change Rule identity, references,
selection, parameters, ordering or provider references. Display names that collide
are distinguished by an origin suffix and an occurrence number when still needed.
Plans and reports retain labels and provenance so saved status remains readable
after configuration changes. `description` explains the expectation and must be
nonblank and contain no NUL.

## Enabled state, enforcement and assignment

`enabled` controls whether a Rule may execute. Disabled definitions remain
structurally validated and visible in plans, but never produce a Rule result.
Runtime prerequisites and parameter resolution are needed only by enabled selected
Rules. An enabled Rule needs an effective execution point.

`blocking` is a review property. Checks are always blocking and must not declare
it, even as true. Reviews use its documented default when absent. Ordinary FAIL
or ERROR outcomes obey this setting; parameter and missing-prompt configuration
errors still prevent success for advisory reviews. The
[Report API](../report/document.md) defines outcome aggregation and error precedence.
A Rule Set cannot enable, disable or change the enforcement of a referenced Rule.

`run_at` declares a Rule's own execution points. A Rule referenced by Rule Sets
uses the union of those reference assignments instead. A reference inherits its
set's points unless it supplies its own nonempty replacement. All references and
point lists are validated, including disabled and overridden assignments. A Rule
executes at most once per run, regardless of repeated references. The
[prepared-plan contract](../plan/prepared.md) defines effective selection and
canonical ordering. Assignments select a Rule without changing its properties.

`depends_on` is check-only and absent means no prerequisites. Reviews must not
declare it, including an empty array. Check dependencies resolve within the
selected checks; they never select extra Rules. Detailed resolution and execution
follow the [dependency reference](../../../docs/configuration.md#fields-shared-by-checks-and-reviews)
and the prepared-plan and report contracts.

`parameters` declares named Rule inputs under the
[Rule Parameters contract](parameters.md). It is distinct from adapter settings
inside `review.parameters`, which do not declare Rule inputs.
