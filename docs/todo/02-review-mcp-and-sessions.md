# TODO 2: Per-review MCP permissions and sessions

Status: TODO. Depends on: [TODO 1](../execution-points-and-plans.md).

## Goal and decisions

Expose only the MCP servers needed by a review while sharing context between
reviews that have identical permissions. Provider configuration remains the
selected configuration's maximum; it references server definitions in Codex configuration,
without duplicating credentials or rewriting the user's settings.

Proposed review metadata:

```toml
mcp_servers = ["project-memory", "browser"]
```

- Omitted or empty means no MCPs. `mcp_servers = ["*"]` explicitly selects all
  provider-allowed servers; `*` cannot be combined with names. It never selects
  every server from the user's personal configuration.
- Named permissions must be a subset of the provider allow list. Reject unknown
  names, duplicates and disallowed names; never expand the selected provider allow list.
- Group scheduled reviews by identical effective MCP set, provider/model/effort
  settings and input identity. Reviews with different permissions never share a
  session. Each group starts a fresh session; no resumed cross-group history.
- Execute groups sequentially, ordered by their smallest review ID; order rules
  within a group by ID. This refines TODO 1's initial review scheduling order.
  Displayed rule results remain sorted by kind and ID.
- Build each group's context only from its own reviews, deduplicating shared
  excerpts. Entry paths remain hints, not filesystem permissions.
- Keep existing child-process restrictions and explicitly disable all servers
  outside a group's set. Fail discovery/configuration before starting a model.
- A review FAIL does not suppress other groups. Malformed output makes that
  group's reviews ERROR; unrelated groups can continue if input and budgets
  remain valid. Input changes stop further calls and prevent overall PASS.

## Work

- [ ] Validate review permissions and resolve wildcard selection offline.
- [ ] Extend plans with stable group IDs, effective permissions and call counts.
- [ ] Execute isolated groups and validate exact result-ID coverage per group.
- [ ] Report progress and actual usage once per group, with rule-to-group links;
      do not sum repeated shared durations or token counters.
- [ ] Update documentation and examples; MCPs remain disabled by default.
- [ ] Add provider-fake tests covering grouping, restrictions and error handling.

## Done when

Provider allows five servers, review A requests two, B requests all five and C
requests none: three isolated sessions expose exactly those sets. A fourth review
with A's settings shares A's call. No group inherits tools or history from another.
Plans need no live discovery; an actual discovery mismatch fails before the model.
Checks-only runs perform zero provider operations regardless of configured MCPs.

Implementation commits: none.
