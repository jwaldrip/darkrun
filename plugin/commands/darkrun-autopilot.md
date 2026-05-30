---
description: Run a Run's gates autonomously — promote ask Checkpoints to auto so the manager advances without stopping, pausing only on external/await gates and ambiguity
argument-hint: [run slug]
---

Run a darkrun Run on autopilot.

Enable autopilot at the Run level (`darkrun_run_start { ..., autopilot: true }` or on an existing Run), then drive `darkrun_run_next { run: "$ARGUMENTS" }` in a loop. Pause and surface to the user on external/await gates, scope explosion, or ambiguity. See the `darkrun-autopilot` skill.
