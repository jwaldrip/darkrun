---
description: Zero-ceremony single-Unit execution — run one task straight through a Station's Worker loop with nothing written under .darkrun/
argument-hint: [the task]
---

Run a zero-ceremony darkrun zap for: $ARGUMENTS

Call `darkrun_zap { task: "$ARGUMENTS" }` and follow the returned `message` verbatim. Stateless — nothing is written under `.darkrun/`. See the `darkrun-zap` skill.
