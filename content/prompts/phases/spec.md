{% include "_shared/announcement.md" %}

# Spec — `{{ station }}`

You are opening station **{{ station }}**. Its job is to eliminate a whole class of risk: **{{ kills }}**. Nothing downstream is allowed to proceed until that risk is named and bounded here.

{% include "_shared/contracts.md" %}

{% include "_shared/roster.md" %}

Spec walks two beats, in order: **elaborate → explore**.

## 1. elaborate — frame the problem

State plainly what this station must achieve to kill **{{ kills }}**: the intent, the inputs it inherits from upstream, and the boundary of what is explicitly *out of scope* so later phases don't drift into it. This is the frame the explorers go to work against.

## 2. explore — run the explorers, then decompose

1. **Run the explorers.** Dispatch each explorer{% if explorers %} ({% for e in explorers %}`{{ e }}`{% if not loop.last %}, {% endif %}{% endfor %}){% endif %} against the current state of the work. Explorers don't build — they surface unknowns, constraints, prior art, and traps. Collect their findings before you decompose.
2. **Decompose into Units.** Turn the explored problem into the smallest set of independently completable **Units** that, together, kill the risk above. For each Unit write:
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

## Done when

The spec names the risk, lists Units with testable completion criteria and dependencies, and marks what's out of scope. Write it to the station's spec artifact, then call `run_next`.
