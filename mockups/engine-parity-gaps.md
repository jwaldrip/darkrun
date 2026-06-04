# Engine Parity — what darkrun ported, and what it missed

A full read of the predecessor engine (its `packages/haiku/src`, ~85K LOC) against
darkrun's current engine. Every row is verified against darkrun source. The
authoritative, machine-readable ledger is **`engine-parity-gaps.csv`** (one row per
mechanism, with a comment column); this doc is the prose companion.

> Vocabulary: "the predecessor" is the prior TypeScript system. Never write its
> name into darkrun code, content, or output.

---

## ◆ Current status — 2026-06-04

**44 mechanisms. 27 built · 17 intentionally not built (4 deliberate-design · 5
redundant · 8 deferred). Every confident gap is closed** — nothing is left on the
"should build" list. What remains is either a deliberate design difference, a
genuine redundancy, or a deferred item waiting on an integration that isn't here
yet.

Start of the work: 3 present · 15 partial · 26 missing → now: 27 built, the rest
dispositioned with a reason.

### Built (27)

- **Cluster A — records carry the story (A1–A7, all done).** Iterations are an
  append-only array with a handoff `note` + `completed_at`; `pass`/active-worker are
  derived from the array (`darkrun_unit_iterate` records each beat and the next
  dispatch shows the prior handoff). Feedback gained `origin` (8 variants),
  `closure_reply`, and `invalidates` (a close re-opens exactly the stamps it
  undercut, re-firing the gate). Append-only `action-log.jsonl` audit journal per run.
- **Cluster B — immune system.** B1 input/premise drift (the sweep witnesses + checks
  declared inputs, files `DriftKind::Input`); B3 per-(unit,input) dedup; B4 cascade
  breaker (`DARKRUN_DRIFT_CASCADE_CAP`); B8 deadlock/churn halt (pre-existing);
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

### Deliberate design — not a gap (4: keep)

- **B2** revert-self-heal + explicit `accept()` over auto-restamp-on-detect (the
  stronger model for darkrun's filesystem-truth design).
- **B6** the hook suite + `drift-witness.log` over an FSM checksum sidecar.
- **C2** the fixed six FSSBPH stations (the whole orientation refactor rests on the
  invariant spine; right-sizing collapses at run start).
- **C6** delivery verification — the discrete gate already polls PR merge; a
  non-discrete run has no PR to re-audit.

### Redundant in darkrun's model (5: skip)

- **B7** dispatch leases — the deadlock halt + automatic `Pending` re-dispatch already
  recover an abandoned unit (backstopped).
- **E4** `run_quality_gates` — subsumed by D1's Audit gate, which already forces every
  declared gate to be recorded.
- **E6** `applies_to` globs — reviewers are per-station rosters, so you scope by simply
  not listing a reviewer where it doesn't apply.
- **G3** clarifications — already captured in the run doc / annotations.
- **G5** persisted session metadata — sessions are in-memory by design; the on-disk run
  state is the durable truth.

### Deferred — real, but waiting on a trigger / integration (8)

- **B5** verifier nonce — do alongside a future seal/verifier split; low value until a
  gate is shown gameable.
- **C5** run-level mode shaping — bundle with the C4 run-level gate.
- **D5** proof upload to PR/MR — hosting integration, orthogonal to engine correctness.
- **F2** human_write — build when a "save this for me" UX appears.
- **F5** session presence grace — polish for a remote-review path that isn't here.
- **G1** plugin_version + migration — worth stamping the version early; migrators later.
- **G2** external_refs — additive field; add when integrations land.
- **G4** draft-PR lifecycle — extend `pr_ref` with a status field when the discrete
  path matures.

---

## Disposition of every line (authoritative — mirrors the CSV)

**Cluster A** — A1–A7 all **done**.

**Cluster B** — B1 **done** · B2 **keep** (deliberate) · B3 **done** · B4 **done** ·
B5 **defer** · B6 **keep** (deliberate) · B7 **skip** (backstopped) · B8 **done**
(pre-existing) · B9 **done** (per-unit + per-fix isolation).

**Cluster C** — C1 **done** · C2 **keep** (deliberate) · C3 **done** · C4 **done** ·
C5 **defer** · C6 **keep** (mostly redundant).

**Cluster D** — D1 **done** · D2 **done** · D3 **done** · D4 **done** · D5 **defer**.

**Cluster E** — E1 **done** · E2 **done** · E3 **done** · E4 **skip** (subsumed by
D1) · E5 **done** (runtime input coverage) · E6 **skip** (per-station rosters) ·
E7 **done** (compound gates).

**Cluster F** — F1 **done** (pre-existing) · F2 **defer** · F3 **done**
(pre-existing) · F4 **done** (parallel review stamp) · F5 **defer**.

**Cluster G** — G1 **defer** · G2 **defer** · G3 **skip** (redundant) · G4 **defer** ·
G5 **skip** (in-memory by design).

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
| Drift on **premises** (inputs), not just outputs | sweeps witnessed input files + dir inventory | **done** — `drift.rs` sweep + `DriftKind::Input` + `input_witnesses` |
| Cross-sweep **dedup** | `source_ref` skip-if-open | **done** — `drift_id_for` + per-(unit,input) id |
| **Cascade alarm** breaker | stop filing at ≥10 open | **done** — `cascade_cap` / `DARKRUN_DRIFT_CASCADE_CAP` |
| Witness **restamp on detect** | restamp before filing | **keep** — darkrun reverts-to-self-heal + explicit `accept()` instead |
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
