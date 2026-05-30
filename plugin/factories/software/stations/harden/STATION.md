---
name: harden
description: Harden for production — security, operability, and a rollout plan, scoped to what the change actually needs.
explorers: [threat, operability]
workers: [hardener, red_teamer, releaser]
reviewers: [security, readiness]
checkpoint: external
locked_artifact: release.md
inputs: [frame.md, spec.md, design.md, proof.md, code]
---

# Harden

Harden is the final station. It kills the defect that only appears in production:
**works-in-dev, dies-in-prod**. Proven software can still fall over under real
load, leak under attack, or be impossible to operate, observe, or roll back.
Harden closes that gap — scoped to what the change actually touches, not a blanket
checklist.

## Risk class eliminated

*Production risk.* The software is proven correct in the lab but is insecure,
unobservable, un-rollback-able, or falls over under real traffic. The failure
shows up where it costs the most: in front of real users.

## What this station produces

- **Hardening** — the security and resilience fixes the threat surface demands:
  authz, input handling, secrets, rate limits, failure modes.
- **Operability** — the observability, alerting, and runbook needed to run and
  debug this in production.
- **Rollout plan** — how it ships, how it is verified live, and how it rolls back
  if it goes wrong.
- **Residual risk** — what is knowingly not addressed and why that is acceptable.

## The pass-loop

- **Hardener** addresses the threat surface and resilience gaps the Explorers found, scoped to the change.
- **RedTeamer** attacks the hardened system like an adversary or a bad day in prod: abuse, overload, partial failure, missing dependency.
- **Releaser** writes the rollout and rollback plan, the runbook, and the residual-risk statement.

## Locked artifact

`release.md` — hardening performed, residual risk accepted, and the rollout /
runbook. This is the production sign-off record.

## Checkpoint

**external.** Shipping to production is a human decision made outside the factory
(a release approval, a PR merge, a deploy sign-off), so Harden hands off to an
external surface and **awaits** the go/no-go.
