# Getting started

darkrun is a **dark factory harness**: it runs your agents lights-out as an
ordered line of stations that take work from raw intent to a shipped, hardened
outcome. You drive the line; the manager keeps every station honest.

:::callout
darkrun runs **inside your agent**, not beside it. You install it as a plugin,
then start runs with a slash command — the agent does the work, the manager
keeps it honest, and you show up at the checkpoints.
:::

## Install (in your agent)

In Claude Code, add the marketplace and install the plugin:

```
/plugin marketplace add darkrun-ai/darkrun
/plugin install darkrun
```

That registers the darkrun **MCP server** (a single native Rust binary — no JS
runtime) and the `/darkrun:*` commands. Other harnesses — Cursor, Gemini,
Codex — wire up the same way; see [Other harnesses](/docs/other-harnesses).

## Start a run

Describe what you want. The manager scaffolds a right-sized run and starts
walking the line:

```
/darkrun:darkrun-new "add rate limiting to the public API"
```

:::steps
- **darkrun-new** scaffolds the run and opens the first station.
- The **manager** advances each station through its phases, doing the work.
- You **review in the desktop app** — approve, request changes, or annotate.
- At each **checkpoint** the run either advances on its own or stops and asks you.
:::

The everyday commands: `darkrun-resume` to advance, `darkrun-inspect` to see
state, `darkrun-checkpoint` to decide a gate. The full surface is in
[Tools and commands](/docs/tools-and-commands).

## The shape of a run

A **Run** moves through six stations, in cost-of-late-discovery order:

| Station  | What it locks                                  |
| -------- | ---------------------------------------------- |
| Frame    | the problem, its value, and why it is worth it |
| Specify  | the contract — what "done" means               |
| Shape    | the design, de-risked with a throwaway spike   |
| Build    | the implementation, unit by unit               |
| Prove    | the evidence — tests that pin behavior         |
| Harden   | the release — the externally-signed checkpoint |

Each station runs the same internal phase machine: **spec → review →
manufacture → audit → reflect → checkpoint**. Audit folds in the quality checks;
reflect is a quick retrospective before the gate. You only stop where a
station's **Checkpoint** asks you to.

## Without an agent (the CLI)

The same engine runs headless if you'd rather drive it directly:

```sh
npm i -g darkrun          # the native binary, no JS app
darkrun run "add rate limiting to the public API"
```

`run` opens a top-level Run and starts the manager loop. Most people use the
agent path above; the CLI is there for scripts and CI.
