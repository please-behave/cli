# Executable adapters

An adapter connects Please Behave reviews to an AI CLI, HTTP API or local model.
It owns authentication, model settings, permissions and provider communication.
Configure it as a [provider](configuration.md#configure-the-ai-provider); select it in a review
or use an explicit project default.

The binding [Adapter Protocol](../knowledge/contracts/adapter/api.yaml) is defined
in Knowledge:

- [Invocation](../knowledge/contracts/adapter/invocation.md): process, streams,
  exit status, deadlines and trust.
- [Request](../knowledge/contracts/adapter/request.md) and its
  [JSON schema](../knowledge/contracts/adapter/request.schema.json): review input
  and adapter settings.
- [Response](../knowledge/contracts/adapter/response.md) and its
  [JSON schema](../knowledge/contracts/adapter/response.schema.json): verdict
  evidence, local validation and optional telemetry.

The [dummy script](../examples/adapters/dummy.py) and accompanying
[TOML configuration](../examples/adapters/dummy.toml) provide a starting point.
Copy them as instructed in the TOML file and select the named provider from a
review. The dummy deliberately exits with status 2 because it performs no
assessment. Replace its result generation with provider communication that
satisfies the contracts above.
