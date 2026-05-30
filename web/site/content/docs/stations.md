# Stations and phases

A **Station** is one stage of the factory line. Every station — no matter what
it produces — runs the same six-phase machine. The phases are the same for
Frame and for Harden; only the workers and the locked artifact change.

## The phase machine

1. **spec** — the explorers gather context and the station states its intent.
2. **review** — that intent is checked before any work begins. Don't build the
   wrong thing fast.
3. **manufacture** — the workers run their passes: Make → Challenge → Resolve.
4. **audit** — the reviewers verify the output against the locked spec **and**
   the full quality checks run (tests, types, lints, builds). Judgment plus
   evidence — there is no separate tests phase. A failing check blocks the gate.
5. **reflect** — an autonomous retrospective on what the station's pass taught
   the run. It blocks nothing; its learnings feed the run-level reflections.
6. **checkpoint** — the gate. Auto advances on its own; Ask waits for you;
   External waits for a signal outside the loop; Await holds for a long task.

## Checkpoints

The **Checkpoint** is how a station ends. There are four kinds:

- **auto** — the manager advances with no human in the loop.
- **ask** — the manager stops and asks you to advance or hold.
- **external** — the station waits on an out-of-band approval (a release sign-off,
  a deploy gate).
- **await** — the station parks on a long-running task and resumes when it lands.

## Passes and workers

Inside **manufacture**, a **Worker** runs a **Pass**: it makes a change, a second
worker challenges it, and a resolver reconciles the two. A station can stack
several passes. When you leave feedback or the manager detects drift, the
**fix-workers** run targeted passes instead of restarting the whole station.
