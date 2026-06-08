# The big picture

There is a moment, the first time you watch an agent build something real, where
the excitement curdles into dread. It is fast. It is also unsupervised, and you
have no idea where it is in the work, what it decided on your behalf, or whether
the thing it just confidently produced is right. So you do the only thing the
tooling lets you do: you sit there and approve every step. You have hired a tireless
worker and made yourself its full-time babysitter.

That is the trap darkrun is built to escape.

## The dark factory

Borrow the metaphor from manufacturing. A **dark factory** is a plant that runs
lights-out — the machines work through the night with no humans on the floor,
because the line was designed well enough that they do not need to be there. The
humans are not gone. They are at the control room, watching the gauges, stepping
in exactly when a gauge goes red. The whole point of the building is that running
it does not require a person standing next to every machine.

Software is ready for the same move. The agents are good enough to run the line.
What has been missing is the line itself — the structure that makes lights-out
operation safe instead of reckless. darkrun is that structure.

## Autonomous agents, gated by humans

The core bet is that you can separate two things people usually conflate: doing
the work, and deciding whether the work is good. The agent does the work — hot,
unattended, the whole station front to back. The human decides — but only at the
**checkpoint**, the one place where a decision actually changes the outcome.

This is the difference between a seatbelt and a chaperone. A seatbelt is friction
you accept at exactly one moment, and it lets you drive fast the rest of the time.
A chaperone is friction at every moment, and it means you never really left the
parking lot. Per-tool approval is a chaperone. The checkpoint is a seatbelt.

:::callout
So the model is simple to say and hard to earn: **checkpoints, not babysitting.**
The agent runs; you gate. Your attention is the scarcest input in the whole
system, and darkrun spends it like it is scarce — at the gates, on the locked
artifacts, where your judgment is the thing that moves the needle.
:::

## One method, many lines

The second bet is that the machine that makes work good is universal, even though
the work is not. Every station in darkrun walks the same phase machine: it
specifies, it reviews the spec, it manufactures through a Make-Challenge-Resolve
loop, it audits the output against the spec with the quality gates folded in, it
reflects, and it locks behind a checkpoint. That loop does not care whether the
station is framing a problem or hardening a release. It is the method.

The **line** is what varies. The software factory orders its stations by the cost
of late discovery — Frame, Specify, Shape, Build, Prove, Harden — because a wrong
assumption found in Frame is cheap and the same assumption found in Harden is a
rewrite. A different factory, for a different kind of work, declares a different
line. One method, many lines. That is what lets darkrun be a harness and not just
a script: the discipline is shared, the recipe is yours.

## The human-agent loop

None of this works if the human's only verb is "approve." So darkrun makes review
a real loop. You read the locked artifact, you annotate it inline, and your
annotations carry **severity** — must, should, nit. That severity steers the gate:
a must blocks the checkpoint, a should is expected, a nit is advice. The feedback
routes back to the fix-workers as **drift**, scheduled ahead of new work, repaired
without restarting the station.

:::keypoints title="Severity steers the gate"
- **must** — blocks the checkpoint until it is resolved.
- **should** — expected before lock, but not blocking.
- **nit** — advisory; routed as drift, never gating.
:::

You are not commenting into a void. You are programming the line with your
judgment, one annotation at a time, and the manager schedules against it.

## Where it is heading

The harness already speaks seven agent surfaces — Claude Code, Codex, Gemini CLI,
Cursor, Windsurf, OpenCode, Kiro — because the method should not be hostage to one
vendor's tool. It ships as a single static Rust binary with a native desktop review
app, because the foundation has to be solid before the speed is worth anything.

Where it goes from here follows the same logic it started with. More factories, for
more kinds of work, all walking the same machine. Tighter feedback between the
reflections and the next Run, so the line gets sharper the more you use it. And
more of the floor going dark — more gates you trust enough to promote to auto —
without ever giving up the control room.

The future of building software is not fewer humans. It is humans who have climbed
into the control room and stopped standing next to the machines. darkrun is how you
get there.

- **[Start here](/start-here)** — install and run your first line.
- **[How it works](/how-it-works)** — the engine model under the metaphor.
