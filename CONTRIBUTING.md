# Contributing

Keep changes focused. For bugs, include your OS, `please --version`, a minimal
configuration and reproduction steps. For features, explain the problem and how
it fits the small CLI described in [SPECIFICATION.md](SPECIFICATION.md).

## Development

See the [README](README.md#build-locally) for tooling. With mise:

```sh
mise trust
mise install
mise run ci:prepare
mise hooks:install
```

Development is governed by the separately installed Please Behave release pinned
in `mise.toml`, using `.please/`. `mise install` installs that exact release.
Build the candidate separately as described in the README; it never becomes the
governor. See [repository quality gates](docs/self-governance.md) for execution
points, prerequisites, CI pinning and Renovate updates. Prepare dependencies
explicitly before running pre-commit and pre-push, which remain offline.

Commit and push to receive validation feedback from the installed Please Behave hooks.
Agents must not manually run comprehensive final validation, including
`mise run check` and `mise docs:check`. Source formatting and targeted diagnosis
are allowed as described in AGENTS.md. Developers
can use `mise run check` for an explicit manual run of Please Behave at pre-push.

Product behavior is tested only through `tests/*.rs`; tests of repository tooling live
next to the scripts (`scripts/test_*.py`, `scripts/test-*.mjs`) and never test the product.
Rust tests start processes only through `support::command`, which removes `PLEASE_*`,
`GIT_*`, `CI` and `GITHUB_ACTIONS` so the governor, hooks and CI cannot leak into them.
Tests must remain offline and credential-free; mock Codex instead of calling it.
Keep source, configuration, documentation and product output in English.
Update relevant docs and focused tests when behavior changes.
Commit Cargo.lock; exclude reports, credentials and local MCP configuration.

## Commits

Use Conventional Commit types: `build`, `chore`, `ci`, `config`, `docs`, `feat`, `fix`,
`perf`, `refactor`, `revert`, `style`, `test`. Other types are rejected.
Use `type(scope): summary`; the scope is optional, but when present
must come from the following list. A breaking-change `!` before the
colon is supported. Write a specific English imperative summary; bodies and
footers are optional. The external repository-conventions Rule Set defines accepted
types; [.please/commit-scopes.txt](.please/commit-scopes.txt) defines local scopes.
Its advisory Gemini Flash-Lite Review evaluates only the header wording, without
the diff. Export GEMINI_API_KEY for the hook; unavailable credentials or provider
errors remain visible without blocking the commit.

| Type | Purpose | Release |
| --- | --- | --- |
| `feat` | New functionality | Minor |
| `fix` | Bug fixes, including hotfixes and security fixes | Patch |
| `perf` | Performance improvements | Patch |
| `chore` | Other maintenance, including routine dependency updates | Patch |
| `build` | Build system, packaging and toolchain | Patch |
| `refactor` | Code restructuring without intended behavior changes | Patch |
| `revert` | Undo an earlier change | Patch for effective reversions |
| `ci` | CI/CD workflows | None |
| `config` | Configuration changes | None |
| `docs` | Documentation | None |
| `style` | Formatting and whitespace only | None |
| `test` | Tests and test infrastructure | None |

Use the most specific type; `chore` is the fallback. Dependency updates normally
use `chore(deps)`; use `fix(deps)` when fixing a known bug or security issue.
Any type with `!` or a `BREAKING CHANGE:` footer triggers a minor release during
the 0.x development series, and a major release after explicit stable graduation.
The initial release is 0.1.0.
Historical `build(release)` version commits do not trigger another release.
Releases no longer create version commits. See [release policy](docs/releasing.md#version-policy)
for combined changes and reverts.

| Scopes | Use for |
| --- | --- |
| `checks`, `reviews`, `profiles` | Rules and rule selection; `profiles` remains an accepted historical commit scope. |
| `cli` | Commands, terminal output and CLI internals |
| `config`, `provider` | Configuration and provider integration |
| `report`, `snapshot` | Results and review input |
| `deps`, `release` | Dependency updates and release tooling |
| `git`, `mise`, `ci` | Git policy, local tasks and workflows |
| `docs`, `tests` | Documentation and test infrastructure |

```sh
sh scripts/setup-git-hooks.sh
# Or: mise run hooks:install
```

The Please Behave governor manages the three hooks through `please sync` and
corresponding execution points. Pre-commit checks formatting and documentation; commit-msg checks the final message before Git
creates or amends a commit; pre-push runs the complete local suite. The shared
message checker and advisory wording Review are imported, with no local duplicate. The setup
shim delegates to `please sync` without changing `core.hooksPath`. Please Behave
preserves compatible unmanaged hook content. Each clone needs setup;
Git's `--no-verify` can bypass a local hook; agents must never bypass validation.
Work on `main`. If a commit hook fails, fix the reported issue and retry the
commit. If pre-push fails, fix the issue, amend your own unpublished commit when
appropriate and retry the push. Preserve unrelated work and never force-push.
Use `please status --details` to inspect captured output without rerunning
checks. Missing tools or broken hooks must be addressed or reported.

Examples: `chore(deps): update dependencies`, `feat(reviews): add scoped context`, `fix(cli): preserve report output`,
`docs: clarify local installation`.

In your PR, describe the problem, resulting behavior and actual validation.
Contributions use the project's [PolyForm Noncommercial License 1.0.0](LICENSE). See
[release instructions](docs/releasing.md) for packaging and publishing.

Native reconciliation tests use temporary repositories and real Git commits/pushes;
they run with the Rust suite at pre-push. Keep tests offline and never
change developer/global Git configuration. The repository integration uses the
published Please Behave release pinned in `mise.toml` with `--yes`.
Do not substitute a candidate built from this checkout.

For release tooling changes, use the Node version in `mise.toml`, prepare npm
dependencies with `npm ci --ignore-scripts`; pre-push runs the release tooling
tests. Commit syntax and wording are evaluated at commit-msg only.
Releases are manually started on main;
Semantic Release handles versions, tags, release notes and four native archives.
