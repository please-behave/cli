# Run context document and patches

Producer: Please Behave. Consumers: checks and external adapters.
[context.schema.json](context.schema.json) defines the JSON structure, required
fields, types and permitted values. This contract defines their meaning and the
referenced patch files. [Delivery](delivery.md) defines access and lifetime.

## Version and identity

New documents carry the API version from [api.yaml](api.yaml) in `schema_version`.
During the version-zero transition, the schema accepts both 0 and 1 with identical
structural and semantic obligations. Unsupported versions and invalid version
types must be rejected without retry or fallback. Unknown document fields are
not permitted by the schema. Compatibility follows
[DEC-0026](../../decisions/DEC-0026.yaml), independently of CLI releases.

`run_id` identifies this run; consumers must not parse it for Git facts.
`execution_point` names the selected execution point. The document contains Git
facts, paths and messages, never working-tree file contents. Consumers read source
files from disk; changes may appear in the separate patches.

## Repository, HEAD and workspace

`repository` identifies the absolute worktree Git directory and common Git
directory, whether this is a linked worktree and whether the repository is shallow.
`head` describes the checked-out commit and its message. An unborn repository has
no HEAD commit or message; a detached HEAD has no branch. `message_truncated`
indicates an incomplete HEAD message.

`workspace` describes the index and working tree. `staged` compares the index to
HEAD; `unstaged` compares the working tree to the index. Untracked and conflicted
paths are listed separately. `clean` is true only when nothing is staged, modified,
untracked or conflicted and the status capture is complete. Ignored files do not
make it dirty. A path in both staged and unstaged lists is partially staged: its
on-disk content differs from the index.

Change paths are relative to the repository root and preserve their spelling;
`old_path`, when applicable, identifies the source of a rename or copy. Repository
paths and the commit-message file path are native paths. Patch filenames are
relative to the context directory, not the repository root.

## Conditional sections

Inapplicable sections are absent rather than empty. An applicable section may
have empty lists when there are no changes.

At `commit-msg`, `commit_message` contains the message file's path and text,
with its own truncation indicator. This is the message being evaluated, distinct
from the existing HEAD commit's message. It is absent at other points.

At a managed `pre-push` invocation, `push` describes the remote name and each
reference update supplied by Git on stdin. Each update identifies local and remote
references and object IDs, whether it creates, updates or deletes a reference, and
whether its local tip equals HEAD (`is_head`). A zero remote object ID is represented
as null; a deletion retains Git's zero local object ID.

For an update, `commits` lists commits in the remote-tip-to-local-tip range, and
changed files and patch describe the net tree difference. For a new reference,
commits are those not reachable from the remote's locally known tracking references;
the patch starts before the oldest listed commit, or at the empty tree for an
initial commit. No fetch is performed. A deletion has no outgoing commit range.
`range_available` is false for deletions and unavailable ranges, including a remote
tip missing locally. Empty lists then do not prove that there are no changes.
Consumers needing an unavailable range must obtain it themselves or fail.
`workspace.clean` together with `is_head` lets a rule establish that the working
tree matches the pushed tip.

`changes` is present when `PLEASE_CHANGE_BASE` resolves locally to an available
commit. `base` is that resolved commit; files describe net changes against the
working tree, including untracked files. An unset or unusable base omits this
section without failing context creation. Review selection can independently
require a valid base. The patch contains the tracked tree difference; untracked
paths in the list do not imply their contents are included in that patch.

## Patches and incomplete evidence

Workspace patches compare the index to HEAD and the working tree to the index,
respectively. Push patches compare the trees of the applicable range. Patch files
contain Git diff text without color, external diff drivers or text conversion.
They belong to this API and use the document's version.

Each patch object names its file, counts the diff output bytes and indicates
truncation. A null file with zero bytes and no truncation means no patch was
produced, including an empty or unavailable range. A patch exceeding
`context.max_section_bytes` is omitted entirely, with a null file, its full output
byte count and `truncated: true`; it is never supplied as a partial patch.

The same configured section bound applies to lists and messages. Messages may be
shortened at a UTF-8 character boundary and carry a truncation flag. Lists expose
omission counts and truncation flags; a truncated Git listing need not provide an
exact count of entries never captured. Workspace paths that cannot be represented
as UTF-8 are counted in `unrepresentable_paths` instead of listed under lossy names.
Consumers must inspect the relevant flags and counts: an empty or partial list is
not evidence of completeness. When required facts are incomplete, a rule must fail
or obtain the missing facts from Git itself.
