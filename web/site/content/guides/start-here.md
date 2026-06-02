# Start here

darkrun is a dark factory harness: it runs your agents lights-out as an ordered
line of stations that take work from raw intent to a shipped, hardened outcome.
You drive the line at the checkpoints; the manager keeps every station honest in
between.

This is the fastest path from zero to your first finished Run. Install, start a
Run, walk it to a checkpoint, approve, ship.

## Install

darkrun ships as a single static binary — end-to-end Rust, no runtime to babysit.
Drop it on your path and point it at a repo:

```sh
darkrun init
```

`init` auto-detects your VCS, hosting, CI, and default branch, then writes a
`.darkrun/` directory: your factory selection, worker config, and settings. That
directory is the only state darkrun keeps, and it lives in your repo where you can
read it.

## Your first Run, end to end

A **Run** is the top-level execution of a factory against a real task. Describe
what you want and let the manager size the line:

```sh
darkrun run "add rate limiting to the public API"
```

Here is the whole loop, start to finish:

1. **Start.** The manager scaffolds a right-sized Run — the software factory's
   stations (Frame → Specify → Shape → Build → Prove → Harden), trimmed to fit
   the work. Small change, short line. Big change, full line.
2. **Pickup.** Each tick, you ask the manager for the next concrete action and it
   hands you exactly one: explore this, decompose that, run this Pass, attach this
   proof. You perform it, then tick again. The agent runs hot inside a station —
   you are not approving keystrokes.
3. **Checkpoint.** Every station ends at a gate. When the manager reaches one set
   to **ask**, it stops and surfaces the locked artifact for your review. This is
   your control point — not the thousand decisions the agent made to get here, the
   one that matters now.
4. **Approve.** Read the artifact, leave annotations if anything is off, then
   decide. Approve and the line advances to the next station. Request changes and
   the manager routes the rework back as **drift** — no full restart, just the
   fix-workers picking up what you flagged.

That is the entire model. You spend attention at the checkpoints; the manager
spends compute everywhere else.

## Checkpoints, not babysitting

The reason the loop feels calm is the checkpoint. The agent does not stop to ask
permission for every tool call — it runs the station to completion and meets you
at the gate with something worth reviewing. Four gate types decide how much of
your attention a station earns: **auto** (low risk, keep moving), **ask** (pull
me in), **external** (a human sign-off elsewhere), and **await** (a long task
the manager waits on). You set the dial; darkrun honors it.

## Where to go next

- **[How it works](/how-it-works)** — the engine model: Factory > Station > Unit
  > Pass, the run loop, the phase machine, and the gate types.
- **[Docs](/docs)** — install details, the station reference, the review surface,
  and running darkrun in other harnesses.
- **[Workflows](/workflows)** — the practical catalog: autopilot, gate-review,
  the annotate-and-rework loop, and the full `/darkrun:*` command surface.
- **[The big picture](/big-picture)** — why a dark factory, and where this is
  heading.
