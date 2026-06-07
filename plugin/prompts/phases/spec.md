{% include "_shared/announcement.md" %}

# Spec — `{{ label }}`

You are opening station **{{ station }}**. Its job is to eliminate a whole class of risk: **{{ kills }}**. Nothing downstream is allowed to proceed until that risk is named and bounded here.

{% include "_shared/contracts.md" %}

{% include "_shared/roster.md" %}

Spec runs **elaboration and discovery in tandem** — they are NOT two sequential
steps. The moment the station opens, kick off both at once: dispatch the explorers
in parallel *while* you frame the problem. They sharpen each other. Only once both
have landed do you decompose.

## elaborate — frame the problem (concurrently with discovery)

State plainly what this station must achieve to kill **{{ kills }}**: the intent, the inputs it inherits from upstream, and the boundary of what is explicitly *out of scope* so later phases don't drift into it. This is the frame the explorers work against — but do NOT wait on a finished frame to start them; the frame and the exploration are written in parallel and inform each other.

## discover — run the explorers in parallel (concurrently with elaboration)

Dispatch **all** explorers{% if explorers %} ({% for e in explorers %}`{{ e }}`{% if not loop.last %}, {% endif %}{% endfor %}){% endif %} **at once, in parallel** — one subagent each, fanned out concurrently, never one-after-another. Explorers don't build — they surface unknowns, constraints, prior art, and traps. They run alongside your framing; neither blocks the other.

{% if knowledge %}
**Project knowledge (priors from earlier runs)** — build on these, don't re-discover them:
{% for k in knowledge %}
- **{{ k.topic }}** — {{ k.body }}
{% endfor %}
{% endif %}

When discovery surfaces a durable project fact worth carrying into **future** runs — a constraint, prior art, a convention, a trap — persist it with **`darkrun_knowledge_record`** (`topic` + `body`). That's the project's shared memory; re-recording a topic updates it. Keep it project-level (cross-run truths), not this run's transient details.

## decompose — once elaboration + discovery have both landed

Turn the framed, explored problem into the smallest set of independently completable **Units** that, together, kill the risk above. For each Unit write:
   - a one-line intent,
   - explicit **completion criteria** (how you'll know it's done — testable, not vibes),
   - its dependencies on other Units (so the manager can wave them).

{% if units %}
### Units already on record
{% for u in units %}
- `{{ u }}`
{% endfor %}
Reconcile these against what the explorers found — extend, split, or tighten them; don't blindly accept them.
{% else %}
There are no Units yet. You are creating them.
{% endif %}

{% if user_facing %}
### User-facing surfaces

This work touches a **user-facing surface**. For every Unit that renders a screen, flow, component, or page, mark it as visual so Shape's design step knows to act: its UI must not be built until the operator has chosen a design direction (via `darkrun_question` / `darkrun_direction`). Make the surface and its acceptance criteria explicit here; non-visual Units carry no such requirement.
{% endif %}

{% if needs_collaboration %}
## Collaborate with the operator — required before this spec locks

This run is in a **collaborative mode**, and the station will not advance to Review until you have actually involved the operator in shaping the spec. Do not author the whole spec solo and surface it only at the gate — bring the operator in *now*, while the frame is still soft:

- Surface the open framing questions and the consequential choices to the operator with `darkrun_question` (a decision) or `darkrun_direction` (a direction to steer), and fold their answers into the spec.
- When the spec genuinely reflects that collaboration, call **`darkrun_elaborate_seal`** for this station — that clears the hold and the next tick advances to Review.

If you tick without involving the operator, the station stays in Spec; a stalled, non-collaborative Spec escalates to the operator rather than slipping past them. (Autonomous modes — autopilot / quick — don't gate here.)
{% endif %}

## Done when

The spec names the risk, lists Units with testable completion criteria and dependencies, marks what's out of scope,{% if needs_collaboration %} the operator has been involved and `darkrun_elaborate_seal` is called,{% endif %} and it's written to the station's spec artifact. Then call `darkrun_tick`.
