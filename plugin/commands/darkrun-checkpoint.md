---
description: Review and decide a Station's Checkpoint — approve to advance, or request changes to route rework as drift
---

Decide a darkrun Checkpoint.

Review the holding Station via `/darkrun:darkrun-show`, then call `darkrun_checkpoint_decide` (`approved: true` to lock and advance, or `approved: false` with `feedback` to route rework back through the feedback track) and continue with `/darkrun:darkrun-pickup`. See the `darkrun-checkpoint` skill.
