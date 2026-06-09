# Start here

darkrun is a dark factory harness: it runs your agents lights-out as an ordered
line of stations that take work from raw intent to a shipped, hardened outcome.
You drive the line at the checkpoints; the manager keeps every station honest in
between.

This is the fastest path from zero to your first finished Run. Install, start a
Run, walk it to a checkpoint, approve, ship.

:::callout
darkrun runs **inside your agent**. You install it as a plugin and start runs
with a slash command — the agent does the work, the manager keeps it honest.
:::

## Install

In Claude Code, add the marketplace and install the plugin:

```
/plugin marketplace add jwaldrip/darkrun
/plugin install darkrun
```

That registers the darkrun **MCP server** (a single static Rust binary — no
runtime to babysit) and the `/darkrun:*` commands. Cursor, Gemini, and Codex
wire up the same way; see [Other harnesses](/docs/other-harnesses).

## Your first Run, end to end

A **Run** is the top-level execution of a factory against a real task. Describe
what you want and let the manager size the line:

```
/darkrun:darkrun-new "add rate limiting to the public API"
```

Here is the whole loop, start to finish:

1. **Start.** The manager scaffolds a right-sized Run — the software factory's
   stations (Frame → Specify → Shape → Build → Prove → Harden), trimmed to fit
   the work. Small change, short line. Big change, full line.
2. **Pickup.** The agent drives the line with `darkrun-resume`: each tick the
   manager hands it exactly one concrete action — explore this, decompose that,
   run this Pass, attach this proof — and the agent performs it, then ticks again.
   It runs hot inside a station; you are not approving keystrokes.
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
- **[Workflows](/workflows)** — the practical catalog: dark mode, gate-review,
  the annotate-and-rework loop, and the full `/darkrun:*` command surface.
- **[The big picture](/big-picture)** — why a dark factory, and where this is
  heading.
