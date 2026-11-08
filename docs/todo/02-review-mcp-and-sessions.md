# TODO 2: Per-review MCP permissions and sessions

Status: superseded by [DEC-0010](../../knowledge/decisions/DEC-0010.yaml) and the
[adapter protocol](../adapter-protocol.md).

Model settings, MCP permissions, tool access and provider sessions belong to
external adapters. Provider definitions and individual reviews can pass structured
parameters without core interpretation. Please Behave supplies no provider or
provider session implementation. Any sharing inside an adapter must preserve
review isolation, requested coverage and the common result contract.
