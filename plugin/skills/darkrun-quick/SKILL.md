---
name: darkrun-quick
description: Quick single-Station Run — create a Run the manager auto-sizes to one Station, then advance it through that Station's phases
---

# Quick Run

A quick task is an ordinary Run that the manager right-sizes down to a **single Station**. The only
difference from `/darkrun:darkrun-start` is that you steer the manager toward one Station instead of
a full factory traversal — everything else (Workers, Reviewers, Checkpoints) is identical.

## Process

1. **Prelaborate briefly.** If the task is vague, ask ONE clarifying question via `AskUserQuestion`
   with `options[]`. Otherwise skip.
2. **Create the Run** with `darkrun_run_start`:
   - `title` — 3–8 words, ≤80 chars, single line. Not a truncated description. Good: `"Fix login
     button padding"`. Bad: `"Fix login button padding on mobile because…"`.
   - `description` — 2–5 sentences of context.
   - `slug` — kebab-case id from the title.
   - `context` — key constraints/decisions from the conversation.
   - `factory_candidates` — the 2–4 best-fit factories from `darkrun_factory_list`.
3. **Let the manager size it.** Right-sizing is automatic: for a one-Station task the manager
   collapses the lifecycle so the Run runs that Station (typically Build → Prove) with auto
   Checkpoints. Don't pre-pick a "mode."
4. **Advance** by calling `darkrun_run_next { run: "<slug>" }`. Each call returns the next concrete
   action; do exactly what it returns and loop until the Station's Checkpoint resolves.

## Guardrails

- If the work clearly needs multiple Stations, stop and suggest `/darkrun:darkrun-start` — don't
  cram it into one Station.
- If the work is trivial and you want zero state, use `/darkrun:darkrun-zap` instead.
