---
station: frame
phase: post
created_at: 2026-06-10T06:43:40.246038+00:00
---
# Frame — outcome (post-execution)

**Verdict: PASS.** Frame killed *wrong-thing* and locked `frame.md` as the inherited frame.

## What the station produced
`frame.md` — Problem, User, Value, a single third-party-observable Success metric, Non-goals, and a dead-TS→darkrun **Reconciliation table** so later stations can't port the brief literally. Three Units, all completed through the full make→challenge→resolve Pass loop:
- `frame-protocol-problem` — the protocol-fidelity problem/user/value + single metric (a no-knowledge agent reaches `RunAction::Sealed` from emitted prompts alone, `deadlock::check` never fires, walk persisted + replayed; green = protocol flowed, never "it compiles").
- `frame-reconciliation-bound` — the 8-row reconciliation table, each row citing a real Rust seam; the load-bearing row (no `<subagent>`/`next_subagent_dispatch_block` relay) verified against the code by the feasibility reviewer.
- `frame-scope-nongoals` — full-brief scope locked (operator decision), 4 non-goals, protocol-green separated from build-green.

## Evidence
- **Pre-execution review:** value + feasibility review-stamped, zero concerns.
- **Audit approval:** value + feasibility approval-stamped all three Units; feasibility independently re-verified the cited seams hold against the Rust.
- **Checks:** `frame.md` present (87 lines, non-empty), zero placeholders/holes, all required sections present.

## Operator decisions baked in
Full brief (all four phases); the deliberately-dumb agent runs **locally at record time** to build the walk, steps persisted, the persisted result deployed for static replay.

## Process notes for the engine (dogfood)
1. Frame Spec/Review prompts referenced `darkrun_knowledge_record` / `darkrun_brief_record` while the then-running binary didn't expose them (stale-binary skew, bundled to a bug report). The binary has since been rebuilt and now exposes them — this very brief was recorded with `darkrun_brief_record`.
2. The Manufacture Pass loop required a manual `darkrun_unit_update status=completed` to lock each Unit; the prompt names no lock step, so the cursor sat in a `noop` loop until set by hand. Both are exactly the unfollowable-prompt / missing-breadcrumb class darkrun-sim exists to catch.
