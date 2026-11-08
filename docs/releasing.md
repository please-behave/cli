# Releases and pipeline quality

Releases are started manually from **Actions → Release → Run workflow → main**.
No hand-written version tag is needed. CI runs automatically on pushes to main
and pull requests. Releases run independent quality gates and native builds before publishing.

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
--workflow release.yml`. `mise run release` is equivalent. `mise deploy` instead
downloads and installs the pinned published Please Behave development governor for the
current OS/CPU, verifying its checksum and executable compatibility before
replacement. Candidate builds remain separate; see the
[source build instructions](../README.md#build-locally).

## Version policy

Semantic Release determines the next version from commits since the last `v*`
release tag, then generates release notes for the GitHub Release:

| Commit | Release |
| --- | --- |
| `feat` | Minor |
| `fix`, `perf`, `chore`, `build`, `refactor`, effective `revert` | Patch |
| Any allowed type with `!` or a `BREAKING CHANGE:` footer | Minor during `0.x`; Major after explicit stable graduation |
| `ci`, `docs`, `style`, `test` without a breaking change | None |
| Historical `build(release)` version commit without a breaking change | None |

The [commit policy](../.githooks/commit-policy) allows eleven types, including
`chore`, and optional scopes from the list in [CONTRIBUTING.md](../CONTRIBUTING.md#commits).
The Conventional Commits parser recognizes the documented breaking `!` syntax.
The local hook and CI reject unsupported types or scopes. CI validates PR commit messages and the PR title used for a
squash merge. Use squash/rebase merging with conventional messages; default merge
commit messages do not satisfy this project's policy.
Push/PR checks validate the proposed commit range; manual releases without a
base validate the selected tip, not legacy commit messages from before this policy.

The highest required increment across all effective commits wins. During 0.x,
features and breaking changes both require minor; patch changes never override them. Patch maintenance (`chore`, `build`, `refactor`, `revert`)
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

Semantic Release's built-in first version is 1.0.0. Therefore, when no semantic
release tag exists, this workflow publishes the initial 0.1.0 through GitHub CLI,
after the same native quality/artifact gates. Source Cargo versions must already
be 0.1.0, and the release/tag targets the exact tested source SHA. From that tag
onward Semantic Release calculates versions and handles publication normally.
If no release-worthy commits exist after a release, the workflow exits without
building or publishing. The initial path uses all history for release notes and
does not create a separate version commit.
Node 24.15+ (24.x) and `npm ci --ignore-scripts` are needed only for release tooling,
not to build or run Please Behave. Dependencies are locked in `package-lock.json`.

## Release sequence

1. Planning on main selects initial 0.1.0 if no semantic tag exists; otherwise a
   Semantic Release dry run calculates the proposed version. Prepare/publish plugins do
   not run; no tag, commit or release is created at this stage.
2. Release checks retain independent tooling, minimum Rust, dependency audits,
   security and license gates. Python integration tests use the published governor
   pinned in `.please-version`. Each native runner stamps Cargo.toml/Cargo.lock
   before formatting, Clippy, tests, candidate build and executable smoke test.
   Normal CI runs all 17 checks on Linux through the governor at `check`.
3. Packaging verifies `please --version` matches the proposed version. It archives
   the tested executable, README and LICENSE, and records hashes, target and
   source commit as internal artifact metadata.
4. Only after CI and all native quality jobs pass, publish downloads the four
   archives and calculates `SHA256SUMS`. A mandatory direct call to
   `python3 scripts/check-release-artifacts.py` verifies completeness, archive/binary
   hashes, executable permissions, packaged documentation, metadata, source commit,
   planned version and checksums. The publication guard then checks the planned
   version and that local/remote main still match the tested source before any
   release tag is created. Publish installs no Please Behave, managed
   Git hooks, Rust, Go, actionlint or Gitleaks and runs no governance checks.
5. For the initial release, GitHub CLI publishes v0.1.0 at the tested source SHA.
   For subsequent releases, Semantic Release calculates the version again, checks
   it against the plan, generates notes, tags the tested source SHA, pushes the tag
   and Git notes, and publishes archives, checksums, README and LICENSE to GitHub
   Releases. Nothing is published to npm or crates.io. Issue/PR comments and release
   labels are disabled. No changelog or release commit is generated.

Cargo.toml and Cargo.lock retain their committed development version. Version
stamping happens only in the native build checkouts, before compilation and tests;
Cargo embeds that version in the binaries. Packaging checks the executable version
against the stamped manifest and planned tag, and records the source SHA and
binary/archive hashes. The direct artifact verification step checks those records
against the planned version and dispatched source revision. Rebuilding a release version from its tag
requires the same `scripts/set-release-version.py VERSION` build-time stamping.
Semantic Release has no version preparation step and never commits generated files.

If main advanced while building, publication is refused; rerun on its latest
commit. Do not push new work during publication. The publisher does not update
the main branch. Release operations are not atomic: a network/API failure after tagging can leave a tag or partially
uploaded release. Inspect the workflow log and remote state before retrying;
never assume a failed run published nothing or delete release tags blindly.
A repeat run after a completed release with no new release-worthy commits exits
without building or publishing again. Partial publication still needs inspection.

## Native downloads

Published v0.1.0 provides the four native archives and `SHA256SUMS`. Build jobs
transfer candidate archives and metadata using matching `please-*` artifact labels.
The [Action repository](https://github.com/please-behave/action) has no released
Action ref yet; CLI governance uses the pinned, checksum-verifying installer.

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

- Push/PR: Linux x64 native gates. Release: all four native hosts. Each runs
  rustfmt, Clippy with warnings denied, unit/acceptance tests, optimized native build and real CLI smoke test. Tests use fake providers,
  never real Codex, credentials or paid APIs.
- Linux: compile-check all Rust targets in the package with the declared minimum
  Rust 1.85, test the Git commit hook, validate commit messages and squash title.
- Workflow syntax/expressions: pinned actionlint. Release rules, breaking-change
  notes, version stamping and archive-integrity checks: credential-free tests.
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
`mise check` or other checks manually. The [quality-gate catalogue](self-governance.md)
documents every execution point, installation, the Renovate-managed pin and report artifacts.

The pipeline also runs actionlint, dependency audits and the minimum-Rust check.
Do not run `npm run release` locally as a test: it is a publishing command. Local
validation cannot certify other hosted platforms; inspect all GitHub jobs before
using a release.

References: [Semantic Release configuration](https://semantic-release.gitbook.io/semantic-release/usage/configuration),
[commit analysis](https://github.com/semantic-release/commit-analyzer),
[GitHub runner images](https://github.com/actions/runner-images).
