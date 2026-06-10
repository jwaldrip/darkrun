{% include "_shared/announcement.md" %}

# Checkpoint — `{{ label }}`

Station **{{ station }}** has passed spec, review, manufacture, audit, and reflect. The gate is now open. Its kind is **`{{ kind }}`** — that determines who decides whether the station locks.

{% include "_shared/contracts.md" %}

Checkpoint walks two beats, in order: **brief → user**.

## 1. brief — produce the closing-brief summary

Write the tight closing brief for whoever holds the decision. It is the durable record of *why this station is allowed to lock*, so make it stand on its own:

- What this station eliminated: **{{ kills }}**.
- The locked artifact{% if locked_artifact %} (`{{ locked_artifact }}`){% endif %} and where the evidence lives — specs, audit verdict, the green check run.
- Any concerns reviewers raised and how they were resolved.
- The retrospective learnings reflect surfaced, if they bear on the lock.

Persist it as the station's **outcome**: call `darkrun_brief_record` with `slug: {{ run }}`, `station: {{ station }}`, `phase: post`, and the closing brief as `body`. This is the durable "what the station produced" record the checkpoint surfaces — write it before clearing the gate.

{% if checkpoint_options %}
## This station offers a choice of gate paths

Station **{{ station }}** is a **compound gate** — the operator may take any of: {% for o in checkpoint_options %}**`{{ o }}`**{% if not loop.last %}, {% endif %}{% endfor %}. The default is **`{{ kind }}`**. If the operator wants a different path (e.g. route this to a formal `external` review instead of an inline `ask`), record their pick with `darkrun_checkpoint_choose` (station + chosen kind) **before** clearing the gate; the next tick re-routes the checkpoint to that path. With no pick, the default below applies.
{% endif %}

## 2. user — the gate decision (`{{ kind }}`)

The gate kind decides *who* clears it. Surface the brief above, then act per the kind:

{% if kind == "auto" %}
**auto** — no human in the loop. The evidence already justifies the lock. Confirm the criteria are met, lock the station, and call `darkrun_tick` to advance.
{% elif kind == "ask" %}
**ask** — a human must approve, **in the desktop review surface this tick raised** (the engine brings it up automatically at an ask gate). Surface the closing brief there and **hold**. Do NOT ask the operator inline — no `AskUserQuestion`, no chat prompt; the decision lives in the desktop, not the transcript. Do not advance the run until the operator approves. On approval, lock and call `darkrun_advance`; on rejection, route their feedback as a fix track. If the desktop did not come up, call `darkrun_run_inspect` to raise it.
{% elif kind == "external" %}
**external** — an external system or process must clear this gate (CI, a deploy, a sign-off elsewhere). Surface what's required, trigger or point at it, and hold until it reports back. Lock only on a real external pass.
{% elif kind == "await" %}
**await** — outstanding asynchronous work must settle before locking. Identify what's in flight, wait for it, and re-check the criteria once it lands.
{% else %}
Unknown checkpoint kind `{{ kind }}` — treat as **ask**: surface the gate to a human and hold.
{% endif %}

## Done when

The gate is cleared per its kind and the station is locked, or the run is held for a decision. Either way, call `darkrun_tick` to record the outcome.
