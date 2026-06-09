# darkrun is a harness

Anthropic published a piece called [Harness Design for Long-Running Application Development](https://www.anthropic.com/engineering/harness-design-long-running-apps). It puts a name to something darkrun already does. A harness is the scaffolding around a model that makes it reliable on work too long to fit in one context window: structured phases, specialized roles, file-based state, and clean handoffs across context resets.

:::callout
darkrun is that, top to bottom. I built it as a harness before the word landed.
:::

## What a harness is

The model is good at the local move. Write this function, fix this test, explain this stack trace. It's bad at the things that span a long job: remembering what it decided three hours ago, keeping the generator honest, knowing when to stop and ask. A harness is the machinery that supplies what the model can't hold on its own.

Anthropic's version has a few load-bearing parts: specialized agent roles, a generator that's separate from the evaluator, communication through files instead of a shared chat buffer, and a way to rebuild state after the context resets. darkrun has a one-to-one mapping for each.

## The mapping

| Harness concept | darkrun |
|---|---|
| The harness itself | A **factory** — the ordered line work runs through |
| Structured phases | **Stations** — Frame, Specify, Shape, Build, Prove, Harden |
| Specialized agent roles | **Workers** running a Make → Challenge → Resolve pass |
| Generator separate from evaluator | **Reviewers** — a distinct evaluator, never the agent grading itself |
| File-based inter-agent state | Everything under `.darkrun/`: `run.md`, `units/*.md`, `state.json` |
| Context reset with structured handoff | `/darkrun:darkrun-resume` rebuilds the cursor from those files |
| "Every component encodes an assumption" | Every station and gate is a guardrail you can remove |

## Workers make, challenge, resolve

A Worker doesn't just emit a draft and move on. It runs a three-beat pass.

:::keypoints title="The three-beat pass"
- **Make** produces the artifact.
- **Challenge** attacks it from a different angle than the one that wrote it.
- **Resolve** reconciles the two into something that survived a real argument.
:::

That's the generator-evaluator split Anthropic names, pulled inside a single station. A model grading its own first draft grades it kindly. Forcing the challenge step means the draft has to survive a critic before it advances.

## Reviewers are a separate evaluator

The harder version of the same rule runs at the station boundary. Reviewers are a different agent than the Worker. The generator never signs off on its own output. When a Reviewer finds something, the finding routes back as feedback the Worker has to clear, and only then does the station's gate get a chance to close.

Generator ≠ evaluator is mechanical here: two different agents with two different jobs, wired so the one that built the thing can't be the one that blesses it.

## State lives in files

There's no hidden run state in a chat buffer. A run is a directory:

```
.darkrun/
  run.md          # the run's frame, decisions, current station
  units/*.md      # each unit of work, its spec and status
  state.json      # the machine-readable cursor
```

This is the part that earns its keep when context resets. Long jobs always blow the window. When that happens, the chat history is gone but the files aren't. `/darkrun:darkrun-resume` reads `state.json`, reconstructs where the cursor sits, and the run keeps going from exactly where it was. No "remind me what we were doing." The harness already knows, because it wrote it down.

## Every component is a removable guardrail

Anthropic's sharpest point: every piece of a harness encodes an assumption about what the model can't do yet, and those assumptions are worth stress-testing. As models get better, scaffolding that was load-bearing becomes dead weight.

darkrun is built so you can test that. Each station retires one class of risk. Each gate is one place the run can stop. If a model gets good enough that the Specify station stops catching ambiguity, you'll see it: the station will start passing through clean every time, and you can cut it. The factory is a set of bets about where things go wrong. When a bet stops paying, you pull it.

## Built as a harness, not retrofitted into one

The reason the mapping is this clean is that darkrun didn't bolt a harness onto a chatbot. The factory is the harness. The stations are the phases. The Workers and Reviewers are the roles. The `.darkrun/` directory is the file-based state. Resume is the context-reset handoff.

Anthropic wrote down the shape of the thing. darkrun is one concrete instance of it, with opinions about where the guardrails go and the cost of late discovery to order them by. Read their post, then read [the stations doc](/docs/stations).

:::callout
It's the same machine, described twice.
:::
