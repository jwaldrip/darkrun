# darkrun engine spec — run organization, derivation, git, reviews

The authoritative target for the engine. Reverse-engineered from the proven
predecessor and mapped onto darkrun vocabulary (Factory>Station>Unit>Pass;
Worker; Run; Checkpoint; Reviewer; Explorer). Companion to `merge-engine.md`
(the 9 tuned merge mechanics) — this doc is the wider model.

The single load-bearing idea everything else hangs on: **a Run's state is a pure
function of on-disk signals + git branch topology, computed live every tick.**
There is no authoritative `state.json` snapshot to keep in sync — derive, don't
store. `state.json` (if kept at all) is a disposable read-through cache.

---

## 1. On-disk layout

```
.darkrun/<run>/
  run.md                         frontmatter (factory, mode, active_station, status, …) + body
  feedback/NN-slug.md            RUN-LEVEL feedback (final closeout / cross-station) — station:""
  knowledge/**.md                run-scope knowledge (explorer output, run-local)
  reflection.md                  synthesized end-of-run reflection (written before seal)
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

## 2. State derivation (no snapshot)

Station phase is derived in this order (a pure function):
`elaborate` (elaboration.md missing/unverified, or zero units) → `review` (any
unit missing a required pre-exec `reviews.<role>` stamp) → `manufacture`/execute
(any unit not at its terminal Pass beat) → `audit`/`gate` (any unit missing a
required `approvals.<role>` stamp) → past-gate (all signed, awaiting merge).

The signals live in **unit frontmatter** and **feedback frontmatter** — which
darkrun units do NOT yet carry:
- unit: `iterations[]` ({worker, started_at, result, pass}), `reviews.<role>`,
  `approvals.<role>`, `inputs[]`, `outputs[]`, `depends_on[]`, `started_at`.
- feedback: `closed_at`/`closed_by` (non-null = closed). Station iteration
  history is **derived from closed feedback**, not a separate log.

Run/SPA/desktop all read the same derivation, so they can never disagree.

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
  → manufacture (Pass loop per unit: Make→Challenge→Resolve; unit→station merge)
  → POST-exec approval:  spec → adversarial fan-out → quality_gates → brief.md(phase:post)
  → user_gate{approval} = the CHECKPOINT                ← final gate, BEFORE merge
  → observations.md                               ← AFTER the checkpoint, before merge
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
- **quality gates run at two points**: as the `quality_gates` approval actor (after
  the station's adversarial fan-out, certifying the FINAL post-fix state), and AGAIN
  at the run tick (`scope: intent`) after the run-level reviewers. Gates classify
  env-unavailable separately and can defer to CI after N non-convergent attempts.
- **the final station checkpoint fires BEFORE the final merge** (the merge is
  unreachable while any approval stamp is null).

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

---

## Gap summary (darkrun today → this spec)

1. Flat `units/`/`feedback/` → per-station dirs + run-level closeout feedback.
2. Locked artifacts never written → persist under `stations/<station>/artifacts/`.
3. Brief/outcome only a viz label → real `brief.md` (phase pre/post) + observations.
4. Units carry no `reviews`/`approvals`/`iterations`/`input_witnesses` → add them.
5. `state.json` snapshot → derive phase from on-disk signals (cache at most).
6. Run-level `witnesses.json` → per-slot witnesses.
7. No `decisions.jsonl`, no `elaboration.md`, no per-station `knowledge`/`discovery`.
8. No three-track cursor priority / no multi-layer loop guards.
9. No per-unit worktrees/branches, discovery worktrees, or fix-chain worktrees.
10. No generated-prompt persistence under `~/.darkrun/projects/.../runs/.../prompts`.
11. No project-level override cascade for bodies; no run-level adversarial reviewers.
12. No input/output 3-point verification; no scope validation; no DAG wave scheduler.
13. No reflection → project-overlay shared-memory loop.
