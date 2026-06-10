{% include "_shared/announcement.md" %}

# Manufacture — `{{ label }}`

This is the build floor. You run the **Pass loop** — _Plan → Make → Challenge → Resolve_ — over the wave-ready Units. The current beat is **{{ worker }}**{% if model %}, on model **{{ model }}**{% endif %}.

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

{% if unit_specs %}
## Each Unit's spec — the contract the beat works against

The subagent you dispatch for a Unit gets **no context beyond what you hand it**. Pass the Unit's spec below into its dispatch verbatim — the completion criteria with their verify commands, the declared paths, and the scope boundary are the contract the beat is judged against.
{% for s in unit_specs %}
### `{{ s.unit }}` — {{ s.title }}
{% if s.inputs %}
- **inputs:** {% for i in s.inputs %}`{{ i }}`{% if not loop.last %}, {% endif %}{% endfor %}
{% endif %}
{% if s.outputs %}
- **outputs:** {% for o in s.outputs %}`{{ o }}`{% if not loop.last %}, {% endif %}{% endfor %}
{% endif %}
{% if s.gates %}
- **quality gates:** {% for g in s.gates %}{{ g }}{% if not loop.last %} · {% endif %}{% endfor %}
{% endif %}

{{ s.body }}
{% endfor %}
{% endif %}

{% if worktrees %}
## Each Unit has its own worktree — work in it

Every wave Unit is isolated on its own branch + worktree, forked off the station branch. Run that Unit's beat **inside its worktree** so its diff never tangles with another Unit's in-flight work; the manager lands each Unit back onto the station branch when it locks. Do **not** commit a Unit's work to the station branch yourself.
{% for w in worktrees %}
- `{{ w.unit }}` → `{{ w.path }}` (branch `{{ w.branch }}`)
{% endfor %}
{% endif %}

{% if handoffs %}
## Handoff from the prior beat — read before you act

Each line is the last worker's own account of what it did, or why it bounced this Unit back. This is the baton: act on it, don't re-derive it.
{% for h in handoffs %}
- `{{ h.unit }}` — **{{ h.worker }}** ({{ h.result }}): {{ h.note }}
{% endfor %}
{% endif %}

## The Pass loop — make → challenge → resolve

The Pass loop is adversarial on purpose: a single confident pass is exactly where LLM output is most often confidently wrong, so a second pass red-teams the first before anything locks.

- **make** — the worker produces the Unit's output against its completion criteria. Build the real thing, not a sketch.
- **challenge** — a second pass attacks what make produced: edge cases, missing handling, lazy assumptions. Assume the first pass was optimistic.
- **resolve** — reconcile make and challenge into a Unit that satisfies its completion criteria with the challenges answered.

{% if worker_roles %}
**Reject routing.** Workers carry a pass-loop role: {% for w in workers %}{% if worker_roles[w] %}`{{ w }}` = {{ worker_roles[w] }}{% if not loop.last %}, {% endif %}{% endif %}{% endfor %}. A `build` worker produces and repairs; a `verify` worker only judges; a `plan` worker only designs. When a beat **rejects**, bounce back to the **nearest preceding `build` worker** (pass it as `next_worker` to `darkrun_unit_iterate`) — skip `verify`/`plan` beats on the way back, since they can't fix. An `advance` rolls forward to the next worker in order.
{% endif %}

{% if verifier_nonce %}
**Quality-gate verifier nonce.** This dispatch carries a one-time verifier token: **`{{ verifier_nonce }}`**. When you record a quality gate with `darkrun_quality_gate_record`, pass it as `nonce`. The engine refuses a gate result without the matching token — so a gate is only ever recorded as part of a real verification dispatch, never self-certified. Run the gate's command for real, then record the actual outcome with this nonce.
{% endif %}

Run **only the `{{ worker }}` beat** this tick. When the beat finishes, **record it** with `darkrun_unit_iterate` — pass the `worker`, the `result` (`advance` or `reject`), and a `note`: on advance, what you did and what the next worker needs to know; on reject, why you bounced it (a reject without a reason is refused). That note becomes the next beat's handoff above. Then call `darkrun_tick`; the manager advances the loop or releases the next wave. A Unit is locked only after Resolve and its completion criteria pass.

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

The `{{ worker }}` beat is complete for every Unit in this wave and its output is recorded. Then call `darkrun_tick`.
