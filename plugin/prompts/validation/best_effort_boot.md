{% include "_shared/announcement.md" %}

# Best-effort boot — `{{ station }}`

The gate **`{{ gate }}`** on unit `{{ unit }}` didn't fail because the code is wrong — it failed because a dependency it needs wasn't reachable. The manager classified it as **environment-blocked** and wants the service up *before* the fix loop touches your code.

## What to do

1. **Boot the declared services.** `.darkrun/boot.md` says how:
{% if services %}{% for s in services %}
   - `{{ s }}`
{% endfor %}{% else %}
   - (no recipe services resolved — bring the dependency up however this repo does)
{% endif %}
2. **Wait for ready.** Give each service a moment to accept connections (a health check or a short sleep), so the gate doesn't re-fail on a half-up dependency.
3. **Re-run the gate** and record it with `darkrun_quality_gate_record`. A `pass` clears the block; a real failure *now*, with the service up, is a genuine defect and routes to the fix loop.

## Done when

The dependency is up and the gate has been re-recorded — `pass` to advance, or a true `fail` if the work itself is wrong. Then call `darkrun_tick`.
