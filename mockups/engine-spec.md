# darkrun engine spec — run organization, derivation, git, reviews

The authoritative target for the engine. Reverse-engineered from the proven
predecessor and mapped onto darkrun vocabulary (Factory>Station>Unit>Pass;
Worker; Run; Checkpoint; Reviewer; Explorer). Companion to `merge-engine.md`
(the 9 tuned merge mechanics) — this doc is the wider model.

The single load-bearing idea everything else hangs on: **a Run's state is a pure
function of on-disk signals + git branch topology, computed live every tick.**
There is **no `state.json`** — the predecessor's migrator *deletes* any pre-existing
file and the engine never reads or writes it for phase (verified:
`current-state.ts:66` "readStageState removed (v4)", `parser.ts:229` "v4 contract:
state.json is gone", `repair-agent.ts:201` "the v0→v4 migrator deletes the file").
darkrun must drop `state.json` entirely — NOT keep it as a cache. Phase/status/gate
outcome are derived on demand from per-unit FM (`iterations[]`, `reviews{}`,
`approvals{}`) + feedback `closed_at` + branch-merge state.

### Contents

1. On-disk layout · 2. Frontmatter contract · 2b. State-derivation algorithm ·
3. Three-track cursor · 4. Pretick · 5. Branch hierarchy + engine-protected merges ·
6. Drift (witness model) · 7. Reviews / quality / checkpoint ordering + role classes ·
8. Inputs/outputs threading + verification · 9. Knowledge / shared memory ·
10. Overrides cascade · 11. Generated prompts · 12. Loop guards · 13. Modes ·
14. Spec / consistency / migratability · 15. Cursor action vocabulary ·
16. Elaborate-loop signals · 17. Feedback lifecycle + fix loop ·
18. Checkpoint kinds + right-sizing · 19. Locks / concurrency · 20. Write guard /
ownership · 21. Human gate / session surface · then the Gap summary + Build order.

Every claim is verified against the predecessor source; this doc is the authoritative,
build-ready target for the run-organization rewrite.

---

## 1. On-disk layout

```
.darkrun/<run>/
  run.md                         frontmatter (factory, mode, active_station, status, …) + body
  feedback/NN-slug.md            RUN-LEVEL feedback (final closeout / cross-station) — station:""
  knowledge/**.md                run-scope knowledge (explorer output, run-local)
  reflection.md                  synthesized end-of-run reflection (written before seal)
  action-log.jsonl               append-only write-attribution journal (drift absorbs agent writes)
  write-audit.jsonl              append-only audit journal (provenance; never surfaced in the UI)
  run-tick.json                  run-scope tick counter (bookkeeping)
  stations/<station>/
    STATION.md                   station-definition copy
    elaboration.md               pre-decompose capture; FM verified_at gates elaborate→review
    brief.md                     ONE file, FM phase: pre|post (the "outcome" is phase:post)
    observations.md              post-checkpoint, pre-merge reflection note
    units/<unit>.md              per-station unit specs (FM drives phase derivation)
    feedback/NN-slug.md          per-station feedback
    artifacts/**                 the station's real deliverables (frame.md, spec.md, code…)
    knowledge/, discovery/       station-scope research surfaces
    decisions.jsonl              append-only per-station decision log
    + drift witnesses (see §6)
```

Stations are **directories**, not just a `station:` frontmatter field + a map key.
Final/closeout feedback lives at the run root (`station:""`); working feedback is
per-station. Locked artifacts are **written to disk** under `stations/<station>/
artifacts/` — today darkrun only passes the artifact name as a prompt variable and
never persists it (a miss).

## 2. Frontmatter contract — the on-disk schema (the de-facto spec)

There is no version field; the schema IS this protected-field set, enforced by the
write guard (§20) + a checksum tamper detector (§14). These are the signals the
derivation reads — darkrun units/feedback today carry almost none of them.

**unit** (`stations/<station>/units/<unit>.md`):
```yaml
name, type, status            # status: pending|active|completed (display only)
depends_on: []                # DAG edges — the wave scheduler sequences on these
pass: 0                       # current Pass number (was "bolt")
worker: ""                    # current worker in the Pass loop (was "hat")
model?, applicable_skills?
inputs: []                    # consumed artifact paths ([] = reads nothing; ABSENCE is an error)
outputs: []                   # produced paths — auto-populated from the unit worktree git diff
quality_gates: [ {name, command, dir?} ]   # build-class units MUST declare (empty [] allowed, explicit)
iterations: [ {worker, started_at, result, pass} ]   # result: advance|reject|null
reviews:   { <role>: {at} | null }    # PRE-exec spec/adversarial stamps
approvals: { <role>: {at} | null }    # POST-exec gate stamps (incl. user, quality_gates)
input_witnesses: { <path>: <sha256> } # per-slot drift witnesses (the signed-over inputs)
started_at?, completed_at?
```
`iterations`/`reviews`/`approvals`/`input_witnesses` are **engine-managed** (FSM
fields, agent cannot write them); `outputs`/`quality_gates` stay agent-editable AFTER
a unit is active (the sanctioned path to fix a gate command / missed output — it
changes how/what is *verified*, not the workflow). Every other field is forward-only
immutable once active.

**feedback** (`stations/<station>/feedback/NN-slug.md`, or run-root for closeout):
```yaml
id/num, title, status, origin, severity   # see §15 for the enums
station: ""                                # "" = run/closeout scope
source_ref                                 # back-ref to origin (e.g. reviewer run id / drift:<kind>:<file>:<sha>)
triaged_at                                 # null blocks the pre-tick gate
resolution: question|inline_fix|stage_revisit | null
targets: { unit, invalidates: [] }
iterations: [ {worker, started_at, result, pass} ]   # the fix-loop Pass history
closed_by, closed_at                       # non-null closed_at = closed (the lifecycle witness)
replies: [], closure_reply, closure_reply_unread
```

**run** (`run.md`): `factory, mode, active_station, status, started_at,
completed_at, sealed_at, follows?, brief?` (`active_station` is a write-only cache,
never read for state). **station** (`STATION.md`): `workers: []`, `fix_workers: []`,
`reviewers: []`, `checkpoint: auto|ask|external|await`, `locked_artifact`, `inputs:
[]`, `optional?`, `elaboration?`.

## 2b. State derivation algorithm (no snapshot)

Verified `derivePhase(units, workers, reviewRoles, approvalRoles, mode,
elaborationVerified)` — a pure function, computed live every tick:
```
1. elaborate gate (skipped in auto): elaborationVerified==false → "elaborate";
   ==null (artifact missing) AND zero units → "elaborate"
2. zero units → "elaborate" (decompose pending)
3. review:  any unit missing any required reviews.<role>  → "review"      (PRE-exec)
4. execute: any unit whose LAST iteration is not (result==advance AND
            worker==workers[last])  → "execute" (manufacture)             [if workers nonempty]
5. gate:    any unit missing any required approvals.<role> → "gate" (audit)(POST-exec)
6. else → null (all signed; past gate, awaiting the station→run-main merge)
```
Order is load-bearing: review MUST be checked before execute (a not-yet-spec-signed
unit has empty iterations and would mislabel as execute). Station selection =
`findCurrentStage` = first station whose units aren't all signed. Station iteration
history is **derived from closed feedback** (sort by `closed_at`), not a separate log.

### 2c. ONE shared derivation crate (engine + http + desktop + website)

This is the structural invariant that makes "they can never disagree" true, and the
predecessor's key move: the pure derivation lives in **one wasm-safe crate**
(`darkrun-core` — already a dependency of `darkrun-mcp`, `darkrun-http`, the desktop,
AND `web/site`), mirroring the predecessor's `packages/shared/derived-stage-state.ts`.
The **engine** (cursor walk), the **HTTP browse** endpoints, and the **desktop** all
call the SAME `derive_station_phase(units, review_roles, approval_roles, …)` over the
SAME on-disk frontmatter, deriving independently — so there is no snapshot for them
to drift from.

**Today darkrun does NOT do this.** `RunState::station_status_summary` /
`active_phase` (`darkrun-core/state.rs`) read the **recorded `st.phase` from the
`state.json` snapshot**; the engine (`position.rs`) computes it and `write_state`s the
snapshot, and http/desktop read that snapshot. So the cursor logic is **not** shared
across the three the way the predecessor shares it — it's "engine derives once,
persists, others read." Dropping `state.json` (gap 5) means: move the pure
`derive_station_phase` into `darkrun-core`, have the engine/http/desktop all run it
over the unit/feedback FM, and delete the snapshot reads. The website (wasm, no live
engine) runs the same crate over its static fixtures / the API payload.

## 3. The three-track cursor

Each `run_next` derives the next action by walking three tracks in priority:
1. **Drift (Track C)** — engine-internal, no agent action: sweep witnesses,
   restamp, file at most one feedback per (file,kind) group.
2. **Feedback (Track B)** — dispatch open feedback into the owning station's
   fix-worker chain (one **Pass** = one trip through the chain).
3. **Run (Track A)** — the normal Spec→Review→Manufacture→Audit→Reflect→Checkpoint
   walk, station by station, then the run-level completion review.

## 4. Pretick (before the cursor reads the tree)

The cursor is a pure read of the working tree, so the tree is force-aligned first,
in order:
1. **Mid-merge detector** — if `MERGE_HEAD/REBASE_HEAD/CHERRY_PICK_HEAD/REVERT_HEAD`
   present → block with the conflicted-file list (structured recovery), FIRST.
2. **Clean-tree gate** — only NON-`.darkrun/` changes block (engine never authors
   agent commits).
3. **Branch reconcile** — fetch; FF `darkrun/<run>/main` ← `origin/<default>`
   (ff-only); FF current station branch ← run-main.
4. **Downstream sync** (debt-gated, engine-protected): (a) `<default>`→run-main,
   (b) run-main→active station branch. Report which step conflicted.
5. **Idempotent self-repairs / migrators** — version-gated migrators + a battery of
   self-heals: missing-approval repair, lost-unit reset (worktree+branch gone),
   stale-lease recovery, duplicate-feedback-id heal, malformed-input auto-file,
   pending fix-chain merge completion.
6. Then the cursor walks.

## 5. Branch hierarchy + engine-protected merges

```
<default>  →(FF only)→  darkrun/<run>/main  ←(protected)←  <default>
darkrun/<run>/main  →(protected, in-place)→  darkrun/<run>/<station>
darkrun/<run>/<station>  ←(protected)←  darkrun/<run>/<unit>        (unit → station)
darkrun/<run>/<station>  →(protected)→  darkrun/<run>/main           (station → run, at completion)
darkrun/<run>/main  →(protected)→  darkrun/<run>/<nextStation>       (fork-forward)
darkrun/<run>/discovery-<station>-<tmpl>  →  station                 (explorer isolation)
darkrun/<run>/fix-<scope>-<FB>  →  station/run-main                  (fix-chain isolation)
darkrun/<run>/main  →  <default>                                     (delivery PR)
```
- **Per-unit worktrees**: `.darkrun/worktrees/<run>/<unit>` on `darkrun/<run>/<unit>`,
  forked from the station branch; created on unit start, merged + reaped on unit
  completion.
- **Engine-protected merge**: `merge --no-commit` → for every engine-owned path on
  the TARGET ref, `git checkout <target> -- <path>` + `git add` (force-hold engine
  state to the side that's always ahead) → remaining `--diff-filter=U` = real agent
  conflict → commit. This is the only merge shape used for engine merges.

## 6. Drift (witness model)

Each signed slot (`reviews.<role>`/`approvals.<role>`) carries `input_witnesses`
(sha256 of the inputs it signed over). A sweep hashes current content; a changed
witness **re-opens that exact slot** (re-fires the fix loop) and is **restamped
to the new sha at detect time** so the same signal can't re-fire next sweep.
Dedup by (file,kind). darkrun today has a coarse run-level `witnesses.json` — this
must become per-slot.

## 7. Reviews / quality gates / checkpoint ordering

Within a station, the exact emission order:
```
elaborate (discovery + decompose; elaboration.md verified)
  → PRE-exec review:  spec (serial, global prompt injection) → adversarial fan-out
  → brief.md (phase:pre)                         ← before the REVIEW user gate
  → user_gate{spec}  = the REVIEW gate
  → manufacture (Pass loop per unit: Make→Challenge→Resolve)
       → quality_gates run when the LAST worker in the loop lands (at advance),
         BEFORE the unit→station merge                ← QUALITY GATE #1
  → POST-exec approval:  spec → adversarial fan-out → quality_gates → brief.md(phase:post)
       → the quality_gates approval actor re-runs them AFTER the reviewers
         (certifies the final post-fix state)         ← QUALITY GATE #2
  → user_gate{approval} = the CHECKPOINT                ← final gate, BEFORE merge
  → observations.md                               ← AFTER the checkpoint, before merge
                                                    (incorporates the user's gate actions)
  → complete_station = the station→run-main merge
```
Then, after the LAST station merges, the **run-level** completion review:
```
spec → run-level adversarial reviewers (fan-out) → intent quality_gates (union of
all stations' gates) → run-level user gate → reflection.md → seal (after landing
on <default>).
```
- **spec review** is a global, code-resident prompt injection applied to every
  station — not a per-station configured reviewer.
- **adversarial review happens twice**: within each station (after spec), and again
  at the run level with run-level reviewers.
- **Reviewer role classes** (the role list is built from these, the single source the
  cursor + dispatch builders share):
  - **Serial roles** = `{spec, quality_gates, user}` — run one-at-a-time and lead the
    walk; everything else fans out in **parallel** (one batched dispatch).
  - **Runtime-observation roles** (e.g. `runtime-verifier`) audit the *built* work
    (boot the app, drive a browser, run the command). Nothing exists to observe
    pre-build, so they're **excluded from the PRE-exec review** and fire only
    POST-exec + at run completion; they get a `proof/` evidence-write carve-out (never
    source). They **HOLD** (file BLOCKED, never sign) if they can't actually run it.
  - **PR-interaction roles** read CI/checks and post/resolve PR threads via `gh`/`glab`
    but never edit source — code fixes flow through feedback + the fix-worker loop.
  - Engine-built-in adversarial roles (`continuity`, `cross-stage-consistency`) are
    always present; factory reviewers come from the override cascade (§10).
- **quality gates run at THREE points** (darkrun decision — keep BOTH the
  per-loop run AND the post-review actor, which the predecessor's v4 collapsed to
  one): (1) when the **last worker in a unit's Pass loop lands** (at advance, before
  the unit→station merge); (2) as the **`quality_gates` approval actor** after the
  station's adversarial reviewers (certifies the final post-fix state); (3) at the
  **run tick** (`scope: intent`) after the run-level reviewers. Gates classify
  env-unavailable separately and can defer to CI after N non-convergent attempts.
- **observations come AFTER the checkpoint gate**, committed with the station merge,
  BY DESIGN: the user's actions/decision at the gate are themselves key signal the
  station's observations must capture. (The pre-gate artifact is the *outcome* =
  `brief.md phase:post`.)
- **the final station checkpoint fires BEFORE the final merge** (the merge is
  unreachable while any approval stamp is null).

**The Pass loop (manufacture, per unit):** the unit's workers run in order; each
appends an `iterations[]` entry `{worker, started_at, result, pass}` with
`result: advance|reject`. A worker carries a `role: plan|build|verify`. An `advance`
moves to the next worker; a `reject` (a Challenge/verify worker rejecting) **bounces
to the nearest preceding `build` worker**, incrementing the Pass. The unit is
complete when the **last** worker `advance`s (`last.result==advance &&
last.worker==workers[last]`) — that triggers quality-gate #1 then the unit→station
merge. Loop guards (§12) cap the Pass count and catch stuck-reject chains.

## 8. Inputs / outputs threading + verification

Enforced at THREE points, all via an `output_exists` primitive:
1. **wave-scheduler pick** — claim a unit only when every `depends_on` is complete
   AND every declared `inputs[]` path exists.
2. **unit start** — `inputs[]` must be declared (`[]` = reads nothing; absence is an
   error) and every input path must exist.
3. **unit completion** — `outputs[]` auto-populated from the unit worktree's git diff
   (engine bookkeeping excluded), scope-checked against the station's declared
   surface (`unit_scope_violation` otherwise), every output must exist on disk.
A unit declaring a sibling's output as an input without `depends_on` on that sibling
is rejected (`inputs_undeclared_producer`) — the wave scheduler sequences only on
`depends_on`. Cross-station threading: a station's outputs land on run-main at
completion; the next station's units declare them as inputs, verified the same way.

## 9. Knowledge / explorer / shared memory

- Run-scope knowledge (explorer/discovery output) is run-local under
  `knowledge/`/per-station `discovery/`, consumed via unit `inputs[]`.
- **Cross-run shared memory** is the reflection → project-overlay loop: at run close
  reflection synthesizes every station's `observations.md` + feedback + outcomes and
  **writes proposed project overlays into the project's `.darkrun/` config** (the
  override tier, §10) for override-class findings; engine-class findings route to a
  report. Future runs read those overlays.

## 10. Overrides (factory/station/worker/fix-worker/prompts/reviewers)

Two resolution idioms, both letting **project `.darkrun/` beat the bundled corpus**:
- **Ordered NAME lists** (a station's `workers:`/`fix_workers:`/a factory's
  `stations:`) — project-first **first-hit** (no merge). A station with no `workers:`
  gets `[]` (no implicit inheritance of the *list*).
- **BODY resolution** (the markdown a worker reads) — a tier cascade, least→most
  specific, last-write-wins by name:
  `corpus/workers → project/.darkrun/workers → corpus/factories/<f>/workers →
   project/.darkrun/factories/<f>/workers → corpus/.../stations/<s>/workers →
   project/.../stations/<s>/workers (WINS)`.
  So a station *names* a worker in its list, but the body is inherited from the
  factory/global tier unless overridden at the station tier. Fix-worker bodies fall
  back to the production-worker body when no fix-scoped body exists. Reviewers and
  templates resolve through the same cascade.

## 11. Generated prompts on disk

Every generated prompt is written to
`~/.darkrun/projects/<project-key>/runs/<run>/prompts/`:
- `stations/<station>/subagent-<unit>-<worker>-<pass>.prompt.md` — the full worker
  prompt (the parent agent is told to read the file and execute exactly).
- `run/…` — run-completion review/fix prompts.
- `main-action-<action>.prompt.md` — oversized main-agent action prompts.
- `refs/<kind>/<name>.md` — immutable snapshots of the source mandate/template the
  generator read, so the record is self-contained across corpus upgrades.
`<project-key>` = the absolute main-worktree root with `/`→`-` (one key shared by all
linked worktrees, via `git rev-parse --git-common-dir`). Written atomically at
dispatch-build time; one slot per logical prompt, overwrite-on-rerun.

## 12. Loop guards (multi-layer)

- **Fix-loop bolt cap** — after N (=3) Passes through a station's fix-worker chain
  on the same feedback, stop dispatching and escalate the feedback to a human.
- **Stuck-reject** — ≥2 consecutive `rejected` iterations on the same worker →
  terminal-stuck, refuse dispatch immediately.
- **Drift cascade breaker** — ≥10 open drift feedbacks → stop filing new drift
  feedback (slots still restamp), trip a cascade alarm recommending repair.
- **Deadlock detector** — the same action signature returned > HALT (=4) times, or
  Track-A↔B churn over CHURN (=8) ticks → swap for a `loop_halted` directive (a
  fresh signature resets it).
- **Per-call loop cap** — the run_next auto-execute loop is capped (=16) with a
  same-signature `no_progress` abort.

## 13. Modes

- **continuous** — full role lists; the human is present at every station Checkpoint
  (internal review session); single run-main draft PR.
- **discrete** — same role lists; each station opens a real draft PR (base =
  run-main) and **merging the PR is the checkpoint signal**. `discrete-hybrid` =
  per-station PR only for stations declaring external review.
- **auto(pilot)** — trims the per-station human gates (no `user` in the review/
  approval lists) but KEEPS the full adversarial fan-out + quality gates (the only
  backstop with no human watching); drops conversation/verify elaborate signals. The
  **run-level final user gate is always present** even in autopilot.
- (**quick** — a single-station run.)

## 14. Spec / consistency / migratability

- The contract is **structural**, not version-stamped: a canonical protected-field
  list per doc (run/station/unit) is the de-facto schema, enforced by the write
  guard + a checksum tamper detector for hookless harnesses.
- Forward-compat is **read-time defaulting** (absent fields coerce to safe
  defaults) — an older-layout run is tolerated without an explicit step.
- A one-shot **importer** migrates a legacy on-disk layout into `.darkrun/`,
  dry-run-first, git-clean-gated, keyed by directory existence (not a version field).
  If darkrun wants explicit versioned forward-migration of its own StateStore, that
  is net-new beyond the predecessor.

## 15. Cursor action vocabulary

`run_next` returns ONE structured next-action per tick (the agent performs it, then
re-ticks). The verified action set (darkrun's `RunAction`):
```
elaborate_loop {signals_unmet[]}     discovery + decompose; multi-signal payload (§16)
dispatch_review {dispatches[]}        PRE-exec reviewers (spec serial, then adversarial fan-out)
write_brief {phase: pre|post}         the brief(pre) / outcome(post) artifact
user_gate {gate_kind: spec|approval}  the REVIEW gate / the CHECKPOINT
start_unit_hat / start_feedback_hat   dispatch a worker in a unit / fix Pass loop
dispatch_quality_gates {scope}        run the quality gates (station or intent scope)
dispatch_approval {dispatches[]}      POST-exec reviewers (approval stamps)
record_observations                   write observations.md (post-checkpoint)
complete_stage                        the station→run-main merge (semantic, not a VCS verb)
intent_review                         run-level adversarial reviewers
record_reflection / pending_seal      run-close reflection + await landing on <default>
seal_intent / sealed                  finalize the run
close_feedback / feedback_question    feedback-track resolutions
unit_inputs_not_declared / unit_outputs_empty_iterations / merge_conflict   structural guards
```
The action's name is semantic — `complete_stage` *means* "this station is done"; the
git merge is an implementation detail the engine performs underneath.

## 16. Elaborate-loop signals

`elaborate_loop` carries a `signals_unmet[]` payload (the agent may satisfy any in
any order; re-tick re-derives). The verified signal set:
```
{signal: discovery, agent, units[]}   one per explorer template missing on disk
                                       (each runs in its own discovery worktree, §5)
{signal: conversation}                elaboration.md captured (the pre-decompose capture)
{signal: verify_conversation}         elaboration.md FM verified_at stamped → gates elaborate→review
{signal: decompose}                   units created with completion criteria + DAG
{signal: verify_decompose}            decomposition verified
```
Plus a keep-or-drop offer the first time an `optional` station is reached. Autopilot
drops conversation/verify_conversation/verify_decompose (auto-keeps optional stations)
but still emits the full set in the optional-offer path so a headless harness can't
deadlock.

## 17. Feedback lifecycle (the enums)

```
status:   pending → fixing → addressed → closed     (the fix path)
          pending → answered            (question resolved by a reply, no code delta)
          pending → non_actionable      (valid but no fix — out-of-scope / immutable target)
          pending → rejected            (invalid finding, dismissed by a worker)
          pending|fixing → escalated    (agent-FB bolt cap exceeded → human waypoint)
   Only pending/fixing block the station gate; escalated is a waypoint, not a blocker.
origin:   adversarial-review | studio-review | engine-review | drift | discovery |
          external-pr | external-mr | user-visual | user-chat | user-question | user-revisit
severity: blocker (stops the gate) > high > medium > low (nit)   — drives fix-loop order
resolution: question | inline_fix | stage_revisit   (router defaults to stage_revisit)
```
Each closed feedback IS one completed revisit cycle — station iteration history is
derived from `closed_at`, not a separate log. A reviewer finding files feedback that
**invalidates** the relevant `approvals.<role>` (resets it to null), so the run can't
seal until reviewers re-sign over the fix.

**The fix loop:** an open feedback dispatches into the owning station's `fix_workers`
chain (Track B); each trip through the chain is one **Pass** with its own counter.
A fix unit/Pass declares `closes: [FB-NN]`; landing it (via the per-feedback
**fix-chain worktree** `darkrun/<run>/fix-<scope>-<FB>`, §5) stamps the feedback's
`closed_by`/`closed_at`. Drift feedback and reviewer feedback both flow through this
same loop; the bolt cap + stuck-reject guards (§12) bound it.

## 18. Checkpoint kinds + gate resolution

A station's `checkpoint:` (its gate kind) and how the `user_gate{approval}` resolves
per mode:
```
auto      — advance with no human (right-sized small work; auto mode)
ask        — internal review session; the human approves in the desktop app
external   — hand off to an external surface (PR/MR/deploy sign-off) and AWAIT it
await      — block until a decision arrives
```
- **continuous:** every `ask` checkpoint pops an internal review session in the app.
- **discrete:** each station opens a real draft PR (base = run-main); **merging the
  PR is the approval signal**. `discrete-hybrid:` per-station PR only for stations
  whose `review:` is `external`.
- **auto(pilot):** per-station `user` gates removed (the cursor drives through);
  `external`/`await` still pause it; the run-level final user gate is always present.
Right-sizing is two concrete mechanisms, NOT a magic size oracle: (1) **optional
stations** — a station marked `optional` is offered for keep-or-drop the first time
the cursor reaches it (dropping removes it from the run's station plan); (2) the
**mode-shaped role lists** — `auto` trims the per-station human gates and the
conversation/verify elaborate signals. The work shape falls out of those, not a
manual mode pick.

## 19. Locks / concurrency

The engine serializes writes to shared branch state with **advisory `mkdir` locks**
(atomic on POSIX + Windows, no native deps; the dir's existence IS the lock):
- `acquireLock(name, tag)` — `mkdir` the lock dir; on contention retry every ~50 ms
  up to a ~30 s timeout; write a `holder.json` (`{pid, tag, at}`).
- **Stale recovery** — a lock is stale when its holder pid is dead
  (`process.kill(pid, 0)` throws) or the dir mtime exceeds a max age; a stale lock is
  *stolen* (`rm -rf` then re-acquire) so a crashed tick never wedges the run.
- Scopes used: `withStationLock(run, station)` (unit→station merges, station entry)
  and `withRunMainLock(run)` (station→run-main merges) — the two points where
  concurrent ticks/agents could race the same branch.
- Released in a `finally` (`rm -rf` the lock dir). darkrun should port this (Rust:
  the same `mkdir`-create-exclusive + pid liveness via `nix::kill(pid, 0)`).

## 20. Write guard / ownership

Engine-managed frontmatter fields are **agent-unwritable**. The guard (a PreToolUse
hook on harnesses that fire hooks; the schema-tool layer otherwise) rejects any raw
Write/Edit that would touch an FSM-driven field. The protected sets (the de-facto
schema, darkrun vocabulary):
- **run**: `status, active_station, started_at, completed_at, phase,
  completion_review_*`.
- **station**: `status, phase, started_at, completed_at, gate_entered_at,
  gate_outcome` (in darkrun these are *derived*, never stored — so the guard mostly
  protects them from being faked).
- **unit**: `status, started_at, completed_at, pass, worker, worker_started_at,
  reviews, approvals, iterations, input_witnesses, scope_reject_attempts`.
  Agent-authorable exceptions: `inputs`, `depends_on`, `title`, `model`, and —
  uniquely — `outputs`/`quality_gates` stay editable AFTER the unit is active (the
  sanctioned path to repair a gate command / missed output).
- Engine-owned **paths** (units, feedback, run.md, the station artifacts the engine
  stamps) go through the schema-validating MCP tools, not raw writes.

**Mid-merge suspension (critical):** while a git merge is in progress
(`MERGE_HEAD`/`REBASE_HEAD`/`CHERRY_PICK_HEAD`/`REVERT_HEAD`/rebase markers), the
ownership/lifecycle guards are **suspended** so the agent *can* edit the conflicted
engine files to resolve them — but **schema validation stays on**, so a malformed
resolution still fails loudly. Without the suspension the guard would refuse the very
writes needed to finish the merge. (This is mechanic #3 of `merge-engine.md`.)

## 21. Human gate / session surface

When the cursor emits a `user_gate` (or a `question`/`direction`/`picker`/`view`
action), the engine **raises a typed session** that the desktop app (and the HTTP/WS
feed) renders for the human. The session types (the wire contract darkrun-api
mirrors):
- **review** — the station gate: the unit list + completion criteria, declared
  outputs, the brief/outcome, and approve / request-changes (request-changes files
  feedback). Both the REVIEW gate (`gate_kind: spec`) and the CHECKPOINT
  (`gate_kind: approval`) are review sessions.
- **question** — a free-text question to the operator; the answer threads back.
- **direction** (design_direction) — pick a design/approach direction.
- **picker** — choose one of N engine-offered options (e.g. mode, an archetype).
- **view** — a read-only surface (an artifact/diff to look at, no decision).
- **visual_review** / **proof** — image/annotate review of a built artifact, and the
  objective-evidence (`proof/`) captures a runtime-observation reviewer produced.

Sessions are keyed by run slug in an in-memory registry (no repo-persisted gate
pointer); the engine raises one, the human resolves it in the app, and the resolution
(approve / answer / pick / merge-the-PR in discrete mode) stamps the corresponding
`approvals.<role>` / closes feedback, so the next tick advances. The review surface is
the ONLY interactive face darkrun drives — never a browser/Playwright flow.

---

## Gap summary (darkrun today → this spec)

1. Flat `units/`/`feedback/` → per-station dirs + run-level closeout feedback.
2. Locked artifacts never written → persist under `stations/<station>/artifacts/`.
3. Brief/outcome only a viz label → real `brief.md` (phase pre/post) + observations.
4. Units carry no `reviews`/`approvals`/`iterations`/`input_witnesses` → add them.
5. `state.json` snapshot → DROP it entirely; move the pure `derive_station_phase`
   into the shared `darkrun-core` crate and have the engine, HTTP, and desktop all
   run it over on-disk FM (today the engine writes the snapshot, others read it —
   the cursor logic is NOT shared across the three; §2c).
6. Run-level `witnesses.json` → per-slot witnesses.
7. No `decisions.jsonl`, no `elaboration.md`, no per-station `knowledge`/`discovery`.
8. No three-track cursor priority / no multi-layer loop guards.
9. No per-unit worktrees/branches, discovery worktrees, or fix-chain worktrees.
10. No generated-prompt persistence under `~/.darkrun/projects/.../runs/.../prompts`.
11. No project-level override cascade for bodies; no run-level adversarial reviewers.
12. No input/output 3-point verification; no scope validation; no DAG wave scheduler.
13. No reflection → project-overlay shared-memory loop.
14. No advisory mkdir lock model (withStationLock/withRunMainLock + stale recovery).
15. No write-guard/ownership enforcement of FSM fields + mid-merge guard suspension.
16. No checkpoint-kind→gate-resolution wiring per mode (auto/ask/external/await).
17. No run-root audit journals (action-log/write-audit/run-tick).
18. No reviewer role classes (serial vs parallel; runtime-observation post-only +
    proof-write carve-out + HOLD-if-cant-run; PR-interaction).
19. No unit `closes:` field / fix-chain feedback threading (fix Pass → closed_at).
20. Right-sizing is only optional-station keep/drop + mode trimming (no size oracle).
21. The cursor/derivation logic is NOT shared across engine/http/desktop (each reads
    the `state.json` snapshot, not one pure derive-from-disk in `darkrun-core`; §2c).
    The website + desktop projections must be updated to ingest the per-station
    structure once it lands.

## Build order (gaps → phases)

The 20 gaps land in six committable phases, each green before the next:

1. **On-disk layout + migration** — gaps 1, 2, 7, 17. `stations/<station>/{units,
   feedback,brief,artifacts,decisions}`, run-level closeout feedback, run-root
   journals, persisted artifacts; migrate `darkrun-sim`.
2. **Derive-from-disk (shared crate)** — gaps 4, 5, 15, 21. Unit/feedback FM signals
   (§2), the pure `derive_station_phase` moved into `darkrun-core` (§2c) and run by
   the engine, HTTP, AND desktop over on-disk FM; drop `state.json`; the write guard
   (§20). The HTTP/desktop/website projections switch to the per-station structure.
3. **Three-track cursor + pretick + loop guards** — gaps 8, 16. Tracks C→B→A, the
   pretick sequence (§4), the multi-layer guards (§12), checkpoint→gate resolution.
4. **Branch/worktree engine + locks** — gaps 9, 14. Per-unit/discovery/fix-chain
   worktrees on the per-station branches, engine-protected merges, the lock model.
5. **Reviews + artifacts** — gaps 3, 6, 11, 18. Spec→adversarial→quality→checkpoint
   ordering, role classes, run-level reviewers, brief/outcome/observations/
   elaboration, per-slot witnesses.
6. **Threading, overrides, prompts, shared memory** — gaps 10, 12, 13, 19, 20. The
   3-point i/o verification + scope + wave scheduler, the override cascade, prompt
   persistence, the reflection→overlay loop, the fix-loop `closes:` threading,
   right-sizing.

**This spec is complete.** It is the authoritative, build-ready target for the
run-organization rewrite — every section verified against the predecessor source,
every gap mapped to a phase.
