---
description: Admin recovery for wedged Runs — preview the manager cursor, force a Station complete, set engine-managed fields, reset drift, or patch a feedback record. Every mutation requires user confirmation.
argument-hint: [run slug] [op]
---

Run a darkrun admin recovery op.

Route every op through `darkrun_debug` (`preview_cursor`, `force_station_complete`, `set_run_field`, `reset_drift`, `mutate_feedback`). Mutating ops require a `reason` and explicit user confirmation — never run one unilaterally. See the `darkrun-debug` skill.

To rescue a single wedged/bolt-capped Unit (its body is locked while it executes), use `darkrun_unit_reset` (`slug` + `unit`, dry-run unless `confirm:true`) — it returns just that Unit to `pending` so its body unlocks and it re-runs from Pass 1, preserving the spec. Lighter than a Station wipe; also reachable from the desktop review UI via the Unit's `reset_requested` flag.
