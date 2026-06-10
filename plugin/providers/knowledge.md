---
provider_kind: knowledge
category: source
always_on: false
splices_into:
  - spec
description: Knowledge source provider — organizational memory, patterns, decisions; pulled in to inform elaboration and decomposition.
---

# Knowledge Provider — Behavior Contract

A knowledge provider is configured (Confluence, Notion, internal wiki) when `providers.knowledge.*` is set in `.darkrun/settings.yml`. Distinct from the spec provider: **spec** handles per-run documents (PRDs, RFCs); **knowledge** handles cross-run organizational memory (patterns, anti-patterns, decisions, prior art). Also distinct from darkrun's own `darkrun_knowledge_record` ledger: that is the project's local memory; this provider is the org's.

This is a **source** provider — read-only by default. Push back only at Reflect (and only when explicitly enabled).

## What you, the agent, must do

### At session start
- Pull organizational patterns relevant to the current factory + station.
- Load prior-art entries: has similar work been done before in this codebase or org?
- Load decision records that constrain the current work (architectural decisions, security policies, naming conventions).

### At Spec
- Search the knowledge base for the Run's problem space. Surface what's already known so the station builds on prior context rather than rediscovering it.
- Cite knowledge entries you pulled via `darkrun_external_ref` or in the relevant unit's body. Persist durable cross-run facts locally with `darkrun_knowledge_record` too — the local ledger is what future runs read first.

### At decompose
- If a unit conceptually re-implements something already documented as a pattern, name the pattern and link to it from the unit body.
- If a unit conflicts with a documented anti-pattern, surface the conflict to the operator before drafting — don't paper over it.

### At Reflect (optional, opt-in)
- Distill run learnings into org knowledge entries when `providers.knowledge.config.push_learnings: true`.
- Format: pattern (what worked + when to apply), anti-pattern (what failed + context), decision (choice + rationale + consequences).

## What NOT to do

- Don't load the entire knowledge base into context — search/filter to what's relevant to the current station.
- Don't push raw darkrun artifacts to the knowledge provider. Translate to the org's format (a reflection becomes a retrospective in the team's format).
- Don't write to the knowledge provider unless `push_learnings: true` is explicitly set.

## Translation map

| External concept | darkrun concept |
|---|---|
| Pattern library entry | Elaboration constraint |
| Anti-pattern entry | Decomposition red flag |
| Architectural decision record | Run / unit constraint |
| Cross-run reference | Station `inputs` |
