---
name: darkrun-gate-review
description: Pre-Checkpoint code review with a multi-agent fix loop — compute the diff, dispatch Reviewers, and process findings before the Checkpoint locks
---

# Gate Review

Run a focused code review at a Station's Checkpoint before the artifact locks.

1. Call `darkrun_gate_review` to compute the diff for the current Station and get review
   instructions.
2. Follow the returned instructions verbatim — they spell out which Reviewers to spawn, what each
   inspects, and how to process findings.
3. Drive the fix loop: dispatch fix-workers for each finding, re-run the affected checks, and repeat
   until the Reviewers come back clean or the user accepts the remaining findings.
4. Once clean, advance the Checkpoint with `/darkrun:darkrun-checkpoint` (approve) or route the
   remaining findings as rework (request changes).

Gate review is the audit phase made explicit — use it when you want a deliberate review pass before
deciding a Checkpoint, rather than letting the Station's default Reviewers run inline.
