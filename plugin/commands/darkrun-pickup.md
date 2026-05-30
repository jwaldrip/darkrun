---
description: Advance a darkrun Run — the factory manager returns the next concrete action
argument-hint: [run slug]
---

Advance the active darkrun Run.

Call `darkrun_run_next` (omit the arg to resume the active Run, or pass `$ARGUMENTS`), do exactly what it returns, and loop until the manager reports completion. See the `darkrun-pickup` skill.
