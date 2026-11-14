# Implementation design

This document records architectural decisions. The [specification](../SPECIFICATION.md)
defines behavior; this file explains how the code is organized to deliver it.

## Modules

`config` defines the project, Rule and Rule Set schemas and validates loaded objects.
`discovery` reads only the fixed configuration paths and resolves Rule resources within
their source boundaries. Local discovery and explicit external references share Rule
loading. `rules` resolves effective assignments; `planner` prepares selection and review
context; `checks` uses the shared process runner; `reviews` validates provider verdicts;
`report` persists results; `terminal` renders them. There are no alternative
configuration loaders or execution engines.

Loaded Checks hold a tokenized command, the source root and the declaring Rule
directory; execution resolves its references anew without following symlinks. Loaded Reviews hold prompt text and consumer-relative
context. There is no parallel metadata registry: blocking belongs to each Review and
follows it into plans and results. Plan and report schema versions describe output
formats, independently of configuration.

## Desired-state reconciliation

Configuration validation describes the valid desired state; `reconciliation` compares it
with external state and builds an ordered plan. The CLI owns confirmation; `sync`, direct
Git-backed `run` points and `sync --yes` share one inspection, application and
verification path. Git hooks are the only reconciliation domain today, so no generic
domain registry exists. Direct `check` and custom points skip hook handling but keep the
repository recursion guard.

The plan records before and after hook bytes and modes. Application rechecks the plan,
stages replacements and uses per-file atomic renames with best-effort rollback; it does
not promise multi-file atomicity or protection from hostile concurrent replacement.
Ownership markers bind managed sections to the common Git directory, so default hooks
serve linked worktrees. Composition only inserts a section before existing POSIX sh
commands; no shell parser or rewriting is used. No provider or network participates.
See [usage and recovery](usage.md#optional-git-hook).

## Snapshots

A snapshot is Git HEAD plus a sorted manifest of hashes, sizes and executable flags
for tracked and nonignored untracked files, plus declared governance inputs even when
ignored (INP-001). Discovery is read-only and uses no temporary commits, index writes
or optional Git locks. Only hashes are stored, so line deltas are unavailable by
design. Snapshots are not filesystem transactions: they are captured before and after
execution and before provider calls, and changes between captures block PASS.
Incomplete coverage always takes precedence over known differences.

## Execution and reporting

All checks tokenize a command string at load time and resolve its references at execution,
using the bounded process runner without an implicit shell. The runner drains both pipes concurrently, keeps only
the first bytes of each stream with full totals, runs POSIX processes in separate groups
and kills descendants on timeout or fail-fast. It is not a hostile-process sandbox.

Reports are written to a temporary file and renamed atomically over `last.json`. They are
unsigned local evidence. Provider review durations are those of their own call; missing
token counters stay unavailable and are never estimated.

## Providers

The planner prepares provider-neutral review input once; execution passes that exact
preparation to the provider, and rendering does no filesystem or Git work. Every review
is one adapter call and reuses the prepared context. Adapters receive evidence and the common verdict schema through the
[versioned protocol](adapter-protocol.md); the core supervises processes and validates
coverage, evidence paths, line ranges and input stability. No provider or default is
supplied.

The provider trait returns a raw document so tests can inject successful, failed or
malformed responses without credentials. Serde types reject unknown fields, wrong types
and duplicate keys, and an invalid document makes its review ERROR. Grounded references are necessary, not proof of accurate conclusions.

Parameter declarations live in the Rule schema and the `parameters` module resolves only
active Rules, removing inherited variables before applying values; no global environment
mutation occurs. File-change triggers filter reviews inside the planner before input
preparation, using a net commit-to-worktree comparison against an explicit baseline that
enters plan identity. Commit-header reviews use a separate evidence source: adapters
receive the bounded first line, never the raw path or hook arguments.

Check dependencies are canonicalized at discovery, validated against the effective
selection with cycle detection before execution, and dispatched by the existing
scheduler only when all prerequisites PASS and a resource is free. Reviews stay in a
separate phase behind the checks' aggregate outcome and snapshot validation.

## External sources

`external` supplies each import binding to the ordinary discovery and validation flow;
`external::cache` owns host Git, shared object storage, immutable checkouts, integrity
receipts and OS locks. Each selected directory contains exactly one Rule Set whose Rule
directories resolve inside the immutable checkout, so sets can share definitions. Local
resolution runs first, per-import mappings translate effective assignments afterwards,
and namespacing follows validation. No second execution engine exists.

`sync` refreshes sources and publishes per-project resolution state atomically, only
after complete validation. Execution and previews read that state and verify cache
integrity without fetching. Repositories are keyed by the selector's hash and checkouts
by resolved commit; no URL is persisted, and Git plumbing materializes commits without
hooks, global filters or submodules. See
[storage, trust and recovery](external-rule-sets.md).

Named and independently imported provider configurations reuse `ProviderConfig` and the
bounded TOML loader and share cache retrieval and resolution state. Provider origins
enter plan identity, and references resolve after merging all explicit imports.

The repository is governed by a separately installed published release; candidate
binaries run only in isolated fixture repositories.
