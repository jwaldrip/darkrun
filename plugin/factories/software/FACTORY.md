---
name: software
description: The software factory — turns a raw run into shipped, hardened software through six risk-eliminating stations.
category: engineering
default_model: sonnet
stations: [frame, specify, shape, build, prove, harden]
fix_workers: [builder, reconciler, validator]
reviewers: [integration-auditor, regression-auditor, security-auditor, accessibility-auditor, runtime-verifier]
reflections: [architecture, process, quality, velocity]
surfaces: [library, api, web_ui, tui, cli, desktop, mobile, data]
---

# Software Factory

The software factory delivers working, hardened software from a raw **Run**. Its
stations are organized by **class-of-risk-eliminated**, ordered by
**cost-of-late-discovery**: the earlier a station sits, the cheaper the defect it
catches and the more expensive that same defect becomes if it slips past.

Every station walks the same six-phase machine:

```
Spec -> Review -> Manufacture(Make -> Challenge -> Resolve) -> Audit -> Reflect -> Checkpoint
```

- **Spec** — the station's Explorers gather only the context this station needs, then the work is decomposed into Units with testable completion criteria and a dependency DAG.
- **Review** — Reviewers adversarially review the spec *before* manufacture: a bad spec that reaches the floor is the most expensive failure in the line.
- **Manufacture** — each Unit runs the three-beat worker sequence Make -> Challenge -> Resolve (build -> red-team -> repair). The challenge beat is deliberate adversarial-hardening, not ceremony.
- **Audit** — Reviewers verify the manufactured output against the locked spec **and** run the station's full quality checks (tests, types, lints, builds). There is no separate tests phase: audit gives both judgment (reviewers) and evidence (the checks). A failing check blocks the checkpoint — no "pre-existing" excuses.
- **Reflect** — an autonomous retrospective on what this station's pass taught the run. It blocks nothing; its learnings feed the run-level reflections so the next run is sharper.
- **Checkpoint** — a gate (auto / ask / external / await) locks the station's durable artifact (downstream stations may not reopen it) or routes rework back as drift.

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
