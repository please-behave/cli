# Rule parameters

Producer: the Rule author declares inputs; the local or importing project supplies
values. Consumer: Please Behave resolves each active Rule's inputs and delivers
them to its check or review adapter. This contract shares the Governance
Configuration API version in [api.yaml](api.yaml), without a new version field.
It applies to both implementations of a [Rule](rule.md).

## Declaration structure and validation phases

[parameters.schema.json](parameters.schema.json) describes the `parameters` table:
a map of names to declarations. Its root describes loading-time structure; the
`activeDeclarations` definition adds name, supported-kind and value constraints
for resolution. Defaults and required fields are defined there. Declarations with
unknown fields, missing structural fields or wrong structural types are rejected
when loading, including in disabled Rules.

Parameter-name syntax, supported parameter kinds and supplied values are checked
when resolving an enabled selected Rule. A structurally readable declaration can
therefore still be an execution error, and an inactive Rule does not require its
values or referenced files. Preview may resolve selected review parameters while
preparing review input or estimating calls; it does not start a process. The
schema does not replace import binding, required-value or filesystem validation.
Resolution preserves TOML type identity rather than coercing other values to strings.

`description` is optional explanatory metadata for the consumer, not a value or
prompt instruction. `required` says whether resolution needs a supplied value.
An omitted optional value remains absent: there is no substituted parameter
default. Schema defaults for absent optional metadata are not TOML null values
and do not inject values into the Rule's input.

## Local and imported binding

A local Rule supplies each value in its declaration's `value`. An imported Rule
must omit declaration values; the consuming project supplies them through that
import's `rule_parameters` table, keyed by source-local Rule ID and parameter
name. Separate imports bind independently. An imported declaration cannot act
as a default when a consuming project supplies nothing.

Supplied names must have corresponding declarations. Resolution validates every
declared parameter of an active Rule, not just those mentioned in its command.
Resolution happens immediately before the Rule runs and fails that Rule alone.
Missing required values, unsupported kinds, invalid names and invalid supplied
values produce an ERROR naming the Rule, parameter and cause. For reviews these
are configuration errors that prevent success even when `blocking` is false.
They are not ordinary advisory review failures.

## String and file values

A `string` value is delivered unchanged, including spaces and characters that
would have shell meaning. A `file` value names an existing regular file relative
to the consuming Git repository root, including for imported Rules. It is not
relative to the Rule directory or an import cache. Absolute paths, parent traversal,
symlink components and targets outside the consuming repository are rejected.
The delivered value is the canonical absolute native UTF-8 path.

File validation happens at resolution; it is not a sandbox or a guarantee against
later filesystem changes. Rule scripts, prompts and bundled resources keep their
own Rule/source path boundaries. Snapshot coverage includes declared local file
inputs under the [Snapshot contract](../report/snapshot.md).

## Delivery

A check receives values only where its command explicitly uses `@param/<name>`.
Resolution happens after command tokenization, and each value remains one argument
without splitting or shell expansion. Referencing an absent optional value is an
execution ERROR, not an empty argument. Values are not exported through hidden
environment variables; inherited `PLEASE_*` variables are scrubbed except for the
internal recursion guard.

Each review adapter receives all its resolved Rule values as `rule_parameters`
under the [Adapter Request contract](../adapter/request.md). They remain separate
from effective adapter profile/review adapter settings. A review has its own adapter call
and parameter environment; sharing an adapter profile never combines Rule inputs or calls.
