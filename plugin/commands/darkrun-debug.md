---
description: Admin recovery for wedged Runs — preview the manager cursor, force a Station complete, set engine-managed fields, reset drift, or patch a feedback record. Every mutation requires user confirmation.
argument-hint: [run slug] [op]
---

Run a darkrun admin recovery op.

Route every op through `darkrun_debug` (`preview_cursor`, `force_station_complete`, `set_run_field`, `reset_drift`, `mutate_feedback`). Mutating ops require a `reason` and explicit user confirmation — never run one unilaterally. See the `darkrun-debug` skill.
