# Rule knowledge

A Rule or Rule Set can carry a structured companion document that records why it
exists. This directory defines their structure. The CLI reads the rule knowledge
document when computing a rule digest.

## Why it exists

A Rule definition says what is checked and how. It does not say why the Rule is
worth having, what it protects and how to tell that its implementation does a
good job. Without that, a Rule can only be judged by whether it runs, and an
agent cannot tell whether a change to it, or a new Rule beside it, still serves
the original intent. A Rule Set needs the same: a stated common ground makes it
possible to decide where a new Rule belongs or that a new set is needed.

## Where the documents live

| Document | Location | Schema |
| --- | --- | --- |
| Rule knowledge | `rule.yaml` next to the Rule's `rule.toml` | [rule.schema.json](rule.schema.json) |
| Rule Set knowledge | `ruleset.yaml` next to the Rule Set's configuration | [ruleset.schema.json](ruleset.schema.json) |

The documents carry no identifier. Identity comes from the definition in the same
directory, so there is nothing to keep consistent. Rules and Rule Sets are
referenced by their IDs.

## Rule digests

[REQ-0352](../../requirements/REQ-0352.yaml) defines when a rule has a digest.
For versioned rules with a knowledge document, the CLI uses the
[ledger digest contract](https://github.com/please-behave/ledger/blob/main/why/contracts/rule-digest/digest.md).
The rule directory path is relative to the repository that holds the rule.
The CLI derives the source from the project's network Git origin or the
configured network Git selector of the import.

[REQ-0353](../../requirements/REQ-0353.yaml) requires the digest to include the
complete parsed rule knowledge object, including unfamiliar fields. Values
must be strings, booleans, arrays or objects with unique string keys.
The CLI reads one UTF-8 YAML document within its metadata size bound and
limits expanded aliases and nesting. Rule Set knowledge has no role in the digest.

## What belongs in them

- **The reason, not the definition.** Values the TOML defines, such as execution
  points, timeouts, parameter types and membership, are not repeated. The
  documents explain why those values are right.
- **Rule: purpose, problem, value.** What the Rule achieves, what happens without
  it and the benefit it brings.
- **Rule: conditions and success criteria.** Each condition carries its reason.
  The success criteria are the measure for reviewing the implementation against
  the intent, not a test plan. A Rule that only passes a command to a well-known
  tool, such as a formatter, a linter or a test run, may omit the success criteria
  and the out-of-scope list, because the tool's own result is the yardstick.
- **Rule: boundaries.** `out_of_scope` names the neighboring Rule that owns a
  matter. `limitations` states what a passing result does not prove.
- **Rule: placement.** Why the Rule is shared or local.
- **Rule Set: commonality and admission.** What binds the members and the
  criteria for letting another Rule in, so that "does it belong here" can be
  answered from the document.
- **Rule Set: members.** Every member with its role. The list must equal the set's
  actual membership.

Write each statement short and in plain product language. A statement that no
reviewer could ever see violated does not belong here. Do not mention earlier
forms of a Rule.

## Status

Experimental. Files may change incompatibly, and the schemas follow
[DEC-0026](../../decisions/DEC-0026.yaml)'s version 0 spirit without carrying a
version. The schemas define the expected document structure. Digest computation
uses the ledger contract to validate canonical input.
