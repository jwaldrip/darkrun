---
description: Wipe one Station of a Run (Units, outputs, artifacts, decomposition, feedback, branch) so the manager re-enters it from scratch — or reset/archive the whole Run. Other Stations stay untouched.
argument-hint: [run slug] [station]
---

Reset a darkrun Station or Run.

Identify the Run, then call `darkrun_run_reset { run, station }` (Station scope) or omit `station` / `darkrun_run_archive` (Run scope). Destructive — confirmed before anything is removed. See the `darkrun-reset` skill.
