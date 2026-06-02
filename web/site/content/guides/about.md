# About darkrun

darkrun is a dark factory harness: it runs your agents lights-out as an ordered
line of stations that take work from raw intent to a shipped, hardened outcome.
You drive the line at the checkpoints; the manager keeps every station honest in
between. The thesis is one line — **checkpoints, not babysitting** — and the whole
system is built to earn it.

## A ground-up Rust rewrite

darkrun is a ground-up rewrite in Rust, end to end. Not a wrapper, not a port — a
single static binary with no runtime to babysit, a native Dioxus desktop review
app, and a clean engine core that the manager loop drives one action at a time.
The rewrite was the point: the train can only move as fast as the tracks it is
built on, and a harness people trust to run their agents unattended has to stand
on a foundation that does not wobble.

It speaks seven agent surfaces out of the box — Claude Code, Codex, Gemini CLI,
Cursor, Windsurf, OpenCode, and Kiro — because the method should not be hostage to
any one vendor's tool.

## The philosophy

Three ideas hold the whole thing up.

**Autonomous agents, gated by humans.** The agent runs hot inside a station, front
to back. The human's control point is the checkpoint — the one place where a
decision changes the outcome — not the thousand tool calls it took to get there. A
seatbelt, not a chaperone.

**One method, many lines.** The phase machine is universal: every station walks
spec → review → manufacture → audit → reflect → checkpoint. The line is yours: the
software factory orders its stations by cost-of-late-discovery — Frame, Specify,
Shape, Build, Prove, Harden — and other factories declare their own. Shared
discipline, your recipe.

**Review is a real loop.** You read the locked artifact, annotate it inline, and
your annotations carry severity — must, should, nit — that steers the gate.
Feedback routes back to the fix-workers as drift, scheduled ahead of new work,
repaired without restarting the station. Your judgment programs the line.

## License

darkrun is licensed under **FSL-1.1-ALv2** — the Functional Source License,
version 1.1, with an Apache 2.0 future grant. In plain terms: it is source-available
and free to use, modify, and self-host for any purpose that is not building a
competing product, and **two years after each release, that release converts to
Apache 2.0** and becomes fully open source. You get the source today, the freedom
to build with it today, and a hard guarantee that it lands in the commons.

## Links

- **[Start here](/start-here)** — install and run your first line.
- **[How it works](/how-it-works)** — the engine model.
- **[The big picture](/big-picture)** — why a dark factory.
- **[Workflows](/workflows)** — the practical command catalog.
- **[Docs](/docs)** · **[Methodology](/methodology)** · **[Glossary](/glossary)**
