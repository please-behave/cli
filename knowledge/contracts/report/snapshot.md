# Snapshot and freshness

A snapshot is part of the [Verification Report API](api.yaml), sharing the report's
version. [report.schema.json](report.schema.json) defines its structure and the
manifest's representable bounds. This contract defines coverage, integrity and
comparison. The [Run Context API](../run-context/api.yaml) is a separate document
for passing Git facts to checks and adapters.

## Covered inputs

`snapshot.head` identifies Git HEAD, or null for an unborn repository. `files` is a
manifest keyed by repository-relative paths, preserving path spelling. Each
entry records its content SHA-256, byte size and executable flag. The manifest
contains tracked files and nonignored untracked files, plus declared local
governance inputs even when ignored: configuration, Rule prompts and entrypoints,
review context and local files referenced by checks, adapters and parameters.
Disabled Rules' governing inputs remain covered. References respect their
repository, source and Rule boundaries; arbitrary script dependencies are not
inferred. Deleted files are absent from the captured manifest.

Unreferenced ignored files, Git internal data and `.please/reports` are excluded.
Writing a report therefore does not make the saved state stale. Imported cache
contents are protected by import integrity verification rather than copied into
the project's snapshot. Internal scheduling history is not a public API.

`fingerprint` is an opaque identity of the captured HEAD and manifest, distinct
from semantic `plan_id`. Consumers must not derive it from the report JSON or
rely on a particular serialization or aggregate hashing algorithm. File content
hashes identify contents; they do not preserve the old content itself. Snapshot
completeness must be considered alongside any identity match.

## Completeness and limits

`complete` states whether all required inputs could be captured. `limitations`
explains gaps. Unsupported symlinks, submodules, special files, non-UTF-8 paths,
unreadable files and exceeded bounds cannot be silently skipped as complete.
The schema bounds each captured file and the manifest's path count. Total captured
content and Git path-list output are each bounded to 128 MiB; `.please` traversal
is limited to 16 levels. These are coverage limits, not permission to pass with a
partial manifest. An incomplete snapshot blocks execution and prevents PASS.

Capturing and comparing state is read-only: no temporary commits, object/ref/index
changes, fetches or optional Git locks. A snapshot is not a filesystem transaction
or sandbox. Source stability is checked before adapter invocation and at run
completion, including after fail-fast or cancellation. Changed or unverifiable
inputs prevent PASS. A captured commit message has its own stability checks and
belongs to its invocation; it is not a working-tree file in this snapshot.

## Freshness

`please status` captures current inputs and compares them with the saved snapshot,
without running Rules. CURRENT requires complete coverage on both sides, the same
HEAD and the same paths and file states. STALE means complete coverage with a
changed HEAD, addition, deletion, content, size or executable flag. Incomplete or
unavailable comparison is UNKNOWN, even when known differences exist.

Staging alone does not change freshness because index membership is not the
working-file baseline. Changing HEAD does. A file changed and then restored to
its saved state does not remain stale merely because Git observed intermediate
changes. Freshness is separate from the report's PASS, FAIL or ERROR outcome.

Changed-path samples are bounded by the saved display setting and the presentation
cap of five paths. Exact changed-file totals are shown only for complete
comparisons; a HEAD change is identified separately rather than counted as a
file. No line-count deltas are supplied, since the snapshot stores hashes rather
than prior text. Git's staged/unstaged/untracked summary is orientation only and
must not substitute for comparison with the saved verification inputs.
