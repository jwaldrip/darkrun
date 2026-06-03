{% include "_shared/announcement.md" %}

# Review — `{{ station }}` spec

Before a single Unit is manufactured, the spec gets reviewed. A bad spec that reaches manufacture is the most expensive failure in the line — kill it here, cheaply.

{% include "_shared/contracts.md" %}

{% include "_shared/roster.md" %}

Review walks three beats, in order: **spec → adversarial → brief**. The operator's decision is a *separate* step the manager surfaces as a gate once this work lands — you do not ask for it inline here.

## 1. spec — verify against the spec

Read the spec produced in the previous phase against its own intent. Before any adversary touches it, confirm it is internally coherent:

- Does it actually name and bound **{{ kills }}**, or does it leave a hole?
- Does every Unit carry testable completion criteria and explicit dependencies?
- Is the out-of-scope boundary stated, so later phases can't drift into it?

## 2. adversarial — adversarial reviewer pass

Dispatch each reviewer{% if reviewers %} ({% for r in reviewers %}`{{ r }}`{% if not loop.last %}, {% endif %}{% endfor %}){% endif %} against the spec. Each owns one lens — let them be ruthless inside it:

- Does the spec actually eliminate **{{ kills }}**, or does it only look like it does?
- Are the completion criteria testable, or are they wishful?
- Are the Units genuinely independent, or will they collide during manufacture?
- Is anything load-bearing left unstated?

**A reviewer reviews — it does not redesign.** Each reviewer MUST NOT propose new requirements outside the spec's stated intent, MUST NOT substitute its own approach or relitigate a settled tradeoff, and MUST NOT block on stylistic preference. It finds where the spec fails *its own* goal and files exactly that — nothing more.

{% if units %}
### Units under review
{% for u in units %}
- `{{ u }}`
{% endfor %}
{% endif %}

If a reviewer blocks, fix the spec and re-review — do not advance a spec a reviewer rejected.

## 3. brief — the review summary

Produce a tight brief of the review: which lenses signed off, which filed concerns, and how each concern was resolved (or why it was deferred). This is the record manufacture inherits — it should make the spec's verdict obvious without re-reading every reviewer's notes.

## Done when

Every reviewer has signed off or filed addressable concerns and the brief is recorded. Then call `darkrun_tick` — the manager opens the operator's pre-execution gate on the next tick. Do not surface the decision inline; the gate does that.
