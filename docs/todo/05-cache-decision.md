# TODO 5: Measure and decide result caching

Status: TODO; decision task, not authorization to build a cache unconditionally.
Depends on [TODOs 1–4](README.md).

## Goal and decisions

Use measured repeated work to decide whether result caching adds enough value to
justify invalidation complexity. Provider-side prompt caching and reusing a saved
Please Behave verdict are different mechanisms; reported cached-input tokens do not mean
Please Behave reused a result or that an entire call was free.

- Start with the current shared prepared context. Keep
  deterministic prompt structure and stable ordering where compatible with the
  review; never omit necessary changing evidence to improve cache hits.
- No paid benchmark calls without explicit authorization. First use existing
  local telemetry and synthetic fixtures; insufficient data means defer caching.
- Record eligible repeated inputs, actual call latency and available input,
  cached-input and output counters. Do not infer monetary savings from tokens.
- No approximate/semantic result reuse. Any future candidate is opt-in and exact
  input only; never reuse across different criteria, context, model/settings,
  effective MCP rights, scopes, base/target identities or verifier/schema versions.
- External MCP evidence can change without repository changes. Reviews using
  live external evidence are ineligible by default; a TTL alone cannot establish
  equivalent evidence. Resolve this before any such cache support.
- Mutable model aliases, unresolved provider defaults and provider changes also
  prevent claiming identical execution. An initial cache must skip reuse where
  those identities cannot be established.
- Never cache ERROR, NOT_RUN, incomplete or unstable verification as successful
  evidence. An initial implementation, if justified, reuses only complete PASS
  results and labels them visibly as reused with their original timestamp.
- Gate result reuse stays disabled initially. Local reports/caches are neither
  signed nor trusted cross-machine gate attestations. Checks still execute unless
  a separate explicit design justifies deterministic check caching.

## Work

- [ ] Gather a small reproducible measurement summary, stating data limitations.
- [ ] Compare no result cache against exact-input reuse: expected hit rate,
      avoided calls/latency and invalidation/storage complexity.
- [ ] Write a short decision: defer, reject or implement a narrowly defined
      opt-in cache. Include evidence and explicit success criteria.
- [ ] If implementing, first specify complete keys, provenance, local ignored
      storage, atomic bounded writes, expiry, invalidation and explicit bypass.
- [ ] If implementing, test every key change, corrupt entries, external evidence,
      unresolved identities, expiry, bypass and visible reused-result reporting.

## Done when

There is an evidence-backed decision committed to the repository. Deferral clearly
states what measurement would reopen it. Implementation is required only for an
affirmative decision, and must not weaken fresh gate verification or claim fresh
LLM execution for reused results.

Decision / implementation commits: none.
