{% include "_shared/announcement.md" %}

# Resolve Drift — `{{ path }}`

A **locked** artifact changed out from under the line. Drift preempts everything — feedback and forward progress both wait — because the line's guarantees only hold if locked decisions stay locked.

{% include "_shared/contracts.md" %}

## What happened

`{{ path }}` was mutated after it was locked{% if station %} by station `{{ station }}`{% endif %}. The manager witnessed the change. Until it's reconciled, every downstream guarantee that depended on that artifact is suspect.

## What to do

1. **Diff** the current `{{ path }}` against its locked state. Know exactly what changed.
2. **Classify** the drift:
   - *Legitimate* — the change is correct and intended. Re-derive anything downstream that depended on the old value, then re-lock.
   - *Accidental / wrong* — the change should not have happened. Revert `{{ path }}` to its locked state.
3. **Reconcile downstream.** Any Unit, spec, or output built on the old version may now be stale — re-check, don't assume.
4. **Re-lock** `{{ path }}` once it and its dependents are consistent again.

## Done when

`{{ path }}` is reconciled and re-locked, downstream artifacts are consistent, and the drift entry is cleared. Then call `run_next`.
