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

A Unit gets a **bounded pass budget** — the manager escalates a Unit that can't converge within it to the operator rather than grinding forever. Don't paper over a stuck Unit to dodge the escalation; a Unit that needs more passes than the budget allows is a signal the spec, the scope, or the approach is wrong, and that's the operator's call to make.

{% if user_facing %}
## Visual work — get a design direction first

This Unit has a **user-facing surface** (a screen, flow, component, or page). Before you build any UI, settle the look and feel *with the operator* — a visual choice is expensive to reverse once built, and it is the operator's to make:

1. **Generate options.** Produce two to four candidate design directions for the surface — mockups / option images that render the layout, hierarchy, and tone of each. Honour the existing brand and design tokens; extend the system, don't reinvent it.
2. **Ask for the decision.** Use `darkrun_question` to have the operator pick the winning mockup from the option images, or `darkrun_direction` to have them choose a design archetype and annotate it (pins, screenshots, comments).
3. **Build to the chosen direction.** Treat the operator's pick — its image urls and annotations — as a locked visual contract. Implement that direction; don't re-litigate it mid-build.

For non-UI work — internal logic, headless jobs, APIs — there is no surface to shape, so skip this entirely and run the beat as normal.
{% endif %}

## Done when

The `{{ worker }}` beat is complete for every Unit in this wave and its output is recorded. Then call `run_next`.
