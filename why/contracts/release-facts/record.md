# Release fact command

Run `please record release --gateway-url <URL> --version <version> [--commit <sha>]`
from the root of the Git working tree. The command reports a fact without
running verification, reading a report or creating a release.

The version must satisfy the gateway's request schema and is sent as supplied,
including whitespace and case. An explicit commit must be 40 or 64 ASCII
hexadecimal characters. It is sent as supplied and need not exist in the local
clone. Otherwise, the command resolves the commit at `HEAD` and requires it to
exist.

The command uses the [gateway origin, repository identity and authentication
rules](../report/document.md#publishing-evidence) shared by `publish` and
`register`. It sends one bearer-authenticated POST with `repository`, `version`
and `commit` to `/v0/facts/releases` under the
[public gateway contract](https://github.com/please-behave/gateway/blob/main/why/contracts/ingest/release-fact.md).
Version 0 is experimental. The gateway path declares the wire version.

| Gateway status | Receipt outcome | Diagnostic | Exit status |
| --- | --- | --- | --- |
| 201 | `created` | Release recorded. | 0 |
| 200 | `unchanged` | Release already known. | 0 |
| 409 | `conflict` | Release version conflicts with a recorded commit. | 2 |

The receipt must be a JSON object of at most 1 KiB with exactly the documented outcome field.
An absent, malformed or inconsistent receipt fails with exit status 2.
Invalid arguments, an unresolved `HEAD` when required, missing configuration
or credentials, gateway rejection and transport failures also exit 2.
Gateway rejections and transport failures have distinct diagnostics. The
command follows no redirects and diagnostics omit credentials, response
bodies and URL details beyond the origin.
