---
name: darkrun-debug
description: Admin recovery for wedged Runs — preview the manager cursor, force a Station complete, set engine-managed fields, reset drift, or patch a feedback record. Every mutation requires explicit user confirmation.
---

Recovery for Runs the manager's normal loop can't clear. Call `darkrun_debug` with `run` + `op` (+ a `reason` on every mutating op), and do exactly what it returns. The tool confirms each mutation with the user before touching `.darkrun/` — surface that prompt and never bypass or auto-retry it. Run the read-only `preview_cursor` op before and after any change.

## Recovering a single wedged Unit

When **one** Unit has run off the rails — burned its Pass budget (the manager escalates a runaway Pass loop), or its spec is wrong but its body is **locked while it executes** — reset just that Unit with **`darkrun_unit_reset`** (`slug` + `unit`, dry-run unless `confirm:true`). It returns the Unit to `pending`, which unlocks its body for editing and clears its Pass history, stamps, and gate results — **preserving the spec** (deps, inputs, outputs, declared gates, body). Then edit the spec and `darkrun_tick`: the Unit re-dispatches from Pass 1. This is lighter than a Station wipe (it touches one Unit, never its siblings) and is exactly what the runaway-Pass escalation message points you to.

The same capability is reachable from the **desktop review UI** without MCP: a unit-reset request there sets the Unit's `reset_requested` flag, and the engine performs the reset on its next tick.

For a destructive Station/Run wipe use `/darkrun:darkrun-reset` instead.
