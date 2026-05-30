---
name: build
description: Build the software — write tests first, implement to green, self-review, and merge each Unit clean.
explorers: [reuse, integration_point]
workers: [test_author, builder, self_reviewer, reconciler]
reviewers: [correctness, maintainability]
checkpoint: auto
locked_artifact: code
inputs: [frame.md, spec.md, design.md]
---

# Build

Build is where the software gets written. It kills **implementation defects** —
the bugs introduced while turning a sound design into running code. Build works
Unit by Unit, each one landing as merged, green, reviewed code before the next
depends on it.

## Risk class eliminated

*Implementation defects.* The frame, spec, and design are all right, but the code
that realizes them has bugs: off-by-ones, unhandled errors, broken integrations,
regressions in code it touched.

## What this station produces

- **Tests first** — the spec's acceptance criteria become executable tests that
  fail before the implementation exists.
- **Implementation to green** — the minimal code that makes those tests pass.
- **Merged Units** — each Unit reviewed and integrated cleanly, so the next Unit
  builds on a known-good base.

## The pass-loop

- **TestAuthor** translates the spec's criteria into failing tests *before* any implementation — the tests define done.
- **Builder** writes the minimal implementation that turns the failing tests green, following the locked design.
- **SelfReviewer** reviews the diff as a hostile reviewer would: correctness, edge cases, naming, dead code, regressions.
- **Reconciler** integrates the Unit — merges, resolves conflicts, and confirms the full suite stays green.

## Locked artifact

Merged green code per Unit. The durable artifact is the code itself, integrated
and passing. Each Unit's merge is the lock.

## Checkpoint

**auto.** Build advances automatically once tests are green and reviews pass —
the test suite is the gate. Risky diffs (security-sensitive, wide blast radius)
escalate to **ask** for a human look.
