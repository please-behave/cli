# Prepared plan and identity

[Preview](preview.md) defines delivery, diagnostics and compatibility.
[plan.schema.json](plan.schema.json) defines structure, field types and permitted
values for both output forms. This contract defines the prepared form's meaning;
internal prepared evidence is not an additional serialized API.

## Selection and rules

`at` is the selected execution point after import mappings. Rule Set references
assign execution points: a reference inherits its set's points unless it declares
its own nonempty replacement list. A Rule with memberships uses the union of
reference assignments instead of its own `run_at`; an overridden own list generates
a hint. All references are validated even for disabled rules and overridden
assignments. Present Rule Sets apply without a separate activation flag.

`scheduled` lists selected enabled Rules once, in canonical order: checks by ID,
then reviews by ID. `required_ids` names that full selection in the same order.
Checks are blocking; a review's `blocking` records its configured enforcement.
Required means selected, including advisory reviews; it does not make their
violations blocking. Fail-fast never reduces the plan's selection.

Each Rule retains its technical `id`, kind, description, optional display `name`
and `selected_by` reasons (`at:<point>` for an enabled selected Rule). Display names
are labels, not references. `origin` records the definition's source path, effective
execution points and contributing Rule Set IDs. Imported origins additionally
identify the source repository by a hash of its selector, selected directory,
configured reference and full resolved commit; credentials and machine-specific
cache paths are not provenance. Import mappings change effective points before
selection and keep distinct import scopes and binding-qualified identities.

`depends_on` records sorted, deduplicated canonical check IDs. Dependencies resolve
within the enabled checks selected at this point; they never select extra Rules.
Invalid, ambiguous, disabled, unselected or review references, self-dependencies
and cycles block planning before execution. Reviews neither declare nor satisfy
check dependencies. Execution runs the check phase before any review, scheduling
runnable checks by estimated remaining dependency-path duration with canonical ID
ties. In fail-fast runs with an effective concurrency limit of at least two,
one slot instead picks the shortest estimated runnable check, with canonical ID
ties, before filling other free slots with remaining-path priority. This affects
only dispatch, not the plan's canonical order. Resource waits and completion
order do not change serialized rule order.

`disabled_rules` lists disabled definitions separately, with no selection reasons;
they are never results. `omitted` contains applicable reviews excluded for
`trigger_not_matched`, or `no_changes` when no triggers were declared. They have no
required ID, result, prepared context, parameter resolution or call estimate.
Undeterminable changes block the plan rather than omitting a review as unchanged.

`checks_only` records the invocation mode. It keeps selected reviews visible but
prepares no review input, evaluates no triggers and requires no change base or
adapter availability. Execution reports those reviews as NOT_RUN; success covers
checks alone. Configuration structure is still validated.

## Inputs and bounded review context

`input` identifies the captured working tree by HEAD and snapshot fingerprint,
with completeness and limitations. HEAD may be absent in an unborn repository.
The snapshot covers tracked and nonignored untracked files plus declared governance
inputs; it remains distinct from the [Run Context API](../run-context/api.yaml).
A prepared plan with incomplete input is blocked. Limits and comparison semantics
belong to snapshot integrity, not a new version field in the plan.

Review changes are staged changes at `pre-commit` and `commit-msg`, outgoing
commits at `pre-push`, and net changes from explicit `PLEASE_CHANGE_BASE` at `check`
and custom points. No baseline is inferred or fetched. When needed, `change_base`
records the base resolved once to a commit. Triggered review scopes contain matching
changed files; reviews without triggers receive all changed files.

At `commit-msg`, selected reviews need the managed hook's message file, including
for offline planning; direct preview without it is blocked. Empty messages are
valid. Optional `input.commit_message` contains the complete captured message when
review input was prepared and participates in identity. It belongs to that
invocation, independently of the working-tree snapshot used by status. Execution
rechecks both tree and message stability around adapter calls and at completion.

`context.selections` describes deduplicated context-file and heading selections.
`sections` records the prepared excerpts' original repository paths, inclusive
line ranges and text byte sizes; an end line cannot precede its start line.
`selected_bytes` totals selected excerpt text. `instruction_bytes` totals active
review criteria. Their associated limits bound preparation, not token usage.
`scopes` identifies each review's changed-file paths, included content bytes and
change-set bytes. File content and change sets contribute only when requested.
Prepared evidence is passed unchanged into execution rather than read again.
Failed preparation stays visible as blocking reasons; zero sizes in a blocked
plan do not establish that no evidence was needed.

## Adapter profiles and planned calls

Top-level `adapter` is the default adapter profile, or absent/null when none
is configured. A review with an explicit adapter profile reference records the named ID
in `adapter` and its complete declared configuration in `adapter_profile`.
These fields occur together and only on reviews. `adapter_origin`, when present,
identifies an independently imported named adapter profile with the same provenance
meaning as an imported Rule. Older plans may omit that provenance.

Configuration includes literal arguments, runtime prerequisite declarations,
timeout and arbitrary JSON-compatible adapter profile parameters. This is declared
configuration, not a resolved executable or an availability check. Imported resource
tokens remain portable tokens rather than cache paths. Review-level parameters
are merged for the adapter request, not written back into this adapter profile declaration.
A selected review without a named or default adapter profile blocks the plan.

`adapter_availability_checked` is false: prerequisites are deferred until execution
after checks. `planned_model_calls` estimates one separate adapter call per active
review whose implementation and Rule parameters can be prepared. Shared adapter profiles
never combine reviews into one call. Checks-only, omitted and disabled reviews
contribute none. The estimate is neither an actual call count nor a token or cost
promise; blocking conditions and execution outcomes can prevent calls.

## Semantic identity

`plan_id` is the SHA-256 identity of normalized planning inputs. It includes the
plan API version, verifier product version, project, execution point, mode,
applicable check/review definitions, adapter profile declarations and imported adapter profile
revisions, source HEAD, non-metadata snapshot file states, prepared evidence,
commit message, completeness, resolved change base and selection/omission metadata.
Consumers compare this opaque identity; they must not derive it by hashing the
public plan JSON, which does not serialize every prepared input.

Governance TOML formatting and selector order are normalized separately from raw
file hashes. Review trigger and context-heading sets and check dependencies are
canonicalized. Resource declarations and canonical dependency IDs participate in
identity. Concurrency limits, fail-fast selection/termination options, local
duration history and dispatch order do not. Raw snapshot identity still detects
configuration file changes even when their semantic plan identity is unchanged.
