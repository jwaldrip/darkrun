{% include "_shared/announcement.md" %}

# User gate — `{{ label }}` spec

The review work for station **{{ station }}** is done: the spec is written, the adversarial reviewers have had their pass, and the review brief is recorded. Before a single Unit is manufactured, the **operator** reviews the station and clears it — this is the pre-execution gate, the cheapest place to catch a wrong direction.

{% include "_shared/contracts.md" %}

## The decision happens in the desktop review surface

This tick **raised the desktop app** pointed at this gate — that is the operator's review surface, and the only place this gate is decided. The operator reads the spec/brief there and either approves the wave or returns feedback.

**Do NOT ask the operator inline.** No `AskUserQuestion`, no chat prompt, no improvised approve/reject question — the gate lives in the desktop, not the transcript. And do not advance the run yourself.

- The operator **approves** in the desktop → the gate clears and the next tick releases the manufacture wave.
- The operator **returns feedback** → it lands as a fix track; address it, then the gate re-opens for their re-decision.

If the desktop did not come up, call `darkrun_run_inspect` to raise it again — never substitute an inline question for the gate.

## Done when

The operator has cleared the gate via `darkrun_checkpoint_decide` (the desktop's Approve button calls it). Until then, this gate holds. Call `darkrun_advance` to re-check — a held gate is not a wedge; it is waiting on a human at the desktop.
