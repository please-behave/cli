# Rule version registration

`please register --gateway-url <URL>` registers committed rule versions from the
root of the Git working tree that holds their definitions. The rule owner
invokes it from CI. It has no execution point or Git hook and runs independently
of verification and attestation publication (why:DEC-0012).

The command shares the [delivery settings and HTTP origin rules](document.md#publishing-evidence)
of `please publish`. The origin is required on every invocation. `[publish]`
selects the authentication method and can override the
repository identity derived from the network Git origin. The identity used for
registration must be a normalized Git host and path that the gateway accepts.
Redirects are disabled. Errors omit credentials, response bodies and URL details
beyond the origin.
With GitHub OIDC, the gateway rejects a registration whose source differs from
the workflow's repository.

A rule directory contains `rule.toml`. Discovery uses the current commit,
the index and nonignored untracked paths. It includes disabled rules and rules
outside `.please/` in the same repository. A definition at the repository root
uses `.` as its path. Imported definitions in external repositories are registered
by their owners. A repository with no rule directories succeeds without a request.

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
