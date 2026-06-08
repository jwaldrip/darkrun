# How it works

darkrun is a small set of ideas that compose. Learn the four nouns, the one loop,
and the gate types, and you can predict exactly what the manager will do next.

## The hierarchy: Factory > Station > Unit > Pass

Work nests four levels deep, and every level has a job.

:::columns
- **Factory** — a methodology: an ordered set of stations that take work from
  intent to shipped. The software factory's line is Frame → Specify → Shape →
  Build → Prove → Harden. The top of the hierarchy.
- **Station** — one stage of the line. It runs the universal phase machine and
  locks exactly one durable artifact. Frame locks the problem; Specify locks the
  contract; Build locks the implementation.
- **Unit** — a discrete piece of work a station produces and you review. A station
  decomposes its work into Units with testable completion criteria before making
  anything.
- **Pass** — one Make → Challenge → Resolve cycle a worker runs inside a Unit.
  Produce a candidate, attack it for its weakest seam, fix what the attack
  surfaced.
:::

A **Run** walks a factory's stations against a real task. That is the whole
vocabulary.

## One method, many lines

Here is the load-bearing distinction. The **phase machine** is universal — every
station, in every factory, walks the same six phases. The **stations** are
per-factory — the software factory's line is ordered by cost-of-late-discovery,
but a different factory declares a different line. One method, many lines. The
machine and the ordering principle are fixed; the station names and count are the
recipe, not the law.

## The run loop: one action per tick

darkrun does not run a Run as a single opaque call. The **manager** is a loop, and
each tick it returns the next concrete action — explore this context, decompose
this station into Units, run this Pass, attach this proof, decide this checkpoint.
You perform exactly that, then tick again.

This is deliberate. A loop that returns one action at a time is inspectable: you
can always ask "what is the manager about to do, and why?" and get a real answer.
It is also recoverable — a wedged Run is just a cursor sitting on an action, and
you can preview it, force it, or reset it without unwinding the whole thing. The
train can only move as fast as the tracks it is built on; the loop is the track.

## The phase machine

Every station runs the same six phases, in order:

```
spec → review → manufacture → audit → reflect → checkpoint
```

- **Specify** explores the context the station needs and decomposes the work into
  Units with testable completion criteria. Nothing is produced yet — the goal is
  to know exactly what "done" means before spending anything making it.
- **Review** challenges the spec before any output exists. It is cheaper to reject
  a bad scope here than to discover it after the work is built.
- **Manufacture** is the Pass loop. Each Unit runs Passes; one Pass is Make →
  Challenge → Resolve. This is where output is actually made.
- **Audit** verifies the produced output against the spec — independently of the
  workers that made it — and folds in the quality gates: the tests, types, lints,
  and evidence. There is no separate tests phase; the checks live here.
- **Reflect** is the autonomous retrospective. The station looks back at how the
  work actually went and feeds that into the run-level reflections, so the line
  learns instead of repeating mistakes.
- **Checkpoint** fires the station's gate and locks the durable artifact. Passing
  advances the line; failing routes the rework back as drift. Once locked,
  downstream stations may not reopen it.

## Checkpoints and gate types

The checkpoint is the human's control point — not per-tool prompts, the gate. The
agent runs hot inside the station and meets you here. Four gate types set how much
attention a station earns:

| Gate         | Behavior                                                          |
| ------------ | ----------------------------------------------------------------- |
| **auto**     | Low-risk station. The manager advances without stopping.          |
| **ask**      | Pull a human in. Surface the artifact and wait for a decision.    |
| **external** | A sign-off that happens elsewhere — a PR review, a release approval. |
| **await**    | A long-running task the manager waits on before it can advance.   |

:::callout warn
You set the **mode** once for the whole Run — `team`, `solo`, or `dark`. In
`dark` mode the line runs unattended (gates resolve `auto`); in `team`/`solo` it
holds for review at every station.
:::

## The three-track priority

When you request changes or leave feedback, the manager does not just queue it. It
runs a fixed priority over three tracks every tick, highest first:

1. **Drift** — locked work that has moved out from under the spec. Repaired first,
   because everything downstream is standing on it.
2. **Feedback** — your annotations on Units, routed to the fix-workers with their
   severity attached.
3. **Run** — normal forward progress through the stations.

Severity steers the gate: a **must** blocks the checkpoint, a **should** is
expected before lock, a **nit** is advisory. So your review is not a comment box —
it is an input the manager schedules against, ahead of new work.

## Where to go next

- **[Workflows](/workflows)** — the loop turned into a practical catalog of
  commands.
- **[The big picture](/big-picture)** — why this shape, and where it is heading.
- **[Methodology](/methodology)** — the cost-of-late-discovery argument in depth.
