# Engine Parity — what darkrun ported, and what it missed (VERIFIED)

A full read of the predecessor engine (its `packages/haiku/src`, ~85K LOC) against
darkrun's current engine. **Every row below has been verified against darkrun
source** — verdict (HAVE / PARTIAL / MISSING) with `file:line` evidence. The point
is honesty about the gap, not a to-do list yet.

> Vocabulary: "the predecessor" is the prior TypeScript system. Never write its
> name into darkrun code, content, or output.

**Verified tally — 44 mechanisms. Start of work: 3 present · 15 partial · 26 missing.
After this session's burn-down: 16 present · 11 partial · 17 missing.**

## Closed this session (committed to main, each tested)

- **Cluster A — fully closed.** Iterations carry a handoff `note` + `completed_at`;
  `pass`/active-worker derived from the array (`darkrun_unit_iterate` records a
  beat; the next dispatch shows the prior handoff). Feedback gained `origin`,
  `closure_reply`, and `invalidates` (a close re-opens the stamps it undercut).
  Append-only `action-log.jsonl` audit journal per run.
- **Cluster B — B1 input/premise drift** (the sweep now witnesses + checks
  declared inputs, files `DriftKind::Input`); **B3 dedup** (per-(unit,input) id);
  **B4 cascade breaker** (`DARKRUN_DRIFT_CASCADE_CAP`). B8 (deadlock halt) was
  already present.
- **Cluster C — C3 severity-ordered feedback** (questions preempt; blocker before
  nit; no starvation).
- **Cluster D — D2 output-existence gate** (a unit can't reach Audit on a declared
  output that isn't on disk); **D3 input-shape gate** (an `input` that names a
  unit, not a path, is held).

Still open (next): B2/B5/B6/B7/B9, C1/C2/C4/C5/C6, D1/D4/D5, all of E, F2/F4/F5,
all of G. A few are deliberate design differences rather than gaps — see notes.

## The one-paragraph truth (post-verification)

darkrun ported the predecessor's **shape** (three tracks, station phase machine,
four checkpoint kinds, units with iterations, reviews/approvals stamps,
surface-routed proof) and — correcting the first draft — **more of its safety
layer than I first credited**: a full 8-hook agent guard suite, a cursor-level
**deadlock/churn halt** (`deadlock.rs`), per-path drift idempotency, and a
drift-witness log. What's still genuinely absent is the **witness-based drift
immune system** (premise/input drift, restamp-on-detect, verifier nonce, integrity
checksum, dispatch-lease recovery, fix-chain isolation), the **quality-gate
execution engine** (gates are declared in content but the engine never runs them),
and — the cluster that started this — records that carry the **story** (an
iteration has no "why"; feedback has no origin or closure reply).

## Corrections from the first draft (darkrun has more than I credited)

- **B8 deadlock/churn halt → HAVE.** `deadlock.rs:1-354` is a real cursor-level
  detector (HALT_THRESHOLD=4, A↔B churn window=10, persisted to `deadlock.json`),
  on top of the per-unit MAX_PASSES=8 guard (`position.rs:369,839`).
- **F1 guard-workflow-fields → HAVE**, and the hook suite is full: `hooks.json`
  wires 8 hooks (guard-workflow-fields, prompt-guard, workflow-guard,
  stamp-agent-write, context-monitor, redirect-plan-mode, inject-state-file,
  edit-auto-read-hint); the guard exits(2) with a redirect and suspends during
  merge (`cli/src/hook.rs:170-206`).
- **F3 stamp-agent-write → HAVE.** PostToolUse hook appends every agent Write/Edit
  to `drift-witness.log`, read by the drift track (`cli/src/hook.rs:393-431`).
- **B2/B3 drift → PARTIAL, not missing.** Re-witness exists via the explicit
  `accept()` tool (`drift.rs:106-138`); `drift_id_for(path)` makes filing
  idempotent per artifact path (`drift.rs:36-42`).
- **F5 presence → PARTIAL.** WS layer tracks live connections + a heartbeat
  endpoint (`http/state.rs:130-177`); missing only grace windows + reattach.

## Disposition of every remaining line

Each open line is now one of: **CLOSED** (committed this session), **DELIBERATE**
(darkrun chose a different design on purpose — not a gap), or **DEFERRED** (a real
gap, scoped, with why-not-yet). Nothing is left unaccounted for.

**Cluster A — all CLOSED** (A1–A7).

**Cluster B**
- B1 input/premise drift — **CLOSED**. B3 dedup — **CLOSED** (per-premise id).
  B4 cascade breaker — **CLOSED**. B8 deadlock halt — **already present**.
- B2 restamp-on-detect — **DELIBERATE**. darkrun's drift self-heals on revert and
  re-witnesses via the explicit `accept()` tool; auto-restamping on detect would
  destroy revert-self-heal (the witness would chase the mutation). The chosen
  model is stronger for darkrun's filesystem-truth design.
- B6 FSM checksum sidecar — **DELIBERATE**. darkrun covers hookless tamper a
  different way: the `stamp-agent-write` hook logs every agent edit to
  `drift-witness.log` and `guard-workflow-fields` blocks raw writes to managed
  files. A checksum sidecar would duplicate that.
- B5 verifier nonce — **DEFERRED**. darkrun has no verifier-subagent/seal split to
  bind a nonce to yet; the analogue is requiring recorded proof at Audit before
  the checkpoint. Worth doing after the proof-at-Audit gate (relates to D1).
- B7 dispatch leases + recovery — **DEFERRED (low)**. darkrun's model re-dispatches
  `Pending` units automatically and the deadlock halt catches an abandoned
  in-flight unit; a TTL lease would tighten recovery but the wedge is already
  caught. 
- B9 fix-chain isolation + downstream invalidation — **DEFERRED (large)**.
  Per-fix worktree isolation is a real feature; downstream invalidation is partly
  delivered (feedback `invalidates` now re-opens stamps — A6). The worktree-per-fix
  half is scoped but large (git plumbing).

**Cluster C**
- C3 severity-ordered feedback — **CLOSED**.
- C1 multi-signal elaborate loop — **DEFERRED (large)**. The Spec phase prompt
  already choreographs elaborate→discover→decompose; exposing them as concurrent
  cursor signals is the predecessor's own biggest refactor (its GAPS.md §1). High
  effort, behavioral-only gain.
- C2 optional stations — **DELIBERATE**. The six FSSBPH positions are a fixed
  invariant by design (the whole orientation refactor rests on it); right-sizing
  collapses at run-start. A live keep-or-drop gate would reintroduce variable
  spines we deliberately removed.
- C4/C5 run-level reviews + mode shaping at run scope — **DEFERRED**. The content
  model already declares run reviewers/reflections; wiring a run-level review gate
  into the walk (distinct from per-station) is scoped, moderate.
- C6 delivery verification re-audit — **DELIBERATE/PARTIAL**. The discrete gate
  already polls PR merge before advancing; a separate pre-seal re-audit is
  redundant for non-discrete runs (no PR).

**Cluster D**
- D2 output-existence gate — **CLOSED**. D3 input-shape gate — **CLOSED**.
- D1 quality-gate execution — **DEFERRED (large, high value)**. In darkrun's
  architecture the agent runs commands (it has Bash); the gap is structured
  recording + enforcement that gates passed. The right shape is a
  `darkrun_quality_gate_record` tool + a `quality_gates` approval stamp required
  at Audit when the factory declares gates. Scoped; next major item.
- D4 coverage acknowledgement — **DEFERRED (moderate)**. A tool + state to mark
  upstream outputs out-of-scope / covered. Needed only once cross-station input
  coverage is enforced.
- D5 proof upload to PR/MR — **DELIBERATE/DEFERRED**. Proof is stored locally and
  surface-checked; pushing it to a PR asset is a hosting-integration feature,
  valuable but orthogonal to engine correctness.

**Cluster E**
- E1 per-role model — **CLOSED** (resolved at dispatch).
- E2 interpretation lens/strict, E3 role plan/build/verify reject-routing,
  E4 run_quality_gates, E5 structured inputs, E6 applies_to, E7 compound gates —
  **DEFERRED (content-model expressiveness)**. Each is a frontmatter field + a
  cursor/prompt behavior; individually moderate, collectively the next content
  pass. E3's reject-routing partly exists (a reject files feedback to Track B).

**Cluster F**
- F1 guard-workflow-fields, F3 stamp-agent-write — **already present (HAVE)**.
- F2 human_write, F4 review_stamp — **DEFERRED**. Both are real (a guarded
  conversational write tool; a parallel-safe review self-stamp). F4 matters once
  reviewers fan out in parallel.
- F5 session presence grace/reattach — **DEFERRED (low)**. Presence + heartbeat
  exist; grace windows are polish.

**Cluster G** — plugin_version+migration, external_refs, clarifications, draft-PR
lifecycle, persisted sessions — **DEFERRED (lifecycle polish)**. None block engine
correctness; each is additive state. G's fixed-six-stations line is **DELIBERATE**
(see C2).

## What darkrun already has (so we don't cry wolf)

- Three-track manager (drift → feedback → run) and the FSSBPH phase machine.
- All four `CheckpointKind`s (auto/ask/external/await) and the gate-review session.
- 43 MCP tools incl. proof attach/get, drift accept, reflections, annotations,
  direction/question/picker sessions, surface classification, scaffold.
- Drift *accept* + re-witness (for **outputs**), feedback triage (severity, move,
  reject, resolve), a `hooks.json` manifest.
- The data model already carries `input_witnesses`, `reviews`, `approvals`, and a
  `Stamp` type — several fields exist but are **not yet wired** into sweeps/gates.

---

## Gap clusters, by theme. Severity: ★★★ correctness/durability · ★★ behavior · ★ expressiveness/polish

### A. Records carry verdicts, not stories ★★★  (this is where the conversation started)

| Gap | Predecessor | darkrun | Confirmed |
|---|---|---|---|
| Iteration handoff/`reason`/`message` | `iteration.reason` (required on reject) + `message` (v9 forward handoff); `renderPriorHandoff` feeds it to the next worker (`prompts/_helpers.ts:232-367`) | `UnitIteration` has none — only worker/started_at/result/pass | ✅ domain.rs:364 |
| Iteration `completed_at` | stamped at terminal result (`schemas/iteration.ts`) | missing (only `started_at`) | ✅ |
| `pass`/`worker` are derived, not stored | bolt = `iterations.length`; hat = `iterations[-1].hat` (`units.ts:147`) | `pass: u32` and `worker: String` stored on the unit → dual source of truth | ✅ domain.rs:406,410 |
| Feedback `origin` taxonomy (12 values) | adversarial-review, drift, discovery, user-chat, … (`schemas/feedback.ts:35`) | `Feedback` has no origin | ✅ |
| Feedback `closure_reply` (+unread) | what the fixer did, surfaced to requester (`feedback.ts:159`) | missing | ✅ |
| Feedback `targets.invalidates[]` | which approval roles clear on close (`feedback.ts:138`) | invalidation is implicit in code, not persisted | ✅ |
| Audit journals | `action-log.jsonl` + `write-audit.jsonl` + per-stage `decisions.jsonl` | none | ✅ |

The fix here is cheap and self-contained, and it's the thing you already named:
make `UnitIteration` append-only with a `note`/`reason`, derive pass/worker from
the array, and thread the prior note into the next worker's dispatch.

### B. The self-healing immune system — the biggest architectural miss ★★★

| Gap | Predecessor | darkrun |
|---|---|---|
| Drift on **premises** (inputs), not just outputs | sweeps every witnessed input file + dir inventory; detects input mutation/add/delete (`drift-sweep.ts:330-716`) | sweep iterates `outputs` only (`drift.rs:62`); the `input_witnesses` field exists but is never swept |
| Witness **restamp on detect** | restamps witness to current SHA before filing FB, so drift can't re-fire (`drift-handle-events.ts:247`) | no restamp |
| Cross-sweep **dedup** | `source_ref: drift:<kind>:<file>:<sha>`, skip if already open | none — same drift can file repeatedly |
| **Cascade alarm** circuit breaker | stop filing at ≥10 open drift FBs (`drift-handle-events.ts:409`) | none |
| Baton **exemption** | a file that is both an input-witness and a current output is exempt (in-loop writes don't re-fire) | none |
| **Verifier nonce** anti-self-certify | cursor mints a one-time nonce; seal tools refuse without it (`verifier-nonce.ts`) | seal/advance is instruction-gated only |
| **FSM checksum** sidecar | `.fsm_checksum` detects tampering on hookless harnesses (`state-integrity.ts:108`) | none |
| Pre-tick **self-repair** | synthesize missing approvals; recover stale dispatch **leases**; reset lost worktree/branch units (`run-tick.ts`, `unit-branch-recovery.ts`) | no dispatch-lease concept, no recovery |
| **Deadlock / churn halt** | same action signature ≥4 ticks, or A↔B churn ≥8 ticks → `loop_halted` (`deadlock-detector.ts`) | **HAVE** — real detector `deadlock.rs:1-354` (HALT_THRESHOLD=4, churn window=10) + per-unit MAX_PASSES=8 |
| Fix-chain **merge gate** + downstream invalidation | fix runs on an isolated branch, merge-gated; revisit clears downstream approvals (`fix-chain-merge-gate.ts`, `invalidate-downstream.ts`) | PARTIAL — `FixFeedback` action exists; no worktree-per-fix, no downstream invalidation |

### C. Cursor richness ★★

- **Multi-signal elaborate loop**: one action carries every unmet signal
  (conversation / verify / discovery:<agent> / decompose / verify_decompose); the
  agent stacks several in a tick. darkrun's Spec phase is monolithic.
- **Optional stages**: keep-or-drop offer on first arrival + `dependents[]`.
  darkrun's six are all mandatory (by design — but the predecessor's right-sizing
  was a live gate, not just run-start collapse).
- **Severity-threshold feedback waves** + batching + **stuck-reject escalation**
  (≥2 consecutive rejects on the same hat → escalate). darkrun returns the first
  open feedback, no batching, no escalation.
- **Intent-scope vs stage-scope** reviews and quality gates (union, deduped).
- **Mode-shaped role lists at two levels** (autopilot drops user gate per-stage but
  the final intent user gate is always sacred).
- **Forward-only gates** (briefs/observations never interrupt an in-flight wave).
- **Delivery verification**: re-audit PR merge state before sealing.

### D. Gate / proof depth ★★

- **Quality-gate dispatch**: run declared gates at stage + intent scope;
  **environment-blocked** classification (DB down ≠ test failed); **defer-to-CI**
  escape hatch after N non-convergent attempts (prevents permanent wedge).
- **Output-existence gate** at closeout (declared outputs must exist; repairs
  extension typos).
- **Input-shape validation gate** (inputs declared as file paths, not unit names).
- **Coverage acknowledgement** (out-of-scope / covered-by-unit decisions).
- **Proof upload to the PR/MR** as a durable asset.

darkrun has `CheckpointKind` + `proof.rs` + `run_surface`, but none of these gates.

### E. Content-model expressiveness ★  (post-orientation-refactor, still missing)

Per-role frontmatter the predecessor's hats/agents carried that darkrun roles don't:
`model`, `interpretation` (lens vs strict dispute posture), `run_quality_gates`,
and `role: plan|build|verify` (reject bounces to the nearest **build** role).
Also: structured cross-stage `inputs: [{stage, output}]`, `review-agents-include`,
`applies_to` artifact globs, `produces: build|knowledge`, **compound gates**
(`checkpoint: [external, ask]`), per-worker output ownership, and studio-level
intent-completion reviewers/fix-hats.

### F. Agent-safety / write path ★★

- **guard-workflow-fields → HAVE.** `hooks.json` wires the full 8-hook suite; the
  PreToolUse guard blocks raw Write/Edit on managed files, exits(2) with a
  redirect-to-tool message, and suspends during merge (`cli/src/hook.rs:170-206`).
- **human_write → MISSING.** No conversational write tool with allow/deny-list +
  symlink guard. HTTP feedback only stamps `author="user"` (`http/handlers.rs:381`).
- **record_agent_write / stamp-agent-write → HAVE.** PostToolUse hook appends every
  agent Write/Edit to `drift-witness.log`, read by the drift track (`cli/src/hook.rs:393-431`).
- **review_stamp → MISSING.** Reviews resolve only via `checkpoint_decide` → full
  tick (`tools.rs:1049`); no parallel-safe self-stamp, so concurrent review waves
  risk the churn the predecessor engineered around.
- Session **presence/heartbeat → PARTIAL** — live-connection count + heartbeat
  endpoint exist (`http/state.rs:130-177`); missing the grace windows and
  reattach precedence (arg > live registry > FM pointer).

### G. Lifecycle / state extras ★

`plugin_version` + migration markers (`migrated: true`); `external_refs` (ticket/
PR/design handles); `clarifications` (stage Q&A); draft-PR lifecycle
(status/ready_at, not just a ref); variable stage list (drop) vs the fixed six;
persisted session metadata (survives restart).

---

## Recommended sequence (dependency-ordered, no commitment yet)

1. **Cluster A** — the story-bearing records. Smallest, highest daily value, and
   the one already surfaced. Append-only iteration with `note`; derive pass/worker;
   thread the handoff into the next worker; add feedback `origin` + `closure_reply`.
2. **Cluster B** — the immune system. This is the real debt: premise-witness drift,
   restamp, dedup, cascade alarm, dispatch leases + recovery, deadlock halt. Without
   it, long autonomous runs corrupt or wedge.
3. **Cluster D** then **C** — gate depth and cursor richness.
4. **E / F / G** — expressiveness and safety polish, prioritized by which actually
   bite in practice.

Each line above should be verified against darkrun source before it's scheduled —
this doc is the starting ledger, not the final word.
