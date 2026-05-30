{% include "_shared/announcement.md" %}

# Manufacture — `{{ station }}`

This is the build floor. You run the **Pass loop** — _Plan → Make → Challenge → Resolve_ — over the wave-ready Units. The current beat is **{{ worker }}**.

{% include "_shared/contracts.md" %}

{% include "_shared/roster.md" %}

## This wave

{% if units %}
Dispatch the **{{ worker }}** beat in parallel across these wave-ready Units:
{% for u in units %}
- `{{ u }}`
{% endfor %}
{% else %}
No Units are wave-ready this tick. The previous wave's dependents are still blocked, or work is mid-flight.
{% endif %}

## The Pass loop — make → challenge → resolve

The Pass loop is adversarial on purpose: a single confident pass is exactly where LLM output is most often confidently wrong, so a second pass red-teams the first before anything locks.

- **make** — the worker produces the Unit's output against its completion criteria. Build the real thing, not a sketch.
- **challenge** — a second pass attacks what make produced: edge cases, missing handling, lazy assumptions. Assume the first pass was optimistic.
- **resolve** — reconcile make and challenge into a Unit that satisfies its completion criteria with the challenges answered.

Run **only the `{{ worker }}` beat** this tick. When it returns, call `run_next`; the manager advances the loop or releases the next wave. A Unit is locked only after Resolve and its completion criteria pass.

## Done when

The `{{ worker }}` beat is complete for every Unit in this wave and its output is recorded. Then call `run_next`.
