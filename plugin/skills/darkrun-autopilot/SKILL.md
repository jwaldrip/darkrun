---
name: darkrun-autopilot
description: Run a Run's gates autonomously — promote ask Checkpoints to auto so the manager advances Station to Station without stopping, pausing only on external/await gates and ambiguity
---

# Autopilot

Drive a Run from description to delivery without stopping at human Checkpoints. Autopilot tells the
manager to promote `ask` Checkpoints to `auto`, so the lifecycle advances Station to Station on its
own. **External** and **await** Checkpoints still hold — those wait on a real-world signal the
manager can't synthesize.

## Process

1. **No active Run?** Create one with `/darkrun:darkrun-start`, then turn on autopilot.
2. **Enable autopilot** with `darkrun_run_start { ..., autopilot: true }` at creation, or on an
   existing Run by updating its mode through the manager. Autopilot is a Run-level setting — do not
   try to flip individual Checkpoints by hand; the manager owns Checkpoint promotion.
3. **Drive the loop** by calling `darkrun_run_next { run: "<slug>" }`. Do exactly what each return
   says, then call `darkrun_run_next` again. When a subagent returns, re-call to advance.

## What still pauses autopilot

- **External Checkpoints** — they need a real PR/MR merge signal and can't be auto-approved.
- **Await Checkpoints** — waiting on a non-review external event (a pipeline, a customer response).
- **Elicitation-required decisions** — design-direction picks, visual approvals.
- **Scope explosions** (see guardrails).
- **The final delivery Checkpoint**, unless the user explicitly asked for fully headless completion.

## Guardrails

- **Pause on blockers or ambiguity.** If the manager returns an error or a decision that can't be
  inferred from the Run's goals, stop and surface it. Never guess.
- **Pause on scope explosion.** If a Station decomposes into more than ~5 Units, stop and confirm
  scope — that signals the work is bigger than it looked and autopilot may be the wrong tool.
- **Don't open PRs autonomously mid-Run.** When the manager surfaces a mid-lifecycle external review,
  hand the PR step to the user. The final delivery PR after the Run completes is the exception.
- **Stop on persistent failures.** Any manager rejection that survives a single retry → stop and
  report it verbatim.
