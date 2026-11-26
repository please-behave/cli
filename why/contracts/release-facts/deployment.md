# Deployment fact command

Run `please record deployment --gateway-url <URL> --target <name> --repository <identity> (--version <version> | --commit <sha>)`
from the root of the reporting repository's Git working tree. The command sends
a deployment fact independently of verification, saved reports and local Git
history.

Supply the case-sensitive registered target name and the deployed software's
repository as a normalized host/path identity or a network Git URL. The CLI
normalizes URLs using the same rules as publishing. The normalized subject
identity must contain 1 to 256 characters. A version must contain 1 to 256
characters without NUL. A commit must contain exactly 40 or 64 ASCII hexadecimal
characters. The command sends the selected version or commit as supplied.

The command uses the shared [gateway origin and project settings](../configuration/project.md#publish-settings).
The authenticated sender owns the target. The CLI validates the shared
`publish.repository` setting when present and always takes the subject from
`--repository`.

It sends one bearer-authenticated POST with `target`, `repository` and exactly
one of `version` or `commit` to `/v0/facts/deployments` under the
[public gateway contract](https://github.com/please-behave/gateway/blob/main/why/contracts/ingest/deployment-fact.md).
Version 0 is experimental. The gateway path declares the wire version.

| Gateway status | Receipt | Diagnostic | Exit status |
| --- | --- | --- | --- |
| 201 | `{"outcome":"created"}` | Deployment recorded. | 0 |
| 200 | `{"outcome":"unchanged"}` | Deployment unchanged. | 0 |
| 400 | Empty | Deployment target is unknown to the gateway for the authenticated sender. | 2 |

The CLI validates the request locally against the gateway schema before treating
an empty HTTP 400 response as an unknown target. Receipts are limited to 1 KiB.
Invalid arguments, missing configuration or credentials, invalid receipts,
gateway rejection and transport failures exit 2. Gateway rejection and transport
failure have distinct diagnostics. The command follows no redirects and
diagnostics omit credentials, response bodies and URL details beyond the origin.
