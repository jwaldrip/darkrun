{% include "_shared/announcement.md" %}

# User gate — `{{ station }}` spec

The review work for station **{{ station }}** is done: the spec is written, the adversarial reviewers have had their pass, and the review brief is recorded. Before a single Unit is manufactured, the **operator** reviews the station and clears it — this is the pre-execution gate, the cheapest place to catch a wrong direction.

{% include "_shared/contracts.md" %}

## The gate is open in the desktop review surface

The manager has surfaced this gate to the operator's review surface (the desktop app). The operator reads the spec/brief there and either approves the wave or returns feedback. **You do not ask inline and you do not advance the run yourself.**

- The operator **approves** → the gate clears and the next tick releases the manufacture wave.
- The operator **returns feedback** → it lands as a fix track; address it, then the gate re-opens for their re-decision.

## Done when

The operator has cleared the gate via `darkrun_checkpoint_decide`. Until then, this gate holds. Call `darkrun_tick` to re-check — a held gate is not a wedge; it is waiting on a human.
