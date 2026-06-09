{% include "_shared/announcement.md" %}

# Environment blocked — `{{ station }}`

The gate **`{{ gate }}`** on unit `{{ unit }}` is **environment-blocked**, and the manager can't auto-recover it: there's no boot recipe for the missing dependency, or the tool to start it isn't installed. Grinding fix passes against a dead dependency just spends tokens to stay stuck.

**Why:** {{ reason }}

## What to do

This one's the operator's call:

1. **Bring the dependency up by hand** — start the service, install the missing tool, free the port — whatever the failure points at.
2. **Or declare a boot recipe** so the manager handles it next time: add the service to `.darkrun/boot.md` (a `name`, a `command`, and the `requires_tool` it needs on PATH).
3. **Or defer to CI** — if the gate genuinely can't run locally, let it ride; after repeated env-blocks the manager defers it to CI rather than wedge the run.

Once the environment is sound, **re-record the gate** with `darkrun_quality_gate_record`, then call `darkrun_tick`.

## Done when

The dependency is available (or the gate is deferred to CI) and the gate has been re-recorded. Then call `darkrun_tick`.
