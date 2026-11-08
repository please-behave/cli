# Repository self-integration

Please Behave is developed under a separately installed, published Hannes CLI.
This document describes that development governor and its `.hannes/` configuration;
product commands and configuration use `please` and `.please/`.
Builds under target/ are candidates under test and never become the governor.
The repository uses local Check definitions and a pinned external commit-message
Rule Set and Security Rule Set. A conditional AGENTS.md quality Review uses Gemini
only at check; Git hooks and native release builds remain checks-only.

The exact published governor pin is recorded in [.hannes-version](../.hannes-version).
It must support named CLI providers, runtime requirements and file-change triggers,
as well as direct commands and unattended hook reconciliation.

## Local setup

Prepare tools and dependencies explicitly:

```sh
mise trust
mise install
npm ci --ignore-scripts
cargo +stable fetch --locked
rustup toolchain install 1.85.0 --profile minimal
cargo +1.85.0 fetch --locked
go install github.com/rhysd/actionlint/cmd/actionlint@v1.7.12
mise deploy
export PATH="$HOME/.local/bin:$PATH"
mise hooks:install
```

The setup task delegates hook management to hannes pass auf. If an old clone still
sets core.hooksPath to .githooks, the setup shim removes that local setting only
when all three obsolete event files are absent. If an event file remains, setup
stops and preserves it for manual integration. Other configured hook paths are
retained and reconciled by Hannes. No global Git settings are changed.
Fresh clones can run hannes pass auf directly. Interactive setup displays the
plan and requests confirmation. For unattended setup use:

```sh
hannes pass auf --auto-approve
```

Setup installs no tools or dependencies. Conflicts still fail. Generated wrappers
live at Git's effective hook path and carry local ownership; never check them in
or copy them between repositories. Only the commit policy remains versioned under
[.githooks](../.githooks).

Python 3.11+, Git, Gitleaks v8 with `gitleaks dir` (tested with 8.30.1), stable
Rust with rustfmt/Clippy and the Node version in mise.toml are required locally.
Install Gitleaks explicitly and put it on PATH before running the security checks
or pre-push gate; `gitleaks version` verifies availability. Hooks inherit PATH
and call the installed hannes binary.
Prepare dependencies online first, including Rust 1.85 for the minimum-version
check and actionlint for workflow validation. Cargo build, Clippy, test and MSRV
commands explicitly use --offline; npm tests use --offline too. The execution
wrapper also sets Cargo/npm offline mode and disables Rustup auto-install when
invoking a Git-backed point. Direct Rule commands need no launcher. Dependency
audits run at check in CI with advisory access; they remain outside offline hooks.
Checks neither install the governor nor repair project files.

## Execution points and required coverage

Every selected rule is mandatory. Each point preserves the exact check membership
of its configured execution point. Checks run once per binding, ordered by ID.
The concrete rule definitions and assignments live in `.hannes/`; product tests
use independent synthetic configurations.

| Execution point | Required checks |
| --- | --- |
| pre-commit | format, docs, agents-size |
| commit-msg | commit-message plus imported commit-message |
| pre-push (default) | format, docs, python-tests, agents-size, release-rules, clippy, rust-tests, native-build, native-smoke, workflows, msrv plus imported license, secret-scan and sensitive-files |
| check | All pre-push checks plus commit-range, rust-audit and npm-audit: 17 checks total; conditional agents-quality Review |
| release | format, clippy, rust-tests, native-build, native-smoke |

The 15 local [checks](../.hannes/checks) use string commands. Python scripts run
as `python3 scripts/<name>.py`, relative to the consuming repository root.
The published v0.14.1 governor supports these commands and explicit path
references for the four imported Checks. There are no local index.sh launchers.
Local Rule directories and IDs use descriptive names without numeric prefixes.
The local commit-message Rule and the imported Rule have distinct identities:
imports retain their git-prefixed binding namespace. ID sorting still places
native-build before native-smoke, which consumes the candidate executable.
Each local Rule declares its execution points directly; this repository defines
no local Rule Sets. External Rule Sets supply commit-message, security and
license checks at full Git commit pins.

The imported commit-message, secret-scan and sensitive-files Checks use
`command = "sh @rule/index.sh"`. The license Check uses
`command = "python3 @source/scripts/file-reference.py"` for its shared script.
These references resolve from the cached Rule directory and imported repository
root respectively, while execution stays in the consuming repository root.
Scripts, parameters, timeouts and exit-code handling are unchanged.

All Rust unit, integration, acceptance and documentation tests run in one Cargo
Rule. Python helper tests use one unittest discovery Rule; test modules use
importable test_*.py names and include deployment, release and real Git-hook fixtures.
The public Please Behave Action tests belong to the separate Action repository.
JavaScript release tests run once through npm test. All three suites run at
pre-push and again in CI, including when local hooks were bypassed. Release builds
and the smoke test of the built executable remain separate mandatory checks.
CARGO_BUILD_TARGET selects Cargo's output target in CI; BUILD_TARGET selects the
same binary for smoke testing and packaging. Neither needs an extra launcher.

Formatting, documentation, Clippy, Rust acceptance tests, deployment
and release regression tests, Conventional Commits, actionlint, minimum Rust 1.85,
Rust/npm dependency audits remain required at the points shown above. Audits
need advisory feeds. Release artifact verification runs directly in the publish
job after download and checksum generation; it requires EXPECTED_RELEASE_VERSION,
GITHUB_SHA and a complete dist/ artifact set.
These checks cannot all be meaningfully executed in an unprepared local checkout.

For manual developer use:

```sh
hannes mach was --at pre-commit
hannes mach was --at pre-push --plan
HANNES_CHANGE_BASE="<baseline-commit>" python3 scripts/run-execution-point.py check --reviews
hannes was ist phase --details
```

For explicit manual use, mise check delegates through the execution wrapper to
Hannes at pre-push. Individual format, lint, test, smoke and docs:check tasks
remain available to developers. Format explicitly edits source; verification
only checks formatting.

## Agent workflow

Agents receive validation feedback from Git commit and push. They must not run
checks, tests, linters or builds manually, including mise check, mise docs:check,
individual Cargo/npm commands or direct Hannes execution. Source formatting is
allowed. The fast formatting and documentation checks run at pre-commit; pre-push
runs the complete local suite, including those checks again. There is no result
cache, so a manual full run before pushing would repeat the suite.

If pre-commit or commit-msg fails, fix the issue and retry the commit. If pre-push
fails, fix the issue, amend only the agent's own unpublished commit when appropriate
and retry the push. Do not bypass hooks, force-push or rewrite unrelated work.
Read hook output or use hannes was ist phase --details for saved output without
rerunning rules. Address missing tools or hook drift, or report the blocker.
Prepare the intended commit before the final push: general local checks inspect
working files, while the imported pre-push security and license checks inspect
HEAD.

## Git hooks

The published governor derives the three required Git hooks from effective Rule
assignments. Each hook runs its fixed, identically named execution point. The
repository configuration declares no hook mappings. The governor manages wrappers
through pass auf, including configured hook paths, compatible unmanaged content,
ownership checks, argument forwarding and recursion protection. Wrappers inspect
synchronization read-only; drift tells the developer to rerun pass auf manually.

The `commit-msg` point imports `commit-msg/conventional-commits` from
[the shared Ruleset repository](https://github.com/christianpflugradt/christianpflugradt-rulesets),
pinned to the full commit in `.hannes/config.toml`. It receives
`.hannes/commit-scopes.txt` through the `scopes_file` parameter. Keep this file
consistent with the scopes in `.githooks/commit-policy`.
Both message checks consume HANNES_GIT_HOOK_ARG_1 and require exactly one
commit-msg argument. The imported Rule validates the header, type and scope.
The local `commit-message` check also enforces Hannes' narrower type allowlist
and summary whitespace requirements using the same policy checker as CI.
`pass auf` retrieves and validates the import during setup; subsequent checks and
hooks use its immutable cache offline. A fresh clone needs access to the source
repository during setup.
The same external repository supplies `multi-point/security`, pinned separately
in `.hannes/config.toml`. Its blocking `sensitive-files` and `secret-scan` checks
run at `check` and `pre-push`. `check` scans tracked and nonignored untracked
working-tree files, including unstaged changes; deleted files are excluded.
`pre-push` scans the current `HEAD` tree, without history or other pushed refs.
The filename policy rejects `.env` and `.env.*` except `.env.example`,
`.env.sample` and `.env.template`, plus Terraform state files. Gitleaks scans
contents, including allowed templates; symlink contents are excluded and
submodules cause ERROR. Scanner configuration comes from the selected snapshot.
The external Rule Set documents its complete policy and limitations.

The `repository-conventions` import pins the same migrated full Git commit as
the other imports in `.hannes/config.toml`. It includes only the blocking `license`
Rule at `check` and `pre-push`, with `target = "LICENSE"`. The Rule uses the shared
`scripts/file-reference.py` entrypoint and resolves bundled references relative
to `HANNES_RULE_DIRECTORY`. LICENSE must match Apache-2.0,
PolyForm-Noncommercial-1.0.0 or Unlicense byte-for-byte; this repository uses the
SPDX PolyForm-Noncommercial-1.0.0 reference. `check` reads the working-tree file;
`pre-push` reads HEAD. Missing or nonmatching files produce FAIL; technical errors produce ERROR.

Pre-commit runs before the final message is available. Hooks block on failure and
never modify the Git index. Local checks verify working files, including
unstaged edits, which can differ from staged or pushed commits. The imported
security and license checks explicitly read `HEAD` at pre-push. Hooks are
bypassable; clean CI checkouts remain authoritative. General Hannes index/commit inputs remain
future work.

## CI, pinning and Renovate

[.hannes-version](../.hannes-version) is the single exact governor pin. The
[CI workflow](../.github/workflows/ci.yml) has one **Quality checks** job on Ubuntu
x64. It prepares Python, Node and Go, then explicitly prepares Rust toolchains,
locked dependencies, actionlint, cargo-audit and checksum-verified Gitleaks through
[scripts/prepare-ci.sh](../scripts/prepare-ci.sh). Preparation is outside Hannes
Rule execution. The job installs the pinned published governor through the public
[Hannes Action](https://github.com/christianpflugradt/hannes/tree/eedbc1dbb227110667a8b40bdcf8603bddb33726), resolves imports once and invokes `check` once.
One `quality-check` artifact preserves reports even on failure.

The separate **GitHub Action compatibility** matrix tests the pinned external
Hannes governor Action on Ubuntu x64 and ARM64 using a disposable consumer. It verifies PATH, install-only
mode, custom selection and failure propagation. These runner-specific tests remain
separate from repository checks.

The [release build workflow](../.github/workflows/quality.yml) runs only for
releases, with four native runners and the `release` execution point.
The point runs after release version stamping and before packaging, selecting
the same five reusable native checks with unchanged commands, timeouts and
blocking behavior. It is an ordinary custom point supported by the pinned
published governor; it performs no packaging or publication.
The release workflow also reuses CI before publication. macOS release runners use the
[native installer](../scripts/install-governor.py). The execution wrapper rejects
candidate governors inside the checkout and checks CI's version against the pin.
Both installers download the pinned release's assets directly, without a GitHub
Releases API request. Missing tags, assets or valid checksums fail installation;
there is no fallback to latest. These installation steps need no GitHub token.

The repository uses published Hannes v0.14.1 for both local hooks and CI.
All development governor Action references pin the v0.14.1 source commit; their
version input continues to come from `.hannes-version`.
The AGENTS.md review sets include_diff = false and its Gemini provider sets
timeout_seconds = 0, leaving request deadlines and retries to the REST adapter.
Candidate binaries remain confined to disposable acceptance fixtures and smoke
tests; they never govern this checkout.

Historically, the initial Hannes governor Action was published on main at
[`2d3cac2324ce54f076c7090e5bf80969eee4b37e`](https://github.com/christianpflugradt/hannes/commit/2d3cac2324ce54f076c7090e5bf80969eee4b37e)
before a second commit added public `uses: christianpflugradt/hannes@2d3cac2324ce54f076c7090e5bf80969eee4b37e`
references. This immutable publication revision contains `action.yml`; it has no
separate Action release tag or compiled artifact. No major-version tag is created.
The Action revision and CLI release pin are independent.

The regex custom manager in [renovate.json](../renovate.json) retains the
GitHub-releases datasource and disabled automerge. Pin updates remain normal
reviewed dependency PRs. A format-breaking update may require configuration
changes; every execution point must validate and plan with the proposed release.

Push/PR checks retain native Linux x64, tooling, MSRV and audit coverage. Releases
reuse that CI workflow and run native validation on all four supported hosts. Tool
installation, dependency preparation, release stamping, packaging and publication
remain separate operations. Publish installs no governor, managed hooks or check
prerequisites. It calculates checksums and directly runs
`python3 scripts/check-release-artifacts.py` before publication. This mandatory
step verifies the complete four-platform archive set, metadata, archive/binary
hashes, executable permissions, packaged README/LICENSE, planned version, tested
source SHA and SHA256SUMS. Failures stop the job. The publication plugin retains
the planned-version and local/remote source guards before tagging, without
repeating artifact verification. Release lifecycle tests exercise tag and Git
note pushes in a disposable repository and verify that publication creates no source commit.

CI and native build jobs retain their failure-report uploads.
Schema-4 last.json records execution
point, required rules, source state and results. The [execution wrapper](../scripts/run-execution-point.py)
adds execution.json with governor version/digest, execution point, checked commit
and exit code. Managed hooks and direct CLI invocations write the ordinary report;
the wrapper supplies companion evidence for CI and mise invocations.

## Conditional document review

See [AGENTS.md quality review](agents-quality-review.md) for the 1200-word gate,
whole-document rubric evaluation, named Gemini provider, local execution and CI
summary. The existing check point runs both checks and triggered reviews; the
execution wrapper's --reviews option enables full execution there. Its default
remains checks-only for hooks and native release points. CI sets an event baseline,
uses the Python REST adapter and supplies its optional secret only during execution.
The review remains non-blocking for FAIL and ERROR; both stay visible.
