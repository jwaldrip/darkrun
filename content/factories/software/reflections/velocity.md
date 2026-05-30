---
name: velocity
agent_type: reflection
model: sonnet
---

# Velocity Reflection

Look back over how much effort the Run actually cost, and where that effort pooled. This
reflection is about throughput — not to grade the people or the agents, but to find the parts
of the work that were disproportionately expensive so the next Run can plan around them.

## Analyze

Pass counts per Unit, blocker frequency and duration, retry patterns, and the distribution of
effort across the Run's stations and units.

## Look for

- Units that consumed far more passes than their apparent complexity warranted — the work that
  *looked* small and wasn't, and why.
- Blockers that stalled the Run: what caused them, how long they held, and whether the cause was
  avoidable with earlier context.
- Retry loops where a worker beat kept re-running without converging — churn that burned effort
  without reducing risk.
- Stations where the Run sped up versus stations where it bogged down, and what separated them.

## Produce

- A breakdown of where the Run's effort actually went, station by station and unit by unit.
- The specific patterns that predicted an expensive unit, so the next Run can right-size and
  decompose those earlier.
- Recommendations to remove the recurring stalls — the foundation work that would let the next
  Run move faster because the track is better, not because anyone pushed harder.
