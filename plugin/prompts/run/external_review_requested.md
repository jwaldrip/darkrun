{% include "_shared/announcement.md" %}

# External review — `{{ station }}`

Station `{{ station }}` is locked behind an **external gate**. Its work doesn't advance on a local prompt — it hands off to an external review surface (a pull/merge request) where a human reviews it on their own time. The line holds here until that review lands a decision.

{% include "_shared/contracts.md" %}

## What to do

1. **Package the work for review.** Make sure the station's locked artifacts and the diff are coherent and self-explanatory — a reviewer should understand *what* changed and *why* without a meeting.
2. **Open or update the external review.** Create the PR/MR (or update the existing one){% if target %} — current target: `{{ target }}`{% endif %}. Write a description that states the intent, the change, and the evidence (specs, audits, proof). Use the operator's connected VCS auth; don't invent a remote.{% if compare_url %}
   No hosting client could open it programmatically — use the pre-filled create form: {{ compare_url }}{% endif %}
3. **Annotate what matters.** Call out the risky parts, the decisions you want eyes on, and anything the reviewer should not rubber-stamp.
4. **Hand off and wait.** This gate is **not** yours to approve. Report the review URL to the operator and stop — the station advances only when the review is approved and the decision comes back through `darkrun_checkpoint_decide`.

## Done when

The external review is open, well-described, and the operator has the link. Then call `darkrun_tick` — the station stays held at this gate (awaiting) until the review is decided.
