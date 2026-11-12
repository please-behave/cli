# External Git Rule Sets

An import selects one Rule Set directory at one Git revision. The set references
Rule directories anywhere inside that repository; multiple sets can share Rules. The [configuration reference](configuration.md)
lists consumer fields, Rule definitions and execution semantics.

## A shared repository

```text
governance-rules/
  repository/config.toml
  releasable-software/config.toml
  rules/checks/readme/rule.toml
  rules/checks/readme/index.sh
  rules/checks/changelog/rule.toml
  rules/checks/changelog/index.sh
```

`repository/config.toml`:

```toml
id = "repository"
description = "General repository governance"
run_at = ["pre-push", "check"]
rules = ["rules/checks/readme"]
```

`rules/checks/readme/rule.toml`:

```toml
id = "repository-readme"
description = "Require the consumer README"
[check]
command = "sh @rule/index.sh"
```

`rules/checks/readme/index.sh`:

```sh
test -s README.md
```

`releasable-software/config.toml`:

```toml
id = "releasable-software"
description = "Release documentation"
run_at = ["pre-push", "release"]
rules = ["rules/checks/readme", "rules/checks/changelog"]
```

`rules/checks/changelog/rule.toml`:

```toml
id = "release-changelog"
description = "Require the consumer changelog"
[check]
command = "sh @rule/index.sh"
```

`rules/checks/changelog/index.sh`:

```sh
test -s CHANGELOG.md
```

The consumer declares custom points and imports these directories:

```toml
project = "My_App"
[execution_points]
custom = ["release"]

[[imports]]
git = "https://github.com/company/governance-rules.git"
path = "repository"
ref = "v1.2.0"

[[imports]]
git = "https://github.com/company/governance-rules.git"
path = "releasable-software"
ref = "v1.2.0"
```

Each selected directory defines exactly one Rule Set in `config.toml`.
External references also support per-reference overrides:

```toml
rules = [
  "rules/checks/readme",
  { id = "rules/checks/changelog", run_at = ["check"] },
]
```

The table's `id` is the same repository-relative directory path as a string.
Omitting `run_at` inherits the set list; specifying it replaces that list with a
nonempty list of known points. This applies equally to checks and reviews.
Effective reference assignments resolve before consumer import mappings, so
overrides use source-scope point names. Repeated paths still load once and their
effective assignments union without changing rule identity.

External `rules` entries are directory paths relative to the repository root,
not IDs or paths relative to the selected set. A Rule directory must be directly
under `checks/` or `reviews/` and contain `rule.toml` plus its resources. In this
example both sets share `rules/checks/readme` without duplicating it.
Only explicitly referenced Rules are loaded. Unreferenced definitions and other
sets do not affect the import. Repeated paths load once; distinct directories
with duplicate Rule IDs in one import fail validation. All selected definitions
are validated, including disabled Rules. The loader resolves references to IDs
before the ordinary assignment resolver runs. Local sets still reference IDs.

Each import remains an independent execution and parameter binding, including
when two sets share a Rule. A set may also assign one Rule to multiple points
through `run_at`. Removing an import stops using that set. Sets accept no project
settings, nested imports or references to other sets. For existing external sets,
replace directory-local IDs in `rules` with repository-relative Rule paths.

Per-import `[imports.execution_points]` maps external custom names to custom names
already declared by the consumer. The table belongs only to the preceding import.
Resolve inheritance first, map effective assignments once, then deduplicate and
namespace Rules. Built-ins (`pre-commit`, `commit-msg`, `pre-push`, `check`) cannot
appear on either side. Unknown sources/targets and invalid names are errors.
Unmapped names stay unchanged and must be consumer-known. Separate imports may
map the same source differently; multiple sources may share one target. Mapping
never edits source/cache files or changes import identity, and works offline.
See the [complete three-import example](configuration.md#import-rule-sets-from-git).

Each import's `rule_parameters.<rule-id>` tables supply only that binding's
values. External declarations omit `value`. File parameters refer to existing
files in the consumer; bundled assets still belong to the source rule. Repeated
imports execute separately and retain distinguishable IDs in plans and reports;
repeated references within one binding execute once. See
[parameter declarations and delivery](configuration.md#rule-parameters).

## Named provider configurations

A selected directory may distribute complete named configurations in
`providers/<id>.toml` beside its `config.toml`. They use the same adapter fields,
defaults and limits as [local provider configurations](configuration.md#configure-the-ai-provider).
All provider files in that directory are validated, including unused definitions.
A review selects one with `provider = "quality"` in its `[review]` table.

Provider IDs are global across local files and all explicitly imported sets.
Cross-import references resolve after loading all imports, independently of order.
A distinct definition with an existing ID fails, even with identical settings;
consumer overrides are unsupported. Repeated imports of the same immutable file
share its configuration. A provider-only set may use `rules = []`, with the usual
required `id`, `description` and nonempty `run_at`. No extra imports are inferred
or downloaded. Reviews without references keep the consumer's explicit default adapter.

Provider files follow the existing source-boundary, integrity and atomic state
publication checks. All configurations identify executable adapters; runtime
requirements may name environment variables and executables. No credential values
belong in provider files. Please Behave ships no provider implementation.

## Refresh, storage and provenance

Host Git fetches exact selectors into shared bare object storage and resolves them
to commits before loading definitions. Tags are peeled to commits; a full hash
must identify that exact commit. Failed selectors never fall back to another ref.
Already available objects are reused. A cached full commit may be available
without remote contact.

Default cache roots are:

- macOS: `$HOME/Library/Caches/please`.
- Linux: `$HOME/.cache/please`.
- An absolute `XDG_CACHE_HOME` overrides these, using `XDG_CACHE_HOME/please`.

The layout is `git/REPOSITORY_DIGEST/COMMIT/contents`, with an integrity receipt
beside contents. Imports at the same commit share this checkout, including imports
through different refs. Repository selectors are hashed rather than persisted in
cache metadata. Equivalent URL aliases are separate identities. HTTP credentials
in URLs and URL queries/fragments are rejected. Please Behave issues no authentication
prompts; configure host Git credentials or SSH access.

Project resolution state lives at `please/external-rule-sets.json` inside the
project's Git directory (per worktree). It records repository digest, configured
ref, selected directory and resolved commit. It stays outside tracked files and
working-tree snapshots. Refresh atomically replaces complete state after all
local and imported rules, provider definitions and references validate. Removed references are dropped. Cached
revisions remain available; no automatic eviction is performed.

OS locks serialize writers, including different projects sharing a cache. Locks
release on process exit. Readers check integrity receipts, file kinds and
executable flags before use; partially populated revisions are never published.
Receipts detect accidental corruption, not an attacker who can rewrite both
contents and receipts. State and reports are local, unsigned evidence.

Imported IDs have the form `git:BINDING_DIGEST:DIRECTORY:OCCURRENCE:ORIGINAL_ID`.
The binding digest includes the repository selector, ref, normalized directory
and per-import parameter values. Repeated identical bindings have separate
one-based occurrence numbers. Execution-point mappings do not enter the digest.
Definition resolution and cached checkouts remain shared across bindings. Local IDs are
unchanged. Plan/report origins retain the configured ref and full resolved commit
without internal cache paths. Saved input freshness compares the consuming working
tree; inspect provenance to see which external revision a report evaluated.

## Source resources and trust

Imported Checks use the same string commands as local Checks. `@rule/` resolves from the declaring
Rule directory; `@source/` resolves from the imported Git repository root, so a
shared script can use `command = "python3 @source/scripts/check-files.py"`.
`@repo/` addresses consuming repository files. Ordinary command paths still use
the consuming root. These references add no shell expansion.

Keep imports pinned to revisions compatible with the consuming CLI. Refreshing a
moving branch can load incompatible rules.

Checks keep the consuming project's working directory. `@rule/` references
locate bundled resources inside the declaring Rule directory, even for shared
scripts, and `@param/<name>` and `@context` pass parameters and the run context. Review prompts load from their
own Rule directories; context paths and evidence refer to the consumer.

Selected set directories, Rule references, definitions and prompts
reject symlinks, parent traversal and unexpected file types. The external
repository is the containment boundary for Rule references and shared scripts:
`@rule/index.sh` resolves in the declaring Rule directory and `@source/scripts/check-files.py`
from the imported repository root, independently of the selected set directory.
Resolution never searches other locations and works offline from the cache.
Each Rule executes independently with its own parameters and result, even when
several Rules reference the same script. Review prompts remain confined to their
own Rule directory. Only designated definitions are interpreted; supporting TOML is data. Git retrieval disables downloaded hooks
and initialization scripts and never initializes submodules. Host credential
helpers and SSH configuration remain trusted integrations.

Checks execute with the Please Behave user's permissions. Resource confinement does not
prevent scripts from accessing other paths, credentials or the network. Import
executable Rules only from trusted sources. Imported generic CLI provider commands
use this same trust boundary and execute from the consumer root with its inherited
environment. They may reference installed programs or consumer-provided wrappers;
Legacy Rule Set-provider command arguments stay literal; independent provider
packages below add explicit cached-resource references. Review every
imported provider definition before approving source refresh. There is no process sandbox.

## Independent provider packages

`[[provider_imports]]` selects a named provider independently of any Rule Set,
using `id`, `git`, `path` and `ref`. The selected directory contains `provider.toml`
and adapter resources. Rulesets and providers may come from separate repositories;
sharing a source revision still reuses the same immutable checkout. No empty
provider-only Rule Set or execution point is required. Existing Rule Set-bundled
providers above remain compatible. See the
[provider import reference](configuration.md#import-providers-from-git) for a
complete consumer and package example.

No provider files are copied into the consumer. Imported command/args and
executable requirements can use `@provider/` for the selected directory,
`@source/` for its repository and `@repo/` for the consumer. Path confinement,
symlink rejection and cache receipts apply. `@@` escapes a literal @. Plain
arguments keep their existing consumer-root semantics. These references are
specific to independent packages; existing local/Rule Set-provider arguments
retain their literal behavior. This makes adapter distribution explicit without
adding provider-specific code to Please Behave.

Both import kinds publish one resolution state atomically. Provider IDs must be
unique across local files, Rule Set exports and independent packages. Repeated
IDs in independent imports fail even when selected content is identical. References
resolve after loading all declared sources. Plans/reports retain provider origin
metadata and stable raw reference tokens; resolved machine-specific cache paths
are runtime details. An unavailable/corrupt imported provider blocks execution,
even for an advisory review. Adapter authentication and tools are still checked
only when a selected review reaches its provider call.

## Recovery and limits

Missing or changed resolution state, missing cached revisions or integrity failures
require `please sync`. A failed refresh preserves the last valid state. For
corrupt immutable entries, remove the affected repository-digest cache directory
and refresh; Please Behave never silently replaces validated revisions. Authentication
comes from host Git/SSH; Git stderr is suppressed to avoid credential disclosure.

Fetches have a 60-second deadline; local Git operations and lock waits have
30-second deadlines. Cached repositories allow 8 MiB/file, 128 MiB total,
100,000 entries and 128 directory levels. Metadata is limited to 64 KiB/file;
the combined configuration allows 256 Rules, 256 Rule Sets and 256 named providers. Resolution state
is limited to 64 KiB; refs to 1024 bytes. Shared-cache locking currently uses Unix
OS locks; external imports on Windows remain unsupported.
