{% include "_shared/announcement.md" %}

# Pending seal — `{{ run }}`

Every station is locked and the work is done — but the run declares a final **`{{ seal }}`** gate, so it isn't sealed yet. Delivery is its own decision: the run holds here until the work is actually accepted, not just built.

## What this means

{% if seal == "external" %}
- The run waits on an **external merge** — the delivery PR/MR must be merged before the run is considered shipped. Built-and-passing is not the same as delivered.
- Until then the artifacts are final but the run is not closed; reopening one is drift.
{% elif seal == "await" %}
- The run waits on an **await decision** — an operator (or an upstream gate) must explicitly accept delivery before the run seals.
- Until then the artifacts are final but the run is not closed; reopening one is drift.
{% else %}
- The run waits on a final delivery decision before it seals. The work is complete; acceptance is pending.
{% endif %}

## What to do

1. **Confirm the work is genuinely deliverable** — all stations locked, evidence in place, nothing half-finished.
2. **Drive the gate to a decision:**
{% if seal == "external" %}   - Make sure the delivery PR/MR is open, green, and ready to merge, and tell the operator it's awaiting merge.{% if compare_url %} No delivery PR exists yet — open it from the pre-filled create form: {{ compare_url }}{% endif %}{% elif seal == "await" %}   - Surface the await decision to the operator with `darkrun_question` so they can accept or hold delivery.{% else %}   - Surface the delivery decision to the operator.{% endif %}
3. **Wait for acceptance.** Don't self-approve delivery. When the gate clears (the merge lands / the operator accepts) the run frontmatter is marked complete and the next tick seals it.

## Done when

The delivery gate is satisfied and the run frontmatter is marked complete. Then call `darkrun_tick` — the manager seals the run.
