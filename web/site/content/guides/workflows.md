# Workflows

A practical catalog of how people actually drive darkrun. Each one is a short
loop built from the same primitives — Runs, Stations, Units, checkpoints — and
each maps to a real `/darkrun:*` command you can run today.

## Start a Run

Describe what you want and let the manager size the line.

```
/darkrun:start "add rate limiting to the public API"
```

The manager scaffolds a right-sized Run: the software factory's stations, trimmed
to the work. Then you advance it:

```
/darkrun:pickup
```

Each `pickup` returns the next concrete action across stations, workers, and
checkpoints. Perform it, then pick up again. Check where you are any time with
`/darkrun:show` — Stations, Units, completion criteria, and Checkpoint status.

For a one-off you do not want to ceremony up, two shortcuts:

- `/darkrun:quick "..."` — a Run the manager auto-sizes to a single Station, then
  walks it through that Station's phases.
- `/darkrun:zap "..."` — zero-ceremony single-Unit execution: run one task
  straight through a Station's Worker loop with nothing written under `.darkrun/`.

## Autopilot: promote ask gates to auto

When you trust the line and want it to run unattended, promote the **ask**
checkpoints to **auto** so the manager advances Station to Station without
stopping.

```
/darkrun:autopilot
```

It runs the Run's gates autonomously, pausing only on **external** or **await**
gates — the ones that genuinely need something outside the loop — and on real
ambiguity. This is lights-out operation: the floor runs dark, you stay in the
control room.

## Pre-checkpoint gate-review

Before a Checkpoint locks, run a multi-agent code review with a fix loop, so the
artifact that reaches your gate is already clean.

```
/darkrun:gate-review
```

It computes the diff for the Station, dispatches Reviewers against it, and
processes their findings — fixing what it can — before the Checkpoint locks. You
review a stronger artifact, not a first draft.

## Review and annotate: the feedback loop

When you reach an **ask** checkpoint, you decide it:

```
/darkrun:checkpoint
```

Approve to advance the Run, or request changes. The richer loop happens on the
Units. Open a Unit in the review surface — the native desktop app, served locally
over loopback — and leave inline annotations. Every annotation carries a
**severity**:

- **must** — blocks the checkpoint. The Station does not lock until this is fixed.
- **should** — expected before lock, but not a hard block.
- **nit** — advisory; the maker can take it or leave it.

Severity steers the gate. Your feedback is an input the manager schedules against,
not a comment box.

## Drift and rework

When you request changes, the manager does not restart the Station. It routes the
rework as **drift** and the fix-workers pick it up, repairing the locked work
without unwinding everything downstream. The three-track priority decides order
every tick: **drift** first (locked work that moved), then **feedback** (your
annotations), then normal **run** progress.

If a Station is genuinely wedged or you want a clean slate:

- `/darkrun:reset` — wipe one Station (its Units, outputs, artifacts,
  decomposition, feedback, and branch) so the manager re-enters it from scratch.
  Other Stations stay untouched. Can also reset or archive the whole Run.
- `/darkrun:debug` — admin recovery for wedged Runs: preview the manager cursor,
  force a Station complete, reset drift, or patch a feedback record. Every
  mutation asks for explicit confirmation first.

## The backlog

Ideas that are not ready to become a Run live in the parking lot.

```
/darkrun:backlog
```

Add, list, review, or promote a backlog item into a real Run when it is ready.

## The skills and commands surface

The full `/darkrun:*` command surface, grouped by what it does:

| Command                  | What it does                                                       |
| ------------------------ | ----------------------------------------------------------------- |
| `/darkrun:setup`         | Configure darkrun for this project — detect VCS, CI, default branch |
| `/darkrun:start`         | Start a new Run; the manager scaffolds a right-sized lifecycle     |
| `/darkrun:pickup`        | Advance the Run — the manager returns the next concrete action     |
| `/darkrun:show`          | Show the Run's state: Stations, Units, criteria, Checkpoint status |
| `/darkrun:quick`         | Quick single-Station Run, auto-sized to one Station                |
| `/darkrun:zap`           | Zero-ceremony single-Unit execution, no state written             |
| `/darkrun:autopilot`     | Run the gates autonomously by promoting ask to auto               |
| `/darkrun:gate-review`   | Pre-Checkpoint multi-agent review with a fix loop                  |
| `/darkrun:checkpoint`    | Review and decide a Station's Checkpoint                           |
| `/darkrun:backlog`       | Parking lot for ideas not yet ready to become a Run               |
| `/darkrun:factories`     | List available factories and their Stations                       |
| `/darkrun:scaffold`      | Scaffold custom Factories, Stations, Workers, and Reviewers        |
| `/darkrun:reset`         | Wipe one Station so the manager re-enters it from scratch          |
| `/darkrun:debug`         | Admin recovery for wedged Runs (confirmation required)             |
| `/darkrun:migrate`       | Migrate legacy lifecycle state into `.darkrun/`                    |
| `/darkrun:statusline`    | Install the one-line station/phase status line                     |
| `/darkrun:report`        | Submit feedback or a bug report about darkrun itself               |
| `/darkrun:version`       | Show the engine version, plugin version, runtime, and entry point  |
| `/darkrun:changelog`     | Show the changelog / release notes                                 |

Want to build your own line? `/darkrun:scaffold` writes editable templates for
Factories, Stations, Workers, and Reviewers under `.darkrun/` — the recipe is
yours to change.

- **[How it works](/how-it-works)** — the model these commands drive.
- **[Docs](/docs)** — the reference behind each surface.
