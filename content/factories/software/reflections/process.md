---
name: process
agent_type: reflection
model: sonnet
---

# Process Reflection

Look back over how the Run actually *ran* — the flow through stations, the friction at
checkpoints, the places the manager and its workers spent effort that did not move the work
forward. This produces learning about the factory itself, not the artifact it built.

## Analyze

Station-transition friction, worker-beat effectiveness, checkpoint outcomes, and tool-failure
patterns drawn from the Run's record.

## Look for

- A station that locked an artifact a later station immediately had to reopen as drift —
  a sign the upstream station's output was incomplete.
- Checkpoints that routed rework back repeatedly: which station, and what kind of gap kept
  reopening?
- Worker passes whose Make beat was abandoned at Challenge — wasted work that better Explore
  context would have prevented.
- Tool or environment failures that forced retries, and context lost across the Run (the same
  decision remade, the same question re-asked at two stations).

## Produce

- Recommendations to sharpen specific role instructions where a beat consistently underdelivered.
- An assessment of which station hand-offs were clean and which leaked, with the input/output
  gap named.
- Configuration or convention changes that would cut the friction this Run hit, so the next Run
  spends its effort on the work instead of the process.
