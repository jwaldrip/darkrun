---
name: darkrun-debug
description: Admin recovery for wedged Runs — preview the manager cursor, force a Station complete, set engine-managed fields, reset drift, or patch a feedback record. Every mutation requires explicit user confirmation.
---

# Debug

Recovery tools for Runs that are corrupt, wedged, or stuck in ways the manager's normal loop can't
clear. Every mutating op runs through `darkrun_debug` and pauses for explicit user confirmation
before any state under `.darkrun/` changes. **The agent never runs an admin op unilaterally.**

## When to use

- A Run is stuck in a loop the manager's halt mechanism caught and the user wants to manually
  unblock it.
- A Station's Units have moved through every Worker, but the cursor won't advance because the
  Checkpoint's review/approval stamps never landed.
- A Run's mode is wrong (e.g. set to autopilot when it shouldn't be) — mode is normally manager-owned
  and a mid-flight change needs an override.
- The drift sweep keeps re-firing on stale witnesses even though the underlying state already
  matches — re-stamp every witness to the current SHA.
- A feedback record is in an impossible state (resolved but resolution still empty) and needs
  surgical correction.

## When NOT to use

- For a full destructive wipe of a Station or Run, use `/darkrun:darkrun-reset`.
- For day-to-day "why didn't this advance" questions, `/darkrun:darkrun-show` or reading the manager
  output is faster.

## How to call

`darkrun_debug` takes `run` + `op` + op-specific args. Always pass a `reason` on mutating ops —
free-text prose explaining *why* you're reaching for the override. It's shown verbatim in the
confirmation prompt so the user can authorize the bypass with full context.

- **`preview_cursor`** — read-only, no confirmation. What would the next `darkrun_run_next` return,
  given current on-disk state? Run it before and after any mutation.
- **`force_station_complete { station, reason }`** — stamp the Checkpoint reviews/approvals for every
  Unit in Stations up to and including the target. Refuses Units that haven't moved through every
  Worker.
- **`set_run_field { field, value, reason }`** — bypass manager-protected fields (primarily mode).
- **`reset_drift { reason }`** — re-stamp every witnessed slot with the current SHA; the drift sweep
  stops finding mismatches afterward.
- **`mutate_feedback { feedback_id, patch, reason }`** — set feedback record fields directly, no
  lifecycle guards.

## Confirmation flow

For every mutating op the tool surfaces the exact mutation and waits. If the user approves, it runs
and returns the result. If the user cancels, the response is `{ action: "cancelled" }` and nothing
changed. Surface both outcomes verbatim and never auto-retry on cancellation — that defeats the gate.

## After an admin op

Always call `preview_cursor` again to confirm the wedge is gone. If the cursor still emits the same
stuck action, the underlying state needs more work — capture the `preview_cursor` output and file a
report via `/darkrun:darkrun-report`.
