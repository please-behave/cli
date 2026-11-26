# Please Behave

[![build](https://github.com/please-behave/cli/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/please-behave/cli/actions/workflows/ci.yml) [![release](https://img.shields.io/github/v/release/please-behave/cli?display_name=tag&sort=semver)](https://github.com/please-behave/cli/releases) [![license](https://img.shields.io/badge/license-PolyForm%20Noncommercial%201.0.0-3949ab.svg)](https://polyformproject.org/licenses/noncommercial/1.0.0/)

[![tests](https://christianpflugradt.github.io/badges-and-stuff/please-behave-cli/tests.svg)](https://christianpflugradt.github.io/badges-and-stuff/) [![coverage](https://christianpflugradt.github.io/badges-and-stuff/please-behave-cli/coverage.svg)](https://christianpflugradt.github.io/badges-and-stuff/) [![LOC](https://christianpflugradt.github.io/badges-and-stuff/please-behave-cli/loc.svg)](https://christianpflugradt.github.io/badges-and-stuff/)

Enforce development rules across coding agents and workflows.

> Please Behave is in early development and not ready for production use.
> Configuration, schemas and protocols can change incompatibly without migration.
> API contracts are experimental and do not yet promise compatibility.

Please Behave is a source-available governance CLI for Git projects. Define
development expectations as version-controlled rules alongside your code,
then evaluate them regardless of whether a person or a coding agent made
the changes.

Use deterministic checks for tests, formatting and other conditions that
tools can verify. Add AI reviews for questions that require interpretation,
such as whether an implementation meets written requirements.

## How it works

Rules live in `.please/`. Each rule is either:

- A **check** that runs a command and determines its result from the exit status.
- A **review** that sends written criteria and project evidence to a configured
  AI adapter.

Checks run first. Reviews run only after the check phase succeeds.
Checks always block on failure; reviews can be blocking or advisory.
Results and detailed output are saved locally for inspection.

Rule sets group rules, and execution points select when they run, such as
before a commit, before a push or explicitly in CI.

Please Behave evaluates development results. It does not implement features
or repair code. AI judgments can be wrong; a review PASS is not proof of
correctness.

## Getting started

### Install a release

Download the archive for your platform and `SHA256SUMS` from
[GitHub Releases](https://github.com/please-behave/cli/releases).
Native downloads are available for macOS Apple Silicon and Intel, and
Linux x64 and ARM64 with glibc. Windows builds are not provided.

Calculate the archive's SHA-256 checksum using `shasum -a 256` on macOS or
`sha256sum` on Linux and compare it with the matching entry in `SHA256SUMS`.
Extract the archive and place its `please` executable in a directory on PATH.

```sh
please --version
```

Downloads are not signed or notarized. Linux binaries are built on Ubuntu
24.04; compatibility with older glibc systems is not guaranteed.

The executable needs Git and the tools your rules invoke. It needs no Rust,
Python or Node runtime of its own. AI reviews additionally require an adapter
and its adapter profile setup; no adapter profile is bundled or selected implicitly.

### Initialize your project

Run these commands from the root of your application's Git working tree:

```sh
please init
please sync
please run
please status
```

`init` requires `.please/` to be absent. It creates a passing example check,
a disabled failing check and a disabled review. The initial run works
without an AI provider, but does not yet verify your application's quality.

`sync` validates configuration and asks before applying required Git hook
changes. It executes no checks or reviews.

### Add a useful check

For example, create `.please/checks/readme/rule.toml` to require a nonempty
README:

```toml
id = "repository-readme"
description = "Require a nonempty README"
run_at = ["pre-push", "check"]

[check]
command = "test -s README.md"
```

Replace or remove the passing starter check when you add your own rules.
Then validate and run:

```sh
please sync
please run
please status --details
```

Check commands run without an implicit shell. Their tools must already be
available. Commit your configuration and supporting files so others can
run the same rules.

## Everyday use

| Command | Purpose |
| --- | --- |
| `please init` | Create the initial `.please/` configuration once. |
| `please sync` | Validate configuration and synchronize declared integrations. |
| `please run` | Run selected rules and save their results. |
| `please status` | Inspect the newest report, or one chosen with `--report <number>`, and its freshness. |
| `please publish --gateway-url <URL>` | Send an attestation from the newest current report to the specified gateway. |
| `please register --gateway-url <URL>` | Register committed rule versions, then Rule Set versions, then any configured deployment targets. |
| `please record release --gateway-url <URL> --version <version> [--commit <sha>]` | Record a release fact. The commit defaults to HEAD. |
| `please help` | Show commands and options. |

Gateway commands use `[publish].auth` in `.please/config.toml`.
Set it to `"github-oidc"` in GitHub Actions with `id-token: write` to use
the workflow's identity. GitHub OIDC lowercases the repository host and path
including a `publish.repository` override. The resulting
identity must match the workflow's repository. API-key authentication remains
the default and preserves repository path case.

`run` uses the `pre-push` execution point by default. Preview its selection
without executing rules:

```sh
please run --at pre-push --plan
```

For local checks without AI calls:

```sh
please run --checks-only
```

A checks-only PASS says nothing about review criteria.
`please status --details` shows saved findings and captured output without
running the rules again.

Git hooks support `pre-commit`, `commit-msg` and `pre-push`. Assign rules to
these points and run `please sync` to install the required hooks.
Local hooks are bypassable and evaluate working files, which can differ
from staged or pushed commits.

For GitHub Actions integration, see
[please-behave/action](https://github.com/please-behave/action).

## Documentation

The [published documentation](https://github.com/please-behave/docs) covers
configuration, running rules, Git hooks, adapters, external Rule Sets and the
examples. The [CLI chapter](https://github.com/please-behave/docs/blob/main/src/cli.md)
is the starting point.

## Build locally

Building from source requires Git, Rust/Cargo and a native compiler/linker.
From a source checkout:

```sh
cargo build --release --locked
./target/release/please --help
```

The resulting executable is `target/release/please`.

## Development

Coding agents follow [AGENTS.md](AGENTS.md).
[Why](why/README.md) defines the product's binding behavior,
responsibilities and decisions.

Development uses the tools pinned in `mise.toml`. With
[mise](https://mise.jdx.dev/) available:

```sh
mise trust
mise install
mise run setup
mise exec -- please sync --yes
```

Git and a native compiler/linker are host prerequisites.
Run Git through `mise exec -- git …` so hooks inherit the configured tools.
Commit and push hooks provide final validation.

Repository governance uses the separately installed published release pinned
in `mise.toml`. Builds from this checkout are candidates under test and must
not replace that governor.

## License

Please Behave is licensed under
[PolyForm Noncommercial License 1.0.0](LICENSE).
