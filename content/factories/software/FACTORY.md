---
name: software
description: The software factory — turns a raw run into shipped, hardened software through six risk-eliminating stations.
category: engineering
default_model: sonnet
stations: [frame, specify, shape, build, prove, harden]
fix_workers: [builder, reconciler, validator]
reviewers: [integration-auditor, regression-auditor, security-auditor]
reflections: [architecture, process, quality, velocity]
---

# Software Factory

The software factory delivers working, hardened software from a raw **Run**. Its
stations are organized by **class-of-risk-eliminated**, ordered by
**cost-of-late-discovery**: the earlier a station sits, the cheaper the defect it
catches and the more expensive that same defect becomes if it slips past.

Every station runs the same universal slot:

```
Explore -> Decompose -> Pass-loop(Make -> Challenge -> Resolve) -> Review -> Checkpoint -> Lock
```

- **Explore** — the station's Explorers gather only the context this station needs.
- **Decompose** — split the work into Units with testable completion criteria and a dependency DAG.
- **Pass-loop** — each Unit runs Passes; one Pass is the three-beat worker sequence Make -> Challenge -> Resolve.
- **Review** — Reviewers verify output against criteria, independent of the Workers that produced it.
- **Checkpoint** — a gate (auto / ask / external / await) advances the station or routes rework back as drift.
- **Lock** — the station's durable artifact is persisted; downstream stations may not reopen it.

## The six stations

| Station | Risk class it eliminates | Locked artifact | Checkpoint |
|---|---|---|---|
| **Frame** | building the *wrong thing* | `frame.md` | ask |
| **Specify** | *ambiguity* in what "done" means | `spec.md` | ask |
| **Shape** | *expensive structural reversal* | `design.md` + spike results | ask |
| **Build** | *implementation defects* | merged green code per Unit | auto |
| **Prove** | *escaped defects* (independent of Build) | `proof.md` | ask |
| **Harden** | *works-in-dev-dies-in-prod* | `release.md` | external |

## Right-sizing

At run start the factory assesses size and may collapse or skip stations — a
one-line fix can drop straight to Build -> Prove — and downgrade checkpoints to
auto. There is no manual mode pick; the work decides the shape.

## fix-workers

When a Checkpoint routes rework back as drift or feedback, **fix-workers**
(Builder, Reconciler, Validator) take the repair without re-running the whole
station. They apply the minimal change, reconcile it against the locked artifact,
and validate the fix landed.

## Run-level review and reflection

Stations and their Reviewers each judge one station's output. Two factory-scope
roles judge the **whole Run**, after the final station's checkpoint closes:

- **Run reviewers** (Integration Auditor, Regression Auditor, Security Auditor)
  are whole-Run, cross-station auditors. They run *after* the last station and
  judge the integrated Run end-to-end — the seams between stations, collateral
  damage to untouched flows, and the attacker's view of the complete result.
  Like a station Reviewer, a run reviewer **gates**: an open finding holds the
  Run's close and routes a repair to the fix-workers until the Run is clean.

- **Reflections** (Architecture, Process, Quality, Velocity) run at Run
  completion and produce *learnings*, not verdicts. A reflection never blocks —
  it looks back over the finished Run on one dimension and writes down what would
  make the next Run sharper. The train moves faster next time because the
  reflection improved the track.
