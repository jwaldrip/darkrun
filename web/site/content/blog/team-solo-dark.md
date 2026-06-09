# Team, solo, dark

darkrun has one dial that decides how much it stops to talk to you. It's global, set once per run. Not a setting per station, not a checkbox grid. One value: **team**, **solo**, or **dark**.

The dial controls where you sit relative to the run. In the loop with your team. In the loop alone. On the loop.

:::keypoints title="The three modes"
- **team** — in the loop with your team; each gate opens a PR they review and merge.
- **solo** — in the loop alone; each gate asks for local review in the desktop app.
- **dark** — on the loop; the run advances, pausing only on external/await gates and real ambiguity.
:::

## The three modes

| Mode | You are... | Each station's gate |
|---|---|---|
| **team** | in the loop, with your team | opens a PR they review and merge |
| **solo** | in the loop, alone | asks for local review in the desktop app |
| **dark** | on the loop | runs through, stops only on real ambiguity |

**team** is for shared work. When a station finishes, it opens a pull request. Your team reviews it and merges it the way they review anything. The gate is external — it lives in your existing PR flow, and the run waits for the merge. The people who own the code stay the people who approve the change.

**solo** is you and the machine. Each station finishes and asks for review locally. You approve it in the desktop app and it advances. No PR overhead, no waiting on anyone, but you still see every station boundary before the run crosses it.

**dark** is lights-out. You pre-elaborate the whole run up front, then it goes station to station without stopping. It pauses for two things only: a gate that's genuinely external or awaiting something it can't produce, and real ambiguity it can't resolve from the frame. Otherwise you're on the loop, not in it. You set the direction once and read the result.

## All three pre-elaborate

This is the part that makes the dial safe to turn. Every mode does the elaboration work up front. The run thinks through the whole job before it starts building, in team and solo and dark alike.

:::callout
Dark front-loads the thinking instead of skipping it: plan the whole job first, then run without interrupting me for it. Dark and solo plan with the same care. They differ on one thing — whether the run stops to show you each plan as it goes, or shows you the whole shape once and then executes.
:::

## Why one global dial

The tempting design is a gate setting per station. Ask on Frame, auto on Build, ask on Harden. It sounds flexible. It's a trap.

Per-station gates make the run unpredictable. You can't answer "will this stop and ask me?" without opening the config and reading six settings. You end up either babysitting a run you thought was autonomous, or walking away from one that was about to block on you. The altitude you're working at keeps shifting under you.

One global dial fixes the altitude. You pick it once and the whole run honors it. In team, every boundary is a PR — you know that going in. In dark, nothing stops except ambiguity — you know that too. You spend your attention at one consistent level instead of context-switching every time a different station decides to handle gates its own way.

```
team   →  external gate   (PR review + merge)
solo   →  ask gate        (local desktop approval)
dark   →  auto gate       (advance; pause only on external/await + ambiguity)
```

The gate kind comes from the mode now, not the station. team makes every gate external, solo makes every gate ask, dark makes every gate auto. The station says what it does; the mode says how much it checks in.

## Pick the altitude, then leave it

A run is a long thing. The cost of the wrong gate model isn't one bad decision, it's a hundred small ones spread across every station boundary, each one either interrupting you when you wanted flow or sliding past when you wanted a look.

Set the dial to the altitude you actually want to work at.

:::columns
- Shipping with your team: **team**.
- Heads-down by yourself: **solo**.
- Specs in, software out: **dark**.
:::

Then stop thinking about gates and let the run honor the choice.
