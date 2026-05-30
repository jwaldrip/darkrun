# Lifecycles

A **lifecycle** is the path a unit of work travels through a factory. The
software factory's lifecycle is six stations long, and each station hands its
locked artifact to the next.

## The software lifecycle

```
Frame  →  Specify  →  Shape  →  Build  →  Prove  →  Harden
```

- **Frame** locks the problem and its value.
- **Specify** locks the contract — the definition of done.
- **Shape** locks the design, de-risked by a throwaway spike.
- **Build** locks the implementation, unit by unit.
- **Prove** locks the evidence — tests that pin behavior.
- **Harden** locks the release behind an external sign-off.

## Inside a station

Every station, regardless of what it produces, runs the same lifecycle in
miniature: spec → review → manufacture → audit → reflect → checkpoint. The
audit folds in the quality checks (tests, types, lints) alongside the reviewers,
so there is no separate tests phase; reflect is an autonomous retrospective that
feeds the run-level reflections. The artifact only locks once the audit passes,
the retrospective is captured, and the checkpoint clears.

## Why the order is fixed

The lifecycle is not negotiable per run, and that is the point. A fixed order is
what lets the manager reason about state, lets reviewers know what to check, and
lets you trust that a locked artifact upstream will not silently move under a
station downstream.
