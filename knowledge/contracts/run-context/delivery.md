# Run context delivery and lifetime

Producer: Please Behave. Consumers: executed checks and external review adapters.
The [document and patch contract](document.md) defines their input. Both contracts
share the API version in [api.yaml](api.yaml); patch files have no separate version.

## Delivery

A run that executes rules writes one context document. All checks and adapters
in that run receive the same context. A check command's `@context` token resolves
to its absolute native path after argument splitting. An adapter receives that
path in the request's `context_file`, as defined by the
[Adapter Request](../adapter/request.md). Consumers must use the supplied path
rather than infer a location or invoke Git to obtain the supplied facts.

Context preparation reads Git without changing its objects, references or index
and without optional locks. It executes no check or adapter. Failure to establish
the context makes the run ERROR and prevents PASS. A check whose context reference
cannot be resolved is ERROR without a fallback.

## Storage and access

The context directory is `please/context/<run-id>/` inside the worktree-local Git
directory. Linked worktrees have separate context storage. The document is
`context.json`; referenced patch files reside beside it. These files are outside
the working tree and input snapshot, so writing them does not change run inputs.

Context files are readable only by their owner. On Unix, run directories use mode
0700 and files use mode 0600. Checks and adapters must treat the document and
patches as read-only. They run with the invoking user's permissions; this
obligation does not provide a sandbox.

## Retention

At the start of a run, old context directories are removed according to
`context.retain_runs`, retaining that many newest earlier runs plus the current
run. Earlier runs count regardless of whether they finished; contexts have no
completed or aborted marker. The current run does not count toward the limit.
Configuration defines the setting's default and permitted values. The context is
transient input rather than a permanent report: consumers needing it later must
account for retention. A saved report's snapshot is a separate representation, and report retention
(`report.retain_runs`) is independent of this setting.
