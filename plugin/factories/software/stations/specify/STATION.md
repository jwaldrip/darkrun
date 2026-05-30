---
name: specify
description: Specify behavior as testable criteria, contracts, and edge cases — the rubric Prove will grade against.
explorers: [contract, edge_case]
workers: [spec_writer, adversary, tightener]
reviewers: [testability, completeness]
checkpoint: ask
locked_artifact: spec.md
inputs: [frame.md]
---

# Specify

Specify turns the framed problem into an unambiguous contract. It kills
**ambiguity** — the gap between "what we think we agreed to" and "what we
actually agreed to" — before that gap hardens into a design or a defect.

## Risk class eliminated

*Ambiguity risk.* The frame is right, but "done" means something different to
everyone. Two engineers read the same line and build two different things.

## What this station produces

- **Testable acceptance criteria** — every criterion has a yes/no answer an
  independent party could check without asking the author what they meant.
- **Contracts** — the shapes that cross boundaries: inputs, outputs, errors,
  invariants, the data that flows in and out.
- **Edge cases** — the inputs that break naive implementations: empty, huge,
  malformed, concurrent, adversarial.

## The pass-loop

- **SpecWriter** drafts acceptance criteria and contracts from the frame and Explorer findings.
- **Adversary** attacks the spec: finds the ambiguous verbs, the untestable claims, the missing edge cases.
- **Tightener** rewrites every criterion until it is testable — replaces "fast", "handles", "robust" with a concrete, checkable assertion.

## Locked artifact

`spec.md` — testable acceptance criteria + contracts + edge cases. This document
**becomes Prove's rubric**: Prove grades the shipped software against exactly
these criteria, so anything left vague here is unprovable later.

## Checkpoint

**ask.** A human confirms the spec captures the real intent before the factory
commits to designing and building against it.
