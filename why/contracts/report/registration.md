# Rule, Rule Set and deployment target registration

`please register --gateway-url <URL>` registers committed rule and Rule Set versions
and configured deployment targets from the root of their Git working tree. The
owner invokes it from CI. It has no execution point or Git hook and runs independently
of verification and attestation publication (why:DEC-0012).

The command shares the [delivery settings and HTTP origin rules](document.md#publishing-evidence)
of `please publish`. The origin is required on every invocation. `[publish]`
selects the authentication method and can override the
repository identity derived from the network Git origin. The identity used for
registration must be a normalized Git host and path that the gateway accepts.
Redirects are disabled. Errors omit credentials, response bodies and URL details
beyond the origin.
With GitHub OIDC, the CLI uses the lowercase repository identity as the source
for each rule and set before computing any rule digest. The gateway requires the source to match
the workflow's repository.

A rule directory contains `rule.toml`. Discovery uses the current commit,
the index and nonignored untracked paths. It includes disabled rules and rules
outside `.please/` in the same repository. A definition at the repository root
uses `.` as its path. Imported definitions in external repositories are registered
by their owners. An empty batch is omitted.

Before sending, the command requires a current commit and verifies the rule
directories against it. It checks the exact bytes read for each definition,
knowledge document and review prompt against their committed blobs. It also
checks Git status for staged, unstaged and nonignored untracked changes within the
rule directories. Missing versions, missing `rule.yaml`, unreadable inputs or
changes within the rule directories fail the entire preparation. Unrelated
working changes do not prevent registration.

The command sends the commit and all prepared rules once, in repository-relative
directory order, using the [gateway registration contract](https://github.com/please-behave/gateway/blob/main/why/contracts/ingest/rule-registration.md).
The batch must fit the gateway's request limits. The rule payload is the input
described by [the canonical digest contract](https://github.com/please-behave/ledger/blob/main/why/contracts/rule-digest/digest.md).
It includes the source, path, ID, version, kind, optional declared name, description
and complete parsed knowledge. Reviews also carry the effective blocking value
and the original prompt. The commit is provenance outside the digest.

The command prints `created`, `unchanged` or `conflict` for each rule after
receiving a receipt with the expected number of outcomes. It compares each
returned digest with the canonical digest of the exact submitted rule. A
conflict reports that content changed without a new version, and the original
registration remains stored. Other outcomes retain their independent meaning.
The exit status is 0 when every outcome succeeds and 2 for a conflict, digest
mismatch, invalid receipt, gateway rejection, transport failure or invalid local
input. Gateway rejections and transport failures have distinct diagnostics.

## Rule Sets

Registration includes local definitions in `.please/rule-sets/*.toml` and
`ruleset.toml` definitions at any directory in the source repository. Local
sets resolve members by ID among local Rules. Definitions named `ruleset.toml`
reference Rule directories relative to the repository root, as for an import.
The set's path identifies its containing directory.

Before sending any batch, the command prepares each set using its committed
definition and sibling [`ruleset.yaml`](../rule-knowledge/README.md). It
requires a set version and resolves each member to the rule registration it
already prepared. Each member carries the exact submitted rule's source,
path, ID, version and digest, plus its declared enabled state. The set carries
its own source, path, ID, version, optional name, description and parsed
knowledge.

The command sends one set batch, in definition path order, using the same
commit and the [gateway Rule Set registration contract](https://github.com/please-behave/gateway/blob/main/why/contracts/ingest/rule-set-registration.md).
It sends the set batch after processing the rule receipt, including when the
receipt reports a rule conflict or a digest mismatch.

The command reports each set's `created`, `unchanged`, `conflict` or
`invalid_members` outcome. It identifies each failing member and whether the
ledger found an unregistered version or a digest mismatch. It accepts the set
digest from the ledger. A conflict, member failure, invalid receipt, gateway
rejection or transport failure makes the command exit with status 2.

## Deployment targets

The command validates every [target declaration](../configuration/project.md#deployment-targets)
before making any registration request. It sends one batch in configuration
order to [`POST /v0/target-registrations`](https://github.com/please-behave/gateway/blob/main/why/contracts/ingest/target-registration.md)
after processing any rule and Rule Set receipts. It can send a target batch
on its own. An empty list omits the batch. The batch contains only the target
names and any descriptions. The gateway derives the owner from the authenticated
sender (why:DEC-0022).

The gateway receipt must contain one outcome per entry with the exact name in
the same order. The command prints `created` or `unchanged` for each target
after validating the receipt. Both outcomes succeed, and a repeated registration
preserves the original description. Invalid declarations, invalid receipts,
gateway rejection and transport failure exit with status 2. Outcome failures
reported in earlier valid receipts still cause exit status 2 after target
delivery. A failed request or invalid receipt stops delivery of later batches.
