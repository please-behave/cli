# Executable adapter protocol

Please Behave supplies no AI provider. An adapter is a trusted executable that
translates this common request into provider communication and translates the
response back. Adapters can call a CLI, an HTTP API or a local model. No Please
rebuild is required. Adapters own authentication, model settings, permissions,
retry behavior and provider-specific response extraction.

## Invocation and versioning

Configure command, literal args and timeout_seconds in `.please/provider.toml`
(the explicit project default) or `.please/providers/<id>.toml` (selected using
`review.provider`). Commands resolve through PATH or a path relative to the
consumer root; absolute executable paths are supported. There is no shell or
argument expansion. The working directory is the consumer root. Adapters may
live under `.please/providers/adapters/`, elsewhere in the project or outside it.
The process inherits the environment without any `PLEASE_*` variable. The execution
point, resolved Rule parameters (`rule_parameters`) and the run context file path
(`context_file`) arrive in the request JSON.

Independent provider packages can be selected through `[[provider_imports]]`.
Their `provider.toml` uses this same adapter contract. Explicit `@provider/`,
`@source/` and `@repo/` tokens in command/args/executable requirements resolve to
verified resources; local and legacy Rule Set-provider arguments remain literal.
Distribution does not change the working directory, environment, protocol or
provider requirements. See [provider imports](configuration.md#import-providers-from-git).

Each invocation receives exactly one UTF-8 JSON object on stdin followed by EOF.
Write exactly one JSON object to stdout and exit zero. All requested reviews must
be represented exactly once. Stderr is captured within the output bounds of checks.
When a call ends as ERROR, its stdout and stderr are retained unmasked and shown by
`please status --details`, so write the original error text to stderr and never
include credentials. Never write progress text to stdout.

Both directions require integer `protocol_version: 1`. Unsupported versions fail
closed; no legacy format detection or fallback occurs. Any incompatible protocol
revision must use a new version supported explicitly by Please. Adapters should
reject unsupported request versions and may ignore unknown request fields.
Please rejects unknown response fields. The verdict schema is sent with each
request so adapters need not duplicate limits or hardcode current Rule IDs.

The overall deadline includes stdin delivery; zero disables it. Timeouts terminate
the process tree. A launch failure, nonzero exit, invalid JSON, unsupported version,
truncated output or invalid verdict produces ERROR for the entire batch. Stdout
is bounded to 64 KiB. The output of a call that ends as ERROR is saved with its report entry. An adapter
reports ERROR only for technical failures it detects itself, such as an
unreachable provider, using the common result format. Blocking and nonblocking reviews follow the same
ordinary enforcement rules regardless of which adapter is selected.

## Request

```json
{
  "protocol_version": 1,
  "execution_point": "check",
  "head": null,
  "input": {
    "excerpts": [], "focused_paths": null, "manifest": ["README.md"],
    "diff": ""
  },
  "context_file": "/path/to/context.json",
  "rule_parameters": {},
  "reviews": [{
    "id": "documentation", "criteria": "Inspect documented expectations.",
    "entry_files": ["README.md"], "context_files": [],
    "parameters": {"variant": "careful", "tools": {"read": true}}
  }],
  "prompt": "The complete common review instructions and prepared evidence...",
  "response_schema": {"type": "object", "required": ["reviews"]}
}
```

The actual response_schema includes all verdict constraints, findings and requested
IDs; the abbreviated schema above illustrates its position. It describes the
`reviews` document, without the adapter envelope's protocol_version or usage.
`head` is the current commit or null for an unborn repository. Excerpts contain
path, start_line, end_line and text with original line numbers. Manifest paths
and diff are orientation hints, not exhaustive evidence. `focused_paths`
is null for project-wide orientation or the selected entry/context paths.
The prompt contains the common review instructions, criteria, excerpts and
orientation. An adapter can use that prompt directly or construct a request from
the structured fields, preserving the review and evidence requirements.

No tools, repository sandbox or safety instructions are supplied by Please;
protecting the project from an adapter's actions is the responsibility of the
adapter and its provider. A conclusion that cannot be established as PASS,
including one resting on insufficient or uncertain evidence, is FAIL; ERROR is
reserved for technical failures. Please cannot enforce this distinction; it is
part of the contract. PASS and FAIL require grounded evidence; Please validates paths and line numbers locally and
checks snapshot integrity before and after execution.

## Parameters

Provider `[parameters]` and Rule `[review.parameters]` are arbitrary tables of
JSON-compatible TOML values. Each review overrides matching top-level provider
keys. A nested table or array replaces the entire value at that key; there is no
deep merge. Other provider keys are retained. Please does not interpret settings
such as a model, reasoning effort, tool permissions or MCP configuration. The
adapter validates its supported settings. Do not store credentials in these
settings: configuration is included in plans/reports and version control.
Typed Rule `[parameters.<name>]` declarations and their environment variables
remain separate from adapter parameters.

```toml
# .please/providers/quality.toml
command = "review-adapter"
timeout_seconds = 180
[parameters]
variant = "quick"
[parameters.tools]
read = true
search = true
```

```toml
# Within a review Rule
[review]
provider = "quality"
prompt = "prompt.md"
[review.parameters]
variant = "careful"
[review.parameters.tools]
read = false
```

That review receives `{"variant":"careful","tools":{"read":false}}`.
The replaced tools value contains no search key. Reviews sharing a provider
reference, Rule parameter environment and diff selection share one invocation;
each has independent merged adapter settings.

## Response

```json
{
  "protocol_version": 1,
  "reviews": [{
    "id": "documentation", "status": "PASS", "reason": "Documented behavior matches",
    "findings": [{"message": "Compared the documented command", "path": "README.md", "line": 1}]
  }],
  "usage": {"input_tokens": 100, "cached_input_tokens": 20, "output_tokens": 30,
            "reasoning_output_tokens": null, "cache_write_input_tokens": null}
}
```

Statuses are exactly PASS, FAIL and ERROR. Reasons are nonempty single lines of
at most 120 characters. Findings contain message (1–2000 UTF-8 bytes), path and
line, with at most 20 findings per review. Path and line must be present, although
both can be null. For worktree evidence, a line requires a path. Worktree
PASS/FAIL need at least one path in the verified manifest and a positive line within that file. ERROR may have no findings.
Wrong types, extra fields, missing/duplicate/extra review IDs and invalid evidence
invalidate the whole batch. Provider-specific envelopes and Markdown fences are
not accepted; adapters must translate them.

Commit-header reviews use the same versioned envelope, with a source-specific
response schema. Each review has `input: "commit-header"`; `input.commit_header`
is the captured first line as a JSON string. The remaining input fields contain
empty excerpts, manifest, focused paths and diff;
`head` is null. The prompt contains the configured criteria and header, without
repository orientation. Adapters must not add repository or commit-body context.
Worktree and header reviews never share a batch. Worktree review metadata uses
`input: "worktree"` and retains its existing evidence contract.

For header PASS/FAIL, provide at least one finding like:

```json
{"source":"commit-header","message":"The summary names a concrete action in the imperative.","path":null,"line":1}
```

The header generation schema requires at least one finding, including for PASS.
Adapter-generated technical ERROR responses may still omit findings.

`source` is an optional addition to the response wire type, required only for
header evidence; file findings continue to omit it. Header reviews reject file
findings, absent header evidence and any line except 1. Worktree reviews reject
header findings. ERROR may have no findings. The CLI captures and revalidates the
message independently; the adapter receives no raw Git hook arguments or path.
Older adapters may continue serving worktree reviews unchanged, but must support
the supplied header schema before they can serve this new input kind.

Usage is optional or null and never changes the verdict. It may contain input_tokens,
cached_input_tokens, output_tokens, reasoning_output_tokens and cache_write_input_tokens.
Only positive integers count as reported; zero, negative, null, missing or otherwise
invalid values count as not reported. Cached, cache-write and reasoning counters are
subsets and are never added twice, and no consistency between counters is validated.
A summed counter is available only when every adapter call of a run reported it;
otherwise it stays unavailable, never zero. Reported usage is retained even if verdict
evidence is invalid.
These counters are adapter claims, not billing or quota statements.

## Minimal adapter and migration

The [dummy script](../examples/adapters/dummy.py) and accompanying
[TOML configuration](../examples/adapters/dummy.toml) form a working example.
Copy them as instructed in the TOML file and select the named provider from a
review. The dummy returns ERROR deliberately: it performs no assessment, so
its review technically cannot take place. Replace its result generation with your own
provider communication while preserving the protocol.

This migration removes the built-in Codex implementation completely. There is no
bundled replacement or implicit provider. Initialization writes no provider file.
Existing `provider = "codex-cli"` and `provider = "cli"` definitions must be
replaced by command/args adapter definitions. Move model, review_profile and
mcp_servers settings into parameters understood by your adapter. PLEASE_MODEL is
no longer set or cleared by the core. Existing generic CLI wrappers must parse the
JSON request and emit the versioned response rather than receiving a plain prompt
and returning a bare verdict. Named selection and explicit project defaults remain.
Old saved provider metadata may be incompatible; rerun verification after migration.
The repository's installed governor and its private `.please/` adapters remain
independent of this product contract.
