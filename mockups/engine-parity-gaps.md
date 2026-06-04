# Engine Parity — what darkrun ported, and what it missed (VERIFIED)

A full read of the predecessor engine (its `packages/haiku/src`, ~85K LOC) against
darkrun's current engine. **Every row below has been verified against darkrun
source** — verdict (HAVE / PARTIAL / MISSING) with `file:line` evidence. The point
is honesty about the gap, not a to-do list yet.

> Vocabulary: "the predecessor" is the prior TypeScript system. Never write its
> name into darkrun code, content, or output.

**Verified tally — 44 mechanisms: 3 present · 15 partial · 26 missing.**

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
