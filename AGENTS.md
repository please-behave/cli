# Working on Please Behave

## Principles

Consistency and avoiding AI slop are the highest quality priorities.

- Follow established decisions, terminology and conventions across code, CLI,
  configuration, tests and documentation. Update all affected places together.
- Implement only requested behavior. Do not invent features, aliases,
  requirements or abstractions. Resolve uncertainty by inspecting existing
  decisions or asking the user.
- Prefer the smallest coherent solution. Avoid unnecessary dependencies,
  complexity, boilerplate, duplication and unsupported claims.
- Keep documentation concise, accurate and useful. Explain concepts before
  using them, provide working examples and distinguish implemented behavior
  from future possibilities.
- Before finishing, review the complete change for contradictions,
  inconsistencies, stale references and unnecessary additions.

## Product and architecture

- Read `docs/product-vision.md` and `SPECIFICATION.md` before changing behavior
  or architecture. The vision defines direction; the specification defines
  supported behavior. Explicit user decisions take precedence.
- Develop from the specification: define behavior and acceptance criteria before
  implementation. Keep specification, implementation, tests and documentation
  consistent.
- Keep Please Behave small and agent-agnostic. Do not introduce speculative
  infrastructure, services or autonomous repair loops.
- The product executable is `please`; preserve `init`, `sync`, `run`, `status`
  and `help`, English user-facing output and lowercase `.please/` configuration.
  Source code, configuration and documentation remain in English.
- Run deterministic checks before LLM reviews. Never treat provider failures,
  invalid JSON or incomplete snapshot coverage as PASS.

## Code discovery

- Prefer `codebase-memory-mcp` when available. Verify the project and index,
  then use task-directed graph searches and source snippets.
- Check index coverage before relying on graph evidence. Inspect source
  directly when coverage is incomplete or tools are unavailable.
- Treat graph results as supporting evidence, not proof of completeness.

## Development governor

This repository is governed by the separately installed, released Please Behave
CLI pinned in `.please-version`. Keep its `.please/` configuration, wrappers,
Git hooks and governance workflows operational. Never use candidate binaries
from this checkout as the governor.

## Validation — Git hooks are authoritative

- Install the repository's own Git hooks using
  `sh scripts/setup-git-hooks.sh` when needed. This installation is explicitly
  authorized; do not install unrelated hooks.
- **Do not manually run tests, linters, builds or validation commands.**
  Validation runs through the installed Git hooks:
  - `pre-commit` validates formatting and documentation.
  - `pre-push` runs the complete local validation suite.
- Read hook failures and fix their reported causes. Use
  `please status --details` to inspect saved reports without rerunning
  checks. Retry the Git operation after fixing the problem.
- Never bypass hooks or replace their validation with manual checks.
- Formatting commands that modify source are permitted.
- Tests must not invoke live Codex reviews, network services or paid APIs.
  Dependency downloads during explicitly authorized preparation are permitted.
- Keep dependencies lean and `Cargo.lock` current.

## Git workflow

- Work on `main`. After each coherent change, commit and push automatically
  without requesting additional approval, including documentation-only changes.
- Follow `CONTRIBUTING.md` and `.githooks/commit-policy` for commit conventions.
  Follow `docs/releasing.md` for release policy.
- Stage only files belonging to the change. Preserve unrelated user edits.
  Never force-push or rewrite another person's work.
- If a hook fails, fix the reported problem and retry. Amend only your own
  unpublished commit when appropriate.
- If validation, commit or push cannot complete, retain the changes and report
  the failure. Never claim an unpushed change was pushed.
- Do not tag, publish releases or modify unrelated remote services without
  explicit authorization.

## Remote workflows

- Never monitor, poll or wait for GitHub Actions or other remote workflows
  unless explicitly requested.
- If further work depends on a workflow completing, ask the user to provide
  its result before continuing.
- Committing or pushing does not authorize workflow monitoring.

## Documentation and repository hygiene

- Keep README complete for supported CLI commands, configuration options,
  defaults and limitations. Put implementation details in `docs/`.
- Keep license metadata consistent with `LICENSE`.
- Never commit credentials, machine-specific paths, local reports or
  developer-specific configuration.
- Use generic placeholders in public examples rather than private project names.
