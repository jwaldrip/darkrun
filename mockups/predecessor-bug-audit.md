# Predecessor bug audit — does darkrun share them?

Audited the predecessor engine's recorded bug reports (from `~/Downloads/`) against
darkrun source. **Two passes:** the first read four hand-written summary `.md`
files (11 named bugs, the table below); the second re-audited the **full corpus of
~42 session bundles** (zipped `BUG-REPORT.md` + transcripts) — see *Full-corpus
re-audit* further down. Each mechanism is classified **FIXED** (darkrun shared it —
now fixed), **immune** (darkrun's design prevents it — verified + regression-tested),
or **backstopped** (caught by an existing guard). Every row has a test.

> First-pass source summaries: `haiku-engine-bugs-admin-portal-reimagine.md`
> (BUG-1…6), `haiku-drift-loop-bug-5.0.3.md` (drift A/B),
> `haiku-pick-design-direction-bug-2026-05-18.md` (BUG-9/10),
> `bug-spa-selection-no-chat-breadcrumb.md` (BUG-11). The real corpus is the
> session zips themselves (`~/Downloads/haiku-*bug*.zip`, extracted to
> `/tmp/haiku-bugs/`).

| # | Predecessor bug | darkrun | Where |
|---|---|---|---|
| **BUG-3** | 0-byte/`touch`ed output passes the existence check (`existsSync` only); empty file reads "stable" to drift | **FIXED** — `missing_outputs` now requires a regular, non-empty file (`output_present`) | position.rs |
| **BUG-1** | CI-deferral attempt counter keyed per-UNIT → a gate defers on its FIRST failure (inherits the unit's count) | **immune** — `attempts` keyed per-gate-name; only `env_blocked` defers, never a `fail` | units.rs |
| **Drift A** | sweep diffs via `git log` on a worktree-prefixed path → `commits:[]` sticky false positive → infinite `drift_detected` | **immune** — drift is pure content-hash (zero git); no `commits` field, no path-prefix bug | drift.rs |
| **Drift B** | `target_invalidates` never clears `approvals.<role>` on FB close → witness never refreshes → loops forever | **immune** — `close_with_reply` actually removes the invalidated roles from `reviews`/`approvals`; restamp-on-detect fires once | feedback.rs · drift.rs |
| **BUG-6** | non-code finding reaches a builder that can only edit-or-reject → loops to the bolt cap, never closes | **immune** — terminal non-code routes (`Answered`, `NonActionable`) settable directly | feedback.rs |
| **BUG-9** | interactive tool didn't write its declared manifest file → discovery gate looped on file-existence | **immune/backstopped** — darkrun doesn't couple a gate to a tool-written file; the deadlock guard escalates any stuck action after 4 ticks | sessions.rs · deadlock.rs |
| **BUG-10** | second tool call replayed the same cached selection without re-opening the picker | **immune** — each `create_*` mints a fresh incrementing session id; no stale replay | sessions.rs |
| **BUG-5** | `report` silently no-ops (returns success) without a Sentry DSN → findings dropped on dev builds | **immune** — `report` always writes a durable `.darkrun/reports/<id>.md`; never a dead sink | meta.rs |
| **BUG-4** | seal wrote frontmatter via a raw non-commit → auto-push skipped → origin stale, CI ran old code | **mitigated** — darkrun's land is a real merge commit (state is gitignored), so the bug can't occur; the seal prompt now surfaces `branch_status` (ahead/diverged) so the operator knows origin still needs a push | position.rs · lifecycle.rs · sealed.md |
| **BUG-2** | intent-scope gates re-emit from frozen unit specs after code relocates (no `superseded`) → loop | **covered** — `env_blocked → deferred_to_ci` (a gate that can't run locally auto-defers); deadlock guard backstops | units.rs |
| **BUG-11** | out-of-band SPA studio/mode selection leaves no breadcrumb → agent mistakes legit fields for silent defaults, breaks autopilot | **immune by design** — `run_start` takes factory+mode as explicit args (no silent defaults); the tick reflects resolved state | position.rs |

## The one real shared bug

Only **BUG-3** was a genuine shared defect (the output-existence gate trusted
`.exists()` and would pass a 0-byte file) — fixed. Everything else darkrun's
architecture already prevented:

- **Drift was the big one.** The predecessor's most severe report (an
  intent-blocking infinite drift loop across two engine versions) came from two
  causes — git-log path resolution and `invalidates` not clearing approvals — that
  the darkrun drift rebuild structurally eliminates: content-hash diffing (no git),
  restamp-on-detect (fires once), and a close that actually clears the stamps. A
  regression test mirrors the exact repro and proves the loop can't recur.
- **The deadlock/churn halt** is darkrun's general backstop for the whole class of
  "engine returns the same unmet signal forever" bugs (BUG-2, BUG-9): any action
  stuck 4× with no progress escalates to the operator instead of looping.
- **No dead sinks, no silent defaults.** `report` always writes locally (BUG-5);
  `run_start` requires explicit factory+mode (BUG-11).

---

## Full-corpus re-audit (the session bundles, not the summaries)

The first pass above read four hand-written summary `.md` files. The real corpus
is **~42 session bundles** (zipped: `BUG-REPORT.md` + full transcripts) in
`~/Downloads`, covering far more failure modes than the summaries named. This
section re-audits darkrun against every distinct mechanism in that corpus. Each
was mapped to darkrun source and classified **FIXED / immune / backstopped** with
an exact citation; the load-bearing immunities (A, D, F below) were re-verified by
hand against the code, not taken on a reviewer's word.

**Result: no new shared defect.** Every mechanism in the real corpus is already
immune or backstopped; BUG-3 (above) remains the only genuine shared defect, and
it is fixed. The new mechanisms and where darkrun stops them:

| Mechanism (predecessor) | darkrun | Where |
|---|---|---|
| **Terminal-feedback re-dispatch** — dispatcher keeps re-selecting a closed/answered/non-actionable FB because selection doesn't filter terminal status → loop-halt | **immune** — `feedback_open` filters all five terminal statuses (`closed, rejected, addressed, answered, non_actionable`) before any selection; a terminal FB never reaches the cursor | position.rs:1096 · feedback.rs:33 |
| **User-gate churn false-positive** — loop guard counts legitimate "waiting for human at the gate" re-emits as no-progress and halts | **immune** — `is_exempt` exempts `UserGate/Checkpoint/PendingSeal/ExternalReviewRequested/Sealed/MergeConflict/Escalate/FeedbackQuestion`; stale history (>1h) resets on resume | deadlock.rs:69 |
| **Returned-but-unexecuted action counted as no-progress** — guard halts on identical *returns* before the work runs | **backstopped** — the wedge signature is `tag@station + progress_fingerprint` (unit/done/pass counts + run status + drift count + per-FB id:status); any real resolution resets the counter — pure action-equality never halts | deadlock.rs:87,123 |
| **Migration livelock / version regression** — migrator writes a version ≤ what it read → re-migrates every read forever | **immune** — `migrate_state` is pure and always stamps the compile-time `SCHEMA_VERSION`; monotonic, idempotent, in-memory | state.rs:127 |
| **Uncommitted-migrator churn** — migrator rewrites the file on every read (dirty git) | **immune** — migration runs in-memory on read; bytes hit disk only on an explicit `write_state` | state.rs |
| **Lossy unit frontmatter migration** — migrating a unit drops unknown fields (`inputs`, `quality_gates`, …) | **immune** — no custom unit migrator; `UnitFrontmatter` round-trips every declared field through serde; `inputs/outputs/quality_gates` survive | domain.rs:517 |
| **Stale baseline** — cached drift/diff baseline not refreshed after migration | **backstopped** — witness hashes are recomputed from file content every `sweep`; nothing caches a baseline across ticks | drift.rs:128 |
| **Non-convergent review loop** — a rejection re-invalidates the very stamp a fix just satisfied → review↔fix ping-pong to the cap | **backstopped** — invalidation is scoped to an explicit immutable `invalidates:[role]` list; a `Closed` FB is terminal (`FeedbackSettled`); per-unit pass budget escalates to the operator instead of looping | feedback.rs:314 · position.rs (MAX_PASSES) |
| **Frontmatter clobber on fix-chain merge** — merge takes one file side wholesale and resets status / drops stamps | **immune** — `engine_protected_merge` re-asserts the whole `.darkrun/<run>` tree from the base side after a `--no-commit` merge; engine state is authoritative-from-base, only agent-content conflicts surface | git/merge.rs:56 |
| **Unsatisfiable same-wave dependency** — a unit dispatched whose dep sits in the same unfinished wave | **immune** — Kahn topo-sort assigns wave = `max(dep_waves)+1`, so a dep is never same-wave; `wave_ready` requires all deps `Completed` | core/dag.rs:98 · units.rs |
| **Phase regression on mid-flight mode change** — switching mode rewinds the station phase machine | **immune** — mode is snapshotted at start; phase derives strictly from append-only completion signals, so it can't rewind | position.rs:764 |
| **Env-vs-source gate not distinguished** — a gate that can't run locally is treated as a real failure → loop | **backstopped** — `GateStatus::EnvBlocked` is distinct; after `GATE_DEFER_AFTER=2` it becomes `DeferredToCi` | units.rs:189 |
| **Missing fix-worker for an artifact type** — feedback targets something no worker can edit → loop | **backstopped** — terminal `Answered`/`NonActionable` routes let the operator settle it; terminal FB is then filtered from dispatch | feedback.rs · position.rs:1096 |
| **Stale gate command path** — a gate command references a moved file/symbol → always fails | **immune by model** — gates are operator-supplied / factory-declared and re-read each run; the engine never caches or dispatches a gate command | gate.rs · units.rs |
| **Tool advertised but no handler** — server lists a tool with no dispatch → call errors/hangs | **immune** — `#[tool_router]` binds name→handler at compile time; an unhandled advertised tool fails to compile | registry.rs · tools.rs |

### Why no fixes were needed

The corpus is dominated by one family — **"the engine returns the same unmet
signal forever."** darkrun closes that family at three structural levels rather
than per-bug:

1. **Selection filters terminal work** — `feedback_open` and the drift/feedback
   tracks never re-offer something already settled, so the most common loop seed
   (re-dispatching a closed item) can't be planted.
2. **The deadlock guard measures real progress, not action identity** — its
   fingerprint is disk state, and it exempts legitimate external waits, so it
   neither false-halts on a human gate nor fails to catch a genuine no-progress
   loop.
3. **Engine state is authoritative-from-base on merge and in-memory on migrate** —
   so the two corruption vectors (merge clobber, lossy/looping migration) have no
   surface.

The predecessor hit these as ~20 separate incidents because it lacked the
structural guard at each level. Where darkrun *does* rely on a backstop rather
than a structural impossibility (env-blocked gates, unfixable feedback, the review
budget), the backstop escalates to a human instead of looping — the opposite of
the predecessor's silent wedge.

*Companion to `engine-parity-gaps.{csv,md}`. First pass (4 summaries): 2026-06-05.
Full-corpus re-audit (~42 session bundles): 2026-06-05. Per-mechanism findings in
`/tmp/audit-group{1,2,3,4}.md`.*
