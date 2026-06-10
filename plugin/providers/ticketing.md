---
provider_kind: ticketing
category: workflow
always_on: false
splices_into:
  - spec
  - manufacture
  - checkpoint
  - pending_seal
description: Ticketing workflow provider — bidirectional sync between darkrun units/runs and external issue trackers.
---

# Ticketing Provider — Behavior Contract

A ticketing provider is configured (Jira, Linear, GitHub Issues, GitLab Issues, …) when `providers.ticketing.*` is set in `.darkrun/settings.yml`. This contract applies any time you're operating on a Run in such a project.

## Mode

Capability is gated on which MCP tools you actually have available:

- **Full operational control** — you have create/update/transition tools for the configured tracker. Create tickets from units, transition status as the unit moves, close on completion.
- **Read-only-with-references fallback** — you have read access (or no MCP tool at all). Don't try to call write APIs. Just record `external_refs` (via `darkrun_external_ref`) so the link is auditable and a human (or a future session with the tools) can sync.

Detect at use time by checking your available tool surface. Don't fail loudly when tools are missing — degrade silently to the fallback.

## What you, the agent, must do

### At run creation
- If the user references an existing epic / parent issue, record it with `darkrun_external_ref` (`ticket` on the run).
- If no epic exists and you have create tools, create one and stamp the new key the same way.
- Without create tools, leave the field empty and tell the user "tracked locally; link an epic later via `darkrun_external_ref`."

### At decompose (drafting units)
- If you have create tools: create a ticket per unit, link it to the run's epic, and record the key on the unit.
- Map unit `depends_on` to ticket blocked-by links when the provider supports it.
- Without create tools: leave the ticket ref empty; the user fills it in retroactively.

### At unit advance (status sync)
- When the unit's first beat starts, move the ticket to **In Progress** (if you have the transition tool).
- When the unit locks (final beat advanced, criteria met), move the ticket to **Done**.
- When a beat rejects (bounces rework), add the reject reason as a ticket comment and keep the ticket In Progress.

### At station / run completion
- Post a station-summary comment to the epic when a station's checkpoint locks (one comment per station, listing the units that landed).
- At run seal, transition the epic to **Done** and post the closing summary.

## What NOT to do

- Don't push raw darkrun frontmatter to the tracker. Translate: unit body prose → ticket description; unit `quality_gates` → ticket checklist; unit `depends_on` → linked tickets.
- Don't create top-level provider keys in settings (no top-level `jira:` block). All config lives under `providers.ticketing.config`.
- Don't fabricate ticket keys. If you can't reach the tracker, leave the ref empty and surface a one-line note in your response.

## Translation map

| darkrun concept | Ticket concept |
|---|---|
| Run | Epic / parent issue |
| Unit | Child issue / sub-task |
| `depends_on` | Blocked-by link |
| Station | (optional) Sprint / iteration assignment |
| Review finding | Comment on the unit's ticket |
| Station checkpoint locks | Comment on the epic |
| Run seal | Epic transition to Done |
