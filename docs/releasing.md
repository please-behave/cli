# Releases and pipeline quality

Releases are started manually from **Actions → Release → Run workflow → main**.
No hand-written version tag is needed. CI runs automatically on pushes to main
and pull requests. Releases verify exact-commit CI evidence, four native candidates and shared release evidence before publishing.

CI runs have no project-defined concurrency limit or automatic cancellation of
older runs. Releases execute one at a time to avoid competing version/tag writes.
Newer dispatches do not cancel a running release, but GitHub's default concurrency
queue allows only one pending release and replaces it if another is dispatched.
GitHub account limits and runner availability still apply.

From the source checkout, the equivalent command is:

```sh
mise release
```

This dispatches `gh workflow run release.yml --ref main`, assuming `gh` is already
authenticated with the required permissions. It starts the workflow for **remote
main** and returns after dispatch; it neither commits/pushes local changes nor
waits for publication. Follow progress in GitHub Actions or with `gh run list
--workflow release.yml`. `mise run release` is equivalent. `mise release --install` additionally
waits first until no CI run on main is queued or in progress, then, after dispatch, waits for the
dispatched run and, if it published a release for that source commit, sets
the governor pin in `mise.toml` to the new version and runs `mise install`. It stops if
`mise.toml` has uncommitted changes, never commits or pushes, and leaves the pin untouched
when the run published nothing. Review and commit the pin separately; governance
configuration may need migrating first. Candidate builds remain separate; see the
[source build instructions](../README.md#build-locally).

## Version policy

Semantic Release determines the next version from commits since the last `v*`
release tag, then generates release notes for the GitHub Release:

| Commit | Release |
| --- | --- |
| `feat` | Minor |
| `fix`, `perf`, `chore`, `build`, `refactor`, `config`, effective `revert` | Patch |
| Any allowed type with `!` or a `BREAKING CHANGE:` footer | Minor during `0.x`; Major after explicit stable graduation |
| `ci`, `docs`, `style`, `test` without a breaking change | None |
| Historical `build(release)` version commit without a breaking change | None |

The external repository-conventions Rule Set allows twelve types and optional
scopes from [.please/commit-scopes.txt](../.please/commit-scopes.txt). The
Conventional Commits parser recognizes the documented breaking `!` syntax.
The commit-msg hook validates syntax and runs an advisory header-only wording
Review. CI and release readiness do not validate commit ranges or PR titles.
Semantic Release still analyzes history to determine versions and release notes.

The highest required increment across all effective commits wins. During 0.x,
features and breaking changes both require minor; patch changes never override them. Patch maintenance (`chore`, `build`, `refactor`, `config`, `revert`)
appears in the release notes alongside features, fixes and performance changes.
Non-release types stay hidden unless they contain a breaking change.
Historical `build(release)` version commits remain hidden from notes. Releases no
longer generate version commits.

A revert of an already released change triggers a patch. An original commit and
its revert within the same unreleased range cancel each other in version
calculation and release notes. Include the exact original header and commit hash
so Semantic Release can recognize the pair; optional scopes are supported:

```text
revert(cli): feat(cli): add an experimental command

This reverts commit FULL_ORIGINAL_COMMIT_SHA.
```

When using `git revert`, edit its default message into the accepted format and
preserve the `This reverts commit ...` line. Other effective changes in the range
still determine the release; breaking changes require minor during 0.x and major
after stable graduation.

Please Behave starts at **0.1.0** and stays in the `0.x` development series. Features and
breaking changes increment the minor version; fixes and maintenance increment
patch. Moving to `1.0.0` requires an explicit change to the development-series
policy in release.config.mjs, not merely a breaking commit.

The published v0.1.0 tag establishes the release history. Semantic Release
calculates subsequent versions and handles publication. If no release-worthy
commits exist after a release, the workflow exits without building or publishing.
The Node version range in [package.json](../package.json) and
`npm ci --ignore-scripts` are needed only for release tooling, not to build or run
Please Behave. Dependencies are locked in `package-lock.json`.

## Release sequence

1. `plan` (Plan) uses a Semantic Release dry run on main to calculate the proposed
   version and bind it to the dispatched source SHA. Prepare/publish plugins do
   not run; no tag, commit or release is created.
2. `release-readiness` (Release Readiness) and `release-candidate` (Release Candidate)
   depend only on Plan and execute independently in parallel when a release is needed.
   Readiness runs `python3 .github/scripts/check-release-ci.py` directly to verify successful push CI
   for the exact source, repository, workflow, main branch and required jobs.
   Source checks, dependency audits and unit tests are reused from CI without an age limit.
3. The four-platform Candidate matrix installs the published governor, synchronizes
   imports and stamps Cargo.toml/Cargo.lock with the planned version. Actions builds first, then invokes the independent local release-smoke Rule
   at `release` against the actual binary on each native host, then packages and
   verifies it. Formatting, Clippy and unit tests are not repeated.
4. Actions records a receipt binding the binary hash before/after quality testing to the
   planned version, source, target, workflow run and attempt. Packaging checks the
   executable version and copies exactly those tested bytes, with README and LICENSE.
   `.github/scripts/check-release-artifacts.py --candidate` validates each individual archive before upload. Artifacts
   are named `please-RUN_ID-RUN_ATTEMPT-TARGET`; metadata and smoke receipts travel
   with the archives but are not public release assets.
5. `publish` (Publish) requires Plan, Readiness and the entire Candidate matrix to
   succeed. It downloads only current-run/current-attempt artifacts and generates
   complete `SHA256SUMS`. A mandatory direct call to
   `python3 .github/scripts/check-release-artifacts.py` verifies exactly four archives and
   their evidence, source/version/run identity, archive/binary/smoke hashes, executable
   permissions, allowed members and README/LICENSE bytes. It never rebuilds binaries.
   Publish verifies common release evidence directly with `.github/scripts/release-evidence.py verify`; it installs no governor or build toolchain.
6. Semantic Release recalculates the version, requires it to match the plan, and
   checks local HEAD, the dispatched SHA and remote main before creating a tag.
   It generates notes, tags the tested source, pushes the tag/Git notes and publishes
   archives, checksums, README and LICENSE. Nothing goes to npm or crates.io. No
   changelog, release commit, issue/PR comment or release label is generated.

### CI proof and reruns

`.github/release-ci.json` explicitly requires the `Check` job in
`.github/workflows/ci.yml`, in `please-behave/cli`, for a push to main at the exact
release SHA. Current CI has one Linux x64 job; four-platform coverage is supplied
by Release Candidate. Consumers with a CI matrix must list every required entry
by a stable, unique job name. Individual workflow steps are not configured or checked.

Readiness resolves repository/workflow IDs and selects the newest matching run,
without filtering for success. Missing, pending or failed runs block. Successful
partial CI reruns are supported: each required job's newest execution within that
same run must succeed, while retained successes from earlier attempts count.
Unrelated runs never combine and an older green run cannot replace a newer failed
or incomplete run. API failures, ambiguous identities and incomplete evidence
produce ERROR. Readiness needs `actions: read` and `contents: read`; it never
starts CI, waits indefinitely or uses its own release run as proof. CI must remain
independent of release validation. Evidence is observed at verification time and
is not a lock against subsequent reruns. API evidence is bounded to 100 attempts and fewer than 1000 listing entries;
verification fails closed when evidence is unavailable or incomplete.

Release artifact transfer is stricter: all four packages must exist in the current
Release run attempt. Rerunning only Publish or a subset of Candidate jobs cannot
reuse artifacts from an earlier attempt; rerun the entire Release workflow if needed.
This does not restrict partial reruns of the independent CI workflow.

Cargo.toml and Cargo.lock retain their committed development version. Version
stamping happens only in the native build checkouts, before compilation and smoke testing;
Cargo embeds that version in the binaries. Packaging checks the executable version
against the stamped manifest and planned tag, and records the source SHA and
binary/archive hashes. The direct artifact verification step checks those records
against the planned version and dispatched source revision. Rebuilding a release version from its tag
requires the same `.github/scripts/set-release-version.py VERSION` build-time stamping.
Semantic Release has no version preparation step and never commits generated files.

If main advanced while building, publication is refused; rerun on its latest
commit. Do not push new work during publication. The publisher does not update
the main branch. Release operations are not atomic: a network/API failure after tagging can leave a tag or partially
uploaded release. Inspect the workflow log and remote state before retrying;
never assume a failed run published nothing or delete release tags blindly.
A repeat run after a completed release with no new release-worthy commits exits
without building or publishing again. Partial publication still needs inspection.

## Native downloads

Published releases provide four native archives and `SHA256SUMS`. Build jobs
transfer archives, metadata and smoke receipts using run/attempt-specific artifact
names. CI and all four native Candidate jobs use the SHA-pinned
[Action](https://github.com/please-behave/action).
Mise installs the governor pinned in `mise.toml`. Each gate uses `install: 'false'`
and `sync: 'true'` to synchronize and execute in one Action step with a Job Summary.
Candidate builds and packaging use
`mise run build --offline` and `mise run package`; integrity checks remain
explicit workflow steps.
Release gates run the `release` point, which selects no reviews; no workflow uses `checks-only`, which is for local development.
The governor is always a published binary, separate from the candidate under test.

| Platform | Rust target | Archive |
| --- | --- | --- |
| macOS Apple Silicon | `aarch64-apple-darwin` | `please-vVERSION-aarch64-apple-darwin.tar.gz` |
| macOS Intel | `x86_64-apple-darwin` | `please-vVERSION-x86_64-apple-darwin.tar.gz` |
| Linux x64 (GNU/glibc) | `x86_64-unknown-linux-gnu` | `please-vVERSION-x86_64-unknown-linux-gnu.tar.gz` |
| Linux ARM64 (GNU/glibc) | `aarch64-unknown-linux-gnu` | `please-vVERSION-aarch64-unknown-linux-gnu.tar.gz` |

Runners are `macos-15`, `macos-15-intel`, `ubuntu-24.04` and
`ubuntu-24.04-arm` respectively. Linux ARM64 is built and smoke-tested natively. Windows
builds/releases are intentionally excluded. Linux is built on Ubuntu 24.04; this
is not a static musl binary or a promise of compatibility with older glibc systems.
Verify downloads with `shasum -a 256 -c SHA256SUMS` on macOS or
`sha256sum -c SHA256SUMS` on Linux (select the downloaded archive's checksum line
if downloading only one platform). Checksums verify integrity, not signed identity.
No code signing, macOS notarization or auto-update mechanism is included.
Native downloads still need Git and, for enabled reviews, Codex CLI/login.

## Quality gates

- Push/PR: Linux x64 formatting, Clippy, unit/acceptance tests, optimized build and smoke. Release: versioned builds and real CLI smoke on all four native hosts. Tests use fake providers,
  never real Codex, credentials or paid APIs.
- Linux: test Git hook integration offline.
- Workflow syntax/expressions: pinned actionlint. Version stamping and
  archive-integrity checks: credential-free tests.
- Dependency audits: cargo-audit checks Cargo.lock against RustSec; npm audit
  blocks high/critical release-tooling advisories. Moderate npm advisories remain
  visible but do not block. Advisory feeds require network access; unavailable
  audits fail the job rather than silently pass.
- Dependency updates use Renovate, with `chore(deps)` commit prefixes to satisfy the
  commit policy. Action references use full commit SHAs; runtime/tool versions
  and dependency updates are reviewed.

## Permissions and local checks

Normal CI and native builds have read-only repository access and checkout never
persists credentials. Only release planning/publishing request `contents: write`:
Semantic Release verifies Git push permission even in a dry run. The job-scoped
`GITHUB_TOKEN` suffices when repository rules permit release tags and Git notes.
No personal login credentials or Codex secrets belong in CI.

Repository rules must permit the release identity to push tags and Git notes.
No main-branch release commit or branch-protection bypass is needed.

Prepare the published governor and dependencies before committing workflow/tooling
changes. Git hooks run local validation at commit and push; agents must not run
`please run --at pre-push` or other checks manually. The [quality-gate catalogue](self-governance.md)
documents every execution point, installation, the Renovate-managed pin and report artifacts.

The pipeline also runs actionlint, and dependency audits.
Do not run `npm run release` locally as a test: it is a publishing command. Local
validation cannot certify other hosted platforms; inspect all GitHub jobs before
using a release.

References: [Semantic Release configuration](https://semantic-release.gitbook.io/semantic-release/usage/configuration),
[commit analysis](https://github.com/semantic-release/commit-analyzer),
[GitHub runner images](https://github.com/actions/runner-images).

## Shared evidence

GitHub Actions runs `.github/scripts/release-evidence.py record` after successful
candidate verification and `.github/scripts/release-evidence.py verify` on the collected
set before publication. `.github/release-artifacts.json` names each target's
archive, metadata and smoke receipt. The manifest binds hashes to version,
source, repository, workflow and current run/attempt. This preserves the previous
pinned implementation as workflow tooling, outside Please Behave.
The local `.github/scripts/release-guard.mjs` verifies source and version before tagging.
