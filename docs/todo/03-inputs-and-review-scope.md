# TODO 3: Inputs and review scope

Status: TODO. Depends on: [TODO 1](../execution-points-and-plans.md) and
[TODO 2](02-review-mcp-and-sessions.md).

## Goal and decisions

Separate **which code is verified** from **what a review evaluates**. Both must be
visible in the plan and report; neither is inferred from the command's caller.

| Profile input | Verified content | Intended use |
| --- | --- | --- |
| `worktree` | Current tracked and included untracked working files | Local development |
| `index` | Exact staged tree, excluding unstaged/untracked changes | Pre-commit |
| `commit` | One explicitly selected commit, resolved to an immutable ID | CI or commit verification |

- Never substitute worktree for index/commit. The selected input supplies both
  project rules/context and source evidence; report their identities together.
- Materialize index/commit inputs in an isolated execution directory. Checks and
  provider reads must see that input, including its configuration, rather than
  accidentally reading the original working directory. Never modify the user's
  index, HEAD, hooks or files, or create commits to manufacture a snapshot.
- Git-dependent checks must see truthful repository metadata for the selected
  input. Document the supported isolation strategy and reject unsupported cases
  before checks; do not fabricate branch/history facts.
- Resolve commit references once. For multiple pre-push refs/commits, require
  explicit invocations per intended target; verifying one worktree is insufficient.
- For commit input, use profile `commit_ref` or invocation `--commit REF`; the
  explicit flag wins and is invalid for other input kinds. Require one of them;
  never assume HEAD for an unspecified commit target.
- Review `scope` is `snapshot` by default or explicitly `change`. Snapshot asks
  whether the selected input meets the criterion; change asks whether a defined
  change introduces a violation. Neither claims complete static or dynamic proof.
- Change reviews require profile `diff_base` and `diff_base_mode`: `direct` or
  `merge_base`. Resolve all references to immutable IDs before execution. Local
  HEAD comparison and merge-base with an explicitly named main branch are both
  supported. No implicit standard branch, network fetch or fallback base.
- Missing/ambiguous merge-bases, unavailable history and incomplete evidence are
  errors. A root commit can use the additional explicit mode `empty_tree`, with
  no `diff_base` reference; it is never inferred from missing history.
- Include additions, deletions, renames, binary/unsupported evidence and relevant
  untracked additions for worktree changes. Base-side findings must identify
  the base revision; deleted lines cannot be validated against the target alone.
- Existing `files` and Markdown `context` guide reading and share requirements;
  they do not replace scope or prevent following necessary dependencies.

## Work

- [ ] Add input resolution/materialization with bounded coverage and cleanup.
- [ ] Implement strict commit selector and diff-base configuration/CLI syntax
      as specified above, preserving the English command names.
- [ ] Add review scope, revision-aware evidence and bounded change preparation.
- [ ] Include resolved input/base IDs, comparison mode and scope in plans,
      group identity, reports and freshness checks.
- [ ] Provide explicit pre-commit/pre-push/CI invocation examples without
      installing hooks or provisioning CI credentials.
- [ ] Test partial staging, divergent worktree/index, dirty checkout during commit
      verification, missing bases, deletions, renames and incomplete coverage.

## Done when

A staged file and a different unstaged version produce different input identities
and are reviewed from the correct contents. Commit verification is independent of
unrelated working changes. Missing evidence never becomes PASS. Every finding is
validated against its declared revision. Cleanup preserves original repository
state, including after failure or cancellation. Gate output states exactly which
input and review scope passed.

Implementation commits: none.
