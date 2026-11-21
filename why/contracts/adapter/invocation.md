# Adapter invocation

Producer: Please Behave. Consumer: the configured external adapter. An invocation
contains exactly one review. Its JSON messages are defined by the
[request](request.md) and [response](response.md) contracts; all three contracts
share the compatibility version in [api.yaml](api.yaml).

## Process boundary

Please invokes the configured executable and literal arguments without a shell
or argument expansion. The working directory is the consuming repository root.
An executable name is resolved through PATH; relative executable paths use that
root and absolute executable paths are supported. Independently imported adapter profile
resource tokens are resolved before invocation according to the adapter profile.
Local and Rule Set-bundled adapter profile arguments remain literal.

The child inherits the environment after removal of inherited `PLEASE_*` variables.
Execution point, resolved Rule parameters and the run-context path are request
data, not additional command arguments or exported variables. Adapter profile executable
and environment prerequisites are checked after deterministic checks, immediately
before the selected adapter is called. A missing requirement is an ordinary review
ERROR, not a configuration error.

Please writes one UTF-8 JSON request to stdin, closes stdin and reads the result.
At exit 0 or 1, stdout must contain exactly one JSON response object, with no
Markdown fences, AI provider envelope or progress text. An adapter reports its own
technical failure using another exit status and diagnostic text on stderr.

| Exit status | Result |
| --- | --- |
| 0 | PASS, only with a valid response and locally valid evidence. |
| 1 | FAIL, only with a valid response and locally valid evidence. |
| Any other status | ERROR; no structured response is required. |

Failure to start, timeout, signal termination, missing or invalid output and
exceeded output limits are ERROR. No retry or alternate-version call is made
by Please. Adapters control their own AI provider communication and retry behavior.
Review enforcement remains separate: an advisory review ERROR stays visible and
can permit overall success; configuration and integrity errors remain fatal.

## Bounds and diagnostics

The configured adapter deadline covers the whole invocation, including request
delivery; zero disables the deadline. On timeout Please terminates the process
tree. Stdout and stderr each retain at most 64 KiB, with total byte counts and
truncation flags. Truncated stdout cannot establish a verdict. An ERROR retains
available output unmasked for `please status --details`; adapters must not print
credentials. Routine output is not a substitute for the structured response.

## Trust

Adapters have the invoking user's permissions. Please supplies no sandbox,
AI provider tools, safety instructions or privileged system prompt. Adapters own
authentication, model settings, permissions and AI provider response extraction.
They may inspect further repository files; initial evidence is not an access
boundary. Protecting the project from adapter actions belongs to the adapter
and the AI provider it calls. Input stability is checked locally around execution.
