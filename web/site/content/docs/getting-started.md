# Getting started

darkrun is a **dark factory harness**: it runs your agents lights-out as an
ordered line of stations that take work from raw intent to a shipped, hardened
outcome. You drive the line; the manager keeps every station honest. The
**software factory** below is the first factory it ships.

## Install

darkrun ships as a single binary. Drop it on your path and point it at a repo:

```sh
darkrun init
darkrun run "add rate limiting to the public API"
```

`init` writes a `.darkrun/` directory with your factory selection and worker
config. `run` opens a top-level **Run** and starts the **manager** — the loop
that advances each station through its phases.

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
manufacture → audit → reflect → checkpoint**. Audit folds in the quality checks
(no separate tests phase); reflect is a quick retrospective before the gate. You
only stop where a station's **Checkpoint** asks you to.

## Drive it

Most of the time you watch the line move and answer at the checkpoints. When a
station produces a **Unit**, you can open it, leave inline feedback, and send it
back for another **Pass**. The **fix-workers** pick up drift and feedback without
restarting the station.
