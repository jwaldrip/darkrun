# Engine Parity — what darkrun ported, and what it missed

A full read of the predecessor engine (its `packages/haiku/src`, ~85K LOC) against
darkrun's current engine. Every row is verified against darkrun source. The
authoritative, machine-readable ledger is **`engine-parity-gaps.csv`** (one row per
mechanism, with a comment column); this doc is the prose companion.

> Vocabulary: "the predecessor" is the prior TypeScript system. Never write its
> name into darkrun code, content, or output.

---

## ◆ Current status — 2026-06-05

**44 mechanisms. 38 built · 6 intentionally not built (3 deliberate-design · 3
redundant). Nothing is deferred and nothing is left on the "should build" list** —
every actionable gap is closed, including the formerly-deferred batch (B5, C5,
D5, F2, F5, G1, G2, G4) and the items reopened from the spreadsheet review (E6,
the run-main-vs-default tracking, and the B7 in-flight verification). What remains
is only deliberate design difference (B6, C2, C6) or genuine redundancy (E4, G3,
G5).

> **This session's deferred-batch close (2026-06-05).** B5 verifier nonce ·
> C5 run-level mode shaping · D5 proof→PR upload · F2 human_write (a guarded
> file write that auto-triggers drift) · F5 presence grace (lost-vs-closed) ·
> G1 schema version (versioned separately from the plugin, per review) ·
> G2 external_refs · G4 draft-PR lifecycle + run-main-vs-default status · plus
> **E6** applies_to (surface-scoped reviewers, reopened from "skip" per review)
> and a **B7** verification that the wave never re-picks an in-flight unit. A
> priority-aware tool-budget cut was added along the way so a growing tool
> surface can't shed essential loop drivers.

> **Drift rebuilt (B1/B2).** Drift was reworked this session to the verified
> predecessor model after a deeper read: it now witnesses **inputs only** (not
> outputs), exempts the same-station input==output **baton**, **restamps on
> detect**, and routes a premise change as scoped **`origin=drift` feedback**
> that re-orients the affected station — replacing the loop-prone output-witness
> + global-hold model. The materiality loop is closed (`darkrun_feedback_set_targets`
> → invalidate → re-sign), and the retired drift track's dead code was fully
> removed across every crate (no dormant types). B2 (restamp-on-detect) moved
> from "keep" to **done**.

Start of the work: 3 present · 15 partial · 26 missing → now: 38 built, the rest
deliberate-design or redundant.

### Built (38)

The earlier 28, plus this session's deferred-batch + review-driven close:

- **B5** verifier nonce — `Station.verifier_nonce` minted at Manufacture dispatch;
  `darkrun_quality_gate_record` refuses a result without the matching token.
- **B7** dispatch-lease guarantee **verified** — `wave_ready` (Pending-only) drops a
  unit the moment it goes InProgress; an in-flight dep never releases its dependent.
- **C5** run-level mode shaping — a right-sized run skips the whole-run review; the
  final seal gate is honored regardless of mode.
- **D5** proof→PR upload — the discrete gate posts the station's proof as a PR/MR
  comment on open (`Hosting::comment` via gh/glab).
- **E6** `applies_to` surface scope — a reviewer fires only on a matching surface
  (software ships an `accessibility-auditor` scoped to web_ui/desktop/mobile).
- **F2** human_write — a guarded operator file write whose writes auto-trigger drift
  when they touch a premise.
- **F5** presence grace — Live → Lost (grace) → Closed, so a blip ≠ a close.
- **G1** schema version — `schema_version` versioned separately from the plugin, with
  the on-read migrator hook.
- **G2** external_refs — ticket/PR/design handles on the run.
- **G4** draft-PR lifecycle (draft→ready→merged + timestamps) **and** run-main-vs-
  default status (`run_main_status`, surfaced on `run_show`).

### Built — the earlier 28

- **Cluster A — records carry the story (A1–A7, all done).** Iterations are an
  append-only array with a handoff `note` + `completed_at`; `pass`/active-worker are
  derived from the array (`darkrun_unit_iterate` records each beat and the next
  dispatch shows the prior handoff). Feedback gained `origin` (8 variants),
  `closure_reply`, and `invalidates` (a close re-opens exactly the stamps it
  undercut, re-firing the gate). Append-only `action-log.jsonl` audit journal per run.
- **Cluster B — immune system.** **B1 input-premise drift, rebuilt** — inputs-only
  witnessing, same-station baton exemption, restamp-on-detect, and a moved premise
  routed as scoped `origin=drift` feedback that re-orients the affected station
  (outputs are never witnessed); **B2 restamp-on-detect** (moved from "keep" to done);
  B3 dedup (one drift feedback per premise+kind); B4 cascade breaker
  (`DARKRUN_DRIFT_CASCADE_CAP`); B8 deadlock/churn halt (pre-existing);
  **B9 per-unit AND per-fix worktree isolation** — each unit's Pass-loop and each
  drift/feedback fix forks onto its own branch + worktree off the station branch and
  lands back on lock/resolution.
- **Cluster C — cursor richness.** C1 collaboration backpressure (a dedicated
  `collaborative` mode hard-holds Spec until `darkrun_elaborate_seal`); C3
  severity-ordered feedback (questions preempt; blocker before nit); **C4 run-level
  review** (a whole-run cross-station audit gates before seal via
  `darkrun_run_review_stamp`).
- **Cluster D — gate & proof depth.** D1 quality-gate execution (declared gates must
  be `pass`/deferred-to-CI before Audit; env-block auto-defers); D2 output-existence
  gate; D3 input-shape gate; **D4 cross-station input coverage** (every upstream
  locked artifact must be carried forward in `inputs` or consciously `inputs_waived`).
- **Cluster E — content expressiveness.** E1 per-role model; E2 reviewer
  interpretation (lens/strict); E3 worker plan/build/verify reject-routing (a reject
  bounces to the nearest preceding build worker); **E5 runtime input coverage** (the
  cursor holds before Manufacture if the decomposition drops a carried input the
  plan produces — the runtime complement to D4's template check); **E7 compound
  gates** (a station offers a choice of checkpoint paths; the operator picks via
  `darkrun_checkpoint_choose`).
- **Cluster F — agent safety.** F1 guard-workflow-fields + the 8-hook suite
  (pre-existing); F3 stamp-agent-write → `drift-witness.log` (pre-existing);
  **F4 per-role review stamp** (`darkrun_review_stamp` records one role without a
  cursor walk, so reviewers/explorers fan out in parallel and the parent ticks once).

### Deliberate design — not a gap (3: keep)

- **B6** the hook suite + `drift-witness.log` over an FSM checksum sidecar.
- **C2** the fixed six FSSBPH stations (the whole orientation refactor rests on the
  invariant spine; right-sizing collapses at run start).
- **C6** delivery verification — the discrete gate already polls PR merge; a
  non-discrete run has no PR to re-audit.

### Redundant in darkrun's model (3: skip)

- **E4** `run_quality_gates` — subsumed by D1's Audit gate, which already forces every
  declared gate to be recorded.
- **G3** clarifications — already captured in the run doc / annotations (review
  confirmed: "fine with whatever mechanism as long as it's captured in elaboration").
- **G5** persisted session metadata — sessions are in-memory by design; the on-disk run
  state is the durable truth (review confirmed skip).

*(Nothing is deferred — the formerly-deferred B5/C5/D5/F2/F5/G1/G2/G4 are now built,
and the formerly-skipped B7 and E6 were verified/built per the spreadsheet review.)*

---

## Disposition of every line (authoritative — mirrors the CSV)

**Cluster A** — A1–A7 all **done**.

**Cluster B** — B1 **done** (rebuilt: inputs-only + baton + restamp→feedback) ·
B2 **done** (restamp-on-detect) · B3 **done** · B4 **done** ·
B5 **done** (verifier nonce) · B6 **keep** (deliberate) · B7 **done** (in-flight
exclusion verified) · B8 **done** (pre-existing) · B9 **done** (per-unit + per-fix
isolation).

**Cluster C** — C1 **done** · C2 **keep** (deliberate) · C3 **done** · C4 **done** ·
C5 **done** (run-level mode shaping) · C6 **keep** (mostly redundant).

**Cluster D** — D1 **done** · D2 **done** · D3 **done** · D4 **done** · D5 **done**
(proof→PR upload).

**Cluster E** — E1 **done** · E2 **done** · E3 **done** · E4 **skip** (subsumed by
D1) · E5 **done** (runtime input coverage) · E6 **done** (surface-scoped reviewers) ·
E7 **done** (compound gates).

**Cluster F** — F1 **done** (pre-existing) · F2 **done** (human_write) · F3 **done**
(pre-existing) · F4 **done** (parallel review stamp) · F5 **done** (presence grace).

**Cluster G** — G1 **done** (schema version) · G2 **done** (external_refs) · G3
**skip** (redundant) · G4 **done** (PR lifecycle + run-main-vs-default) · G5 **skip**
(in-memory by design).

---

## Reference — the gap clusters by theme

The predecessor-side mechanics, kept as reference. Severity: ★★★
correctness/durability · ★★ behavior · ★ expressiveness/polish. Current darkrun
status is in the disposition list above and the CSV.

### A. Records carry the story ★★★ (where the conversation started) — DONE

Iterations were verdict-only; now they're an append-only array carrying the *why*:
a handoff `note` on advance, a reason on reject, `completed_at` per beat, with
`pass`/worker derived from the array. Feedback carries `origin`, `closure_reply`,
and `invalidates`. A per-run `action-log.jsonl` records how the run actually walked.

### B. The self-healing immune system ★★★ — DONE / deliberate

| Mechanic | Predecessor | darkrun now |
|---|---|---|
| Drift on **inputs** (premises), NOT outputs | witnesses input files; outputs are not drift | **done (rebuilt)** — inputs-only witness; a moved premise → one `origin=drift` feedback that re-orients the affected station |
| **Baton exemption** (input==output in one station) | suppress a same-stage produced premise | **done** — `produced_basenames_by_station`; an in-place edit no longer self-drifts |
| Cross-sweep **dedup** | `source_ref` skip-if-open | **done** — one open drift feedback per premise+kind |
| **Cascade alarm** breaker | stop filing at ≥10 open | **done** — caps open drift feedback; restamp still runs |
| Witness **restamp on detect** | restamp before filing | **done (rebuilt)** — restamp-on-detect fires once; the feedback owns the unresolved re-orientation |
| **FSM checksum** sidecar | `.fsm_checksum` tamper detect | **keep** — hook suite + `drift-witness.log` cover it |
| **Verifier nonce** | one-time token required by seal | **defer** — no seal/verifier split to bind to yet |
| Dispatch **leases** + recovery | `dispatched_at` + TTL recovery | **skip** — deadlock halt + `Pending` re-dispatch backstop it |
| **Deadlock / churn halt** | signature/churn → halt | **done** — `deadlock.rs` (pre-existing) |
| Fix-chain **isolation** + downstream invalidation | fix on isolated branch, merge-gated; revisit clears downstream | **done** — per-unit + per-fix worktree isolation; `invalidates` clears downstream stamps |

### C. Cursor richness ★★ — DONE / deliberate

- **Collaboration backpressure (elaborate)** — **done**, gated to a dedicated
  `collaborative` mode that hard-holds Spec until `darkrun_elaborate_seal`.
- **Severity-ordered feedback** — **done** (questions preempt; blocker before nit).
- **Run-level review** — **done** (`RunReview` holds after the last station until the
  whole-run reviewers stamp, before seal).
- **Optional stations** — **keep** (the fixed six are the point; right-size at start).
- **Run-level mode shaping** — **defer** (bundle with the run-level gate).
- **Delivery verification** — **keep** (discrete gate already polls PR merge).

### D. Gate / proof depth ★★ — DONE / deferred

- **Quality-gate execution** — **done** (declared gates run + recorded; Audit holds on
  `gates_unmet`; env-block auto-defers to CI).
- **Output-existence gate** — **done**. **Input-shape gate** — **done**.
- **Cross-station input coverage** — **done** at the template level (D4:
  `validate_input_coverage`) AND at runtime (E5: the cursor holds if a unit drops a
  carried input the plan produces).
- **Proof upload to PR/MR** — **defer** (hosting integration).

### E. Content-model expressiveness ★ — DONE / redundant

`model` (**done**), `interpretation` lens/strict (**done**), `role: plan|build|verify`
reject-routing (**done**), compound gates `checkpoint: [external, ask]` (**done**),
runtime input coverage (**done**). `run_quality_gates` (**skip** — subsumed by D1) and
`applies_to` globs (**skip** — per-station rosters) are redundant in darkrun's model.

### F. Agent-safety / write path ★★ — DONE / deferred

- **guard-workflow-fields + 8-hook suite** — **done** (pre-existing).
- **stamp-agent-write** → `drift-witness.log` — **done** (pre-existing).
- **review_stamp** — **done** (`darkrun_review_stamp`: a subagent records one role
  without a cursor walk, so reviewers/explorers fan out in parallel).
- **human_write** — **defer**. **Session presence grace** — **defer** (presence +
  heartbeat exist; grace windows are polish).

### G. Lifecycle / state extras ★ — deferred / redundant

`plugin_version` + migration (**defer**), `external_refs` (**defer**), draft-PR
lifecycle (**defer**); `clarifications` (**skip** — in the run doc / annotations) and
persisted session metadata (**skip** — in-memory by design).

---

*Authoritative ledger: `engine-parity-gaps.csv`. This doc and the
`engine-gaps-comparison.html` view are kept in sync with it.*
