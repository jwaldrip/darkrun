---
name: frame
description: Frame the problem — who has it, why it matters, and how we will know it is solved.
explorers: [context, value]
workers: [framer, challenger, distiller]
reviewers: [value, feasibility]
checkpoint: ask
locked_artifact: frame.md
inputs: []
---

# Frame

Frame is the factory's opening station. It kills the most expensive defect of
all: **building the wrong thing**. A flawless implementation of the wrong feature
is pure waste, and the cost of discovering it grows with every station it slips
past. Frame stops that at the cheapest possible point — before a single line of
spec or code exists.

## Risk class eliminated

*Wrong-thing risk.* The work is technically correct but solves a problem nobody
has, for a user who does not exist, with no way to tell whether it worked.

## What this station decides

- **The problem** — stated plainly, in the user's terms, not ours.
- **The user** — a concrete person with a concrete job-to-be-done.
- **The value** — why solving this matters now, and what it is worth.
- **The success metric** — the single observable that tells us we won.
- **The non-goals** — what we are explicitly *not* doing, to bound the work.

## The pass-loop

- **Framer** drafts the problem/user/value/metric from the run and Explorer context.
- **Challenger** attacks the frame: is the problem real, is the user right, is the metric measurable?
- **Distiller** reconciles the attack into the tightest frame that still survives.

## Locked artifact

`frame.md` — problem, user, value, success metric, non-goals. Every later station
inherits this frame and may not silently redefine it; a change to the problem is
drift that routes back here.

## Checkpoint

**ask.** Framing is a judgment call with real stakes, so a human confirms the
problem is worth solving before the factory spends anything specifying it.
