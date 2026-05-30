---
description: Pre-Checkpoint code review with a multi-agent fix loop — compute the diff, dispatch Reviewers, and process findings before the Checkpoint locks
---

Run a darkrun gate review.

Call `darkrun_gate_review` to compute the Station diff and get review instructions, follow them verbatim, drive the fix loop until clean, then decide the Checkpoint via `/darkrun:darkrun-checkpoint`. See the `darkrun-gate-review` skill.
