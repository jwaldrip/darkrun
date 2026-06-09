# Checkpoints, not babysitting

The fastest way to waste a person is to make them watch an agent type. Attention is the scarce resource in any run. Every minute spent reading keystrokes is a minute not spent on the one decision that actually changes the outcome.

:::callout
darkrun is built to spend your attention in exactly one place: the checkpoint.
:::

## A checkpoint is a station boundary

A factory run is an ordered line of stations. A checkpoint is the gate at the end of one. The station does its work, and at the boundary the run reaches a gate that decides whether it advances on its own or stops and pulls you in.

That gate is the only place you're asked to look. Not mid-station, not while the Worker is drafting, not while a Reviewer is grading. The work happens; the gate is where it surfaces.

## The gate kind comes from the mode

You don't configure gates per station. You set one global dial for the run, and the gate kind falls out of it.

| Mode | Gate kind | What happens at the boundary |
|---|---|---|
| **team** | external | opens a PR your team reviews and merges |
| **solo** | ask | asks you for local review in the desktop app |
| **dark** | auto | advances on its own; stops only on real ambiguity |

In **team**, every checkpoint is a pull request. The run waits for your team to merge. In **solo**, every checkpoint asks you locally before it advances. In **dark**, checkpoints clear automatically and the run keeps moving, stopping only when it hits something external it's waiting on or genuine ambiguity it can't resolve from the frame.

One dial. The whole run honors it. You know going in how often it'll check in, because you picked the altitude.

## Low-risk work clears without you

The reason this saves attention instead of just rationing it: a checkpoint that has nothing to decide doesn't need you. In dark mode, a station that retired its risk cleanly and produced exactly what the frame asked for advances on the auto gate and you never see it. No notification, no approval, no "looks good" you typed without reading.

```
station finishes ─▶ checkpoint ─▶ gate
                                   │
              auto + clean ────────┼──▶ advance (you never saw it)
              ambiguity / risk ────┴──▶ stop, ask you
```

The decisions that matter stop and ask.

:::keypoints title="The decisions that matter stop and ask"
- A design trade-off the frame didn't settle.
- A release the run won't sign off on its own.
- An ambiguity it can't resolve without you.
:::

Those hit the gate and wait.

So you don't pay attention evenly across the whole run. You pay it where it's load-bearing and nowhere else.

## Watching is not the same as being in the loop

There's a reflex that says staying in control means watching. It's backwards. Watching an agent type gives you no real control — you can't meaningfully intervene keystroke by keystroke, and trying just burns the attention you'd need for the decision that's actually coming.

Real control is being present at the boundary where a choice gets made, and absent everywhere else. That's what the checkpoint gives you. The run does the work. The gate brings you in when there's a decision worth your judgment, and leaves you out when there isn't.

That's the line between driving a run and babysitting one. Babysitting is watching everything and deciding nothing. Driving is deciding at the checkpoints and ignoring the rest. darkrun is built for the second one.

:::callout
Pick your mode. Let the line run. Show up at the gate.
:::
