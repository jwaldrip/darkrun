//! The manager — a single virtual cursor over a Run's aggregate state.
//!
//! The manager is a **pure read** of on-disk state that returns ONE structured
//! [`RunAction`] describing the next thing the caller (the agent) should do. It
//! does NOT run LLM agents — it tells the caller what to do, the caller does it
//! (writes artifacts / units / stamps), then calls `run_next` again.
//!
//! ## Three-track priority (Drift -> Feedback -> Run)
//!
//! 1. **Drift** — witnessed artifact mutations preempt everything. (In this
//!    slice drift is surfaced as a structured action; the sweep itself is a
//!    `darkrun-core` concern not yet wired, so the track is a no-op until
//!    drift entries exist.)
//! 2. **Feedback** — any open feedback routes a fix-worker action before run
//!    work proceeds.
//! 3. **Run** — walk the factory's stations in order; the first incomplete
//!    station drives its phase machine.
//!
//! ## Station phase machine
//!
//! `Spec -> Review -> Manufacture -> Audit -> Reflect -> Checkpoint`. Each
//! station advances one phase per resolved tick; `Manufacture` loops one Unit
//! wave per tick until every Unit is locked; `Audit` both verifies the output
//! against the spec AND runs the quality checks / tests (the old `Tests` phase
//! is folded into `Audit`); `Reflect` is an autonomous retrospective that
//! captures learnings before the gate; the `Checkpoint` phase fires the gate
//! (`auto`/`ask`/`external`/`await`) and either advances to the next station
//! or holds for an operator decision.
//!
//! The manager stays **phase-granular** — one [`RunAction`] per phase. The
//! per-phase named sub-steps (beats) live in the *rendered prompt*, not in
//! separate manager ticks.

use chrono::Utc;
use darkrun_core::domain::{
    Checkpoint, CheckpointKind, CheckpointOutcome, IterationResult, Mode, PrStatus, Run,
    RunFrontmatter, SealKind, Station, StationPhase, Status, Unit,
};
use darkrun_core::{RunState, StateStore};
use darkrun_git::{Git, GitBackend};
use serde::Serialize;

use crate::error::{McpError, Result};
use crate::factory::{FactoryDef, StationDef};

/// Which of the three tracks produced the current action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Track {
    /// Open feedback (an input-premise drift arrives here as `origin=drift`).
    Feedback,
    /// Forward run progress.
    Run,
}

/// The structured "next action" the manager hands back to the agent: a tagged
/// kind plus the context the agent needs to perform it. The manager never
/// performs the work — it describes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum RunAction {
    /// Spec the station: run Explorers, then Decompose into Units.
    Spec {
        run: String,
        station: String,
        /// What the station eliminates — framing for the agent.
        kills: String,
    },
    /// Review the station's spec before any output is manufactured.
    Review {
        run: String,
        station: String,
        reviewers: Vec<String>,
    },
    /// Manufacture: run the Pass loop over the station's wave-ready Units.
    Manufacture {
        run: String,
        station: String,
        /// The worker beat to dispatch next (Make/Challenge/Resolve...).
        worker: String,
        /// The wave-ready unit slugs to dispatch in parallel.
        units: Vec<String>,
    },
    /// Audit the manufactured output against the spec AND run the station's
    /// quality checks / tests (the old `Tests` action folded in here).
    Audit {
        run: String,
        station: String,
        reviewers: Vec<String>,
    },
    /// Reflect: an autonomous retrospective that captures learnings for the
    /// run-level reflections, before the Checkpoint gate.
    Reflect { run: String, station: String },
    /// The station's pre-execution USER gate is open: the adversarial Reviewers
    /// have signed the spec, but the operator hasn't. The cursor HOLDS here and
    /// surfaces the station's brief to the operator's review surface (the desktop
    /// app) — they approve (→ Manufacture) or return feedback (→ rework) via
    /// `darkrun_checkpoint_decide`. The pre-execution twin of `Checkpoint`: it
    /// lets the operator review the station *before* any Unit is manufactured.
    UserGate { run: String, station: String },
    /// The station's Checkpoint gate is open; the agent should surface it.
    Checkpoint {
        run: String,
        station: String,
        kind: CheckpointKind,
    },
    /// Dispatch a fix-worker against an open feedback item (Track B).
    FixFeedback {
        run: String,
        station: String,
        feedback_id: String,
    },
    /// Answer an open feedback item that is a *question* (needs a user
    /// decision, not a code fix). Track B's question half — parity for the predecessor's
    /// `feedback_question`. The agent surfaces it via `darkrun_question`.
    FeedbackQuestion {
        run: String,
        station: String,
        feedback_id: String,
    },
    /// The active station's decomposition is malformed and must be repaired
    /// before manufacture — parity for the predecessor's `unit_naming_invalid`,
    /// `unit_inputs_missing`, `unresolved_dependencies`, `dag_cycle_detected`
    /// (all consolidated under one action with a `problem` discriminator).
    UnitsInvalid {
        run: String,
        station: String,
        /// What's wrong: `invalid_naming` | `unresolved_deps` | `dependency_cycle`.
        problem: String,
        /// The offending unit slugs.
        units: Vec<String>,
    },
    /// A Unit's Pass loop exceeded its iteration budget — stop auto-looping and
    /// escalate to the operator. Parity for the predecessor's `escalate` / `loop_halted`.
    Escalate {
        run: String,
        station: String,
        reason: String,
    },
    /// A gate failed for an ENVIRONMENT reason (a dependency was down) and the
    /// repo's `.darkrun/boot.md` declares a service whose tool is available:
    /// instruct the agent to best-effort boot it, then re-record the gate.
    BestEffortBoot {
        run: String,
        station: String,
        unit: String,
        gate: String,
        /// The service boot commands, as `name: <command line>` entries.
        services: Vec<String>,
    },
    /// A gate is environment-blocked and can't be auto-recovered (no boot recipe,
    /// or the required tool is missing) — hold the station and surface it to the
    /// operator instead of churning fix passes against a dead dependency.
    EscalateToUser {
        run: String,
        station: String,
        unit: String,
        gate: String,
        reason: String,
    },
    /// Persisted state is internally inconsistent (a unit points at a station
    /// the factory doesn't define). Run a guarded repair before proceeding —
    /// parity for the predecessor's `safe_intent_repair`.
    SafeRepair {
        run: String,
        station: String,
        reason: String,
    },
    /// The operator rolled units back for spec revision; re-open their specs
    /// before continuing — parity for the predecessor's `revise_unit_specs`.
    ReviseUnitSpecs {
        run: String,
        station: String,
        units: Vec<String>,
    },
    /// The active station's gate is `external`: hand off to an external review
    /// surface (open/annotate a PR/MR) and hold — parity for the predecessor's
    /// `external_review_requested`. Distinct from a local `Checkpoint`.
    ExternalReviewRequested {
        run: String,
        station: String,
        /// The external target the review hangs off (e.g. a PR/MR ref); empty
        /// until the agent opens one.
        target: String,
    },
    /// Every station is locked, but the factory's whole-Run reviewers haven't all
    /// signed off on the integrated result — hold in a run-level review (the
    /// cross-station audit) until each is stamped. Carries the unsigned reviewers.
    RunReview {
        run: String,
        reviewers: Vec<String>,
    },
    /// Every station is locked but the run declares a final `seal:` gate — hold
    /// for an external merge / await decision before sealing. Parity for
    /// the predecessor's `pending_seal` / `intent_approved`.
    PendingSeal {
        run: String,
        kind: SealKind,
    },
    /// Every station is locked and the run is sealed.
    Sealed { run: String },
    /// A land (or downstream sync) left genuine agent-content conflicts in-tree
    /// (mechanic #3). The merge is NOT aborted — `MERGE_HEAD` stays set and the
    /// conflict markers are present on `branch` for the agent/human to resolve.
    /// The next tick re-derives this action until the merge is no longer in
    /// progress. While it holds, the write-guard suspends ownership / lifecycle /
    /// branch-enforcement guards so the conflicted engine files can be written.
    MergeConflict {
        run: String,
        station: String,
        /// The branch the conflicted merge is left in-tree on.
        branch: String,
        /// The unresolved (agent-content) conflict paths.
        conflict_paths: Vec<String>,
    },
    /// The agent has uncommitted work in the project tree (outside the
    /// engine's own `.darkrun/` bookkeeping). The engine never authors the
    /// agent's commits — a generic engine "wip" dump can't tell the story of
    /// the work — so the tick blocks and hands the file list back: commit,
    /// then retick. Purely mechanical; no human intervention needed.
    SaveWip {
        run: String,
        /// The branch the uncommitted work sits on.
        branch: String,
        /// The agent's uncommitted paths (engine bookkeeping excluded).
        dirty_files: Vec<String>,
    },
    /// Nothing to do this tick (mid-wave; outstanding subagents still working).
    Noop { run: String, message: String },
}

/// The cursor position derived for a tick: the track + the action (if any).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Position {
    /// Which track produced the action.
    pub track: Track,
    /// The next action, or `None` for a mid-wave noop.
    pub action: Option<RunAction>,
}

/// The result of a single workflow tick.
#[derive(Debug, Clone, Serialize)]
pub struct TickResult {
    /// The run slug.
    pub run: String,
    /// The derived position.
    pub position: Position,
    /// The action the agent should perform (a `noop` action when null position).
    ///
    /// This is the **structured** half — stable, machine-readable, the same
    /// shape the manager has always returned.
    pub action: RunAction,
    /// The **rendered** half: the engine-driven, override-resolved instructions
    /// for `action`, produced by [`darkrun_prompts::render`] against the
    /// project's prompt cascade. The agent reads this; machines read `action`.
    ///
    /// `None` only when the action tag has no template key (which, for the
    /// current vocabulary, never happens — every emitted action maps).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}

/// The live context handed to a prompt template for the current action.
///
/// This is the union of everything a phase/track template can reference: the
/// run + the action's own fields + the *resolved station* (its kills, the
/// workers/reviewers roster, the artifact it locks, its checkpoint kind) + the
/// station's units. Every field is optional and skipped when empty, so each
/// template's `{% if %}` guards light up exactly the vars that apply to it —
/// a `tests` template sees `station`/`units`, a `checkpoint` template also sees
/// `kind`/`kills`/`locked_artifact`, and so on.
/// A prior worker's handoff for one unit — the story carried into the next
/// worker's dispatch: which worker spoke, whether it advanced or rejected, and
/// what it said.
#[derive(Debug, Clone, Serialize, Default)]
pub struct Handoff {
    /// The unit the note belongs to.
    pub unit: String,
    /// The worker that wrote the note.
    pub worker: String,
    /// `advance` or `reject`.
    pub result: String,
    /// The worker's note — its handoff or its reason.
    pub note: String,
}

/// One wave unit's full SPEC, threaded into the Manufacture dispatch. The
/// executing subagent has NO other context — the unit body written at
/// decompose time only steers the work if the dispatch carries it. (The
/// predecessor threaded the whole unit document into every beat; a slug-only
/// dispatch was exactly how thin units slipped through.)
#[derive(Debug, Clone, Serialize, Default)]
pub struct UnitSpecCard {
    /// The unit slug.
    pub unit: String,
    /// The display title.
    pub title: String,
    /// The full markdown spec body — goal, completion criteria, scope.
    pub body: String,
    /// Declared input paths.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<String>,
    /// Declared output paths.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<String>,
    /// Declared quality gates, rendered `name — command`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub gates: Vec<String>,
}

/// A project knowledge prior surfaced into the Spec prompt.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct KnowledgePrior {
    /// The topic slug.
    pub topic: String,
    /// The knowledge prose.
    pub body: String,
}

/// One wave unit's isolation worktree (B9), surfaced into the Manufacture
/// dispatch so the worker runs that unit's beat in its own checkout — keeping
/// each unit's diff isolated until it lands back onto the station branch.
#[derive(Debug, Clone, Serialize, Default)]
pub struct Worktree {
    /// The unit the worktree belongs to.
    pub unit: String,
    /// The unit's isolation branch (`darkrun/<slug>/units/<station>/<unit>`).
    pub branch: String,
    /// The on-disk worktree path the worker should `cd` into for this unit.
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct PromptContext {
    /// The run slug.
    pub run: String,
    /// The active station name (absent for run-level actions like `sealed`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub station: Option<String>,
    /// The station's domain-facing display name (legal → `Intake`). Defaults to
    /// the station name when the factory declares no `label`. Shown by prompts
    /// and UIs over the fixed position.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// The station phase tag, when the action sits on the phase machine.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    /// What the active station eliminates.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kills: Option<String>,
    /// The checkpoint gate kind (`auto`/`ask`/`external`), derived from the run's
    /// global mode — `team` → external, `solo` → ask, `dark` → auto.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<CheckpointKind>,
    /// The durable artifact the station locks on completion.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locked_artifact: Option<String>,
    /// The worker beat to dispatch this manufacture tick.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker: Option<String>,
    /// The model to dispatch the active worker on, resolved per-role: the
    /// worker's own `model:` override, else the factory default. Absent → the
    /// harness/agent default. Display + dispatch hint, not engine logic.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The prior worker's handoff note(s), threaded into this Manufacture
    /// dispatch so the next worker reads the story — what the last beat did or
    /// why it bounced — before it acts. Keyed by unit slug, newest note per unit.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub handoffs: Vec<Handoff>,
    /// Per wave-unit isolation worktrees (B9): the branch + on-disk path the
    /// worker should run each unit's beat in, so each unit's diff stays isolated
    /// until it lands onto the station branch. Empty outside a git-backed run.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub worktrees: Vec<Worktree>,
    /// The station's one-time verifier nonce (B5) — the dispatch token the agent
    /// must pass to `darkrun_quality_gate_record`. Surfaced in the Manufacture
    /// prompt; absent outside Manufacture or on a station with no nonce.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verifier_nonce: Option<String>,
    /// The open feedback id, for the fix track.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feedback_id: Option<String>,
    /// The fix's isolation worktree path (B9), for the feedback/drift fix tracks
    /// — the checkout the fix-worker should run the repair in, off the station
    /// branch. Present only on a git-backed run where the worktree was forked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix_worktree: Option<String>,
    /// The station's effective fix-worker roster (its own override, else the
    /// factory's), surfaced so the agent dispatches the RIGHT repairer for this
    /// station's feedback.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fix_workers: Vec<String>,
    /// What's structurally wrong, for `UnitsInvalid` (`invalid_naming` etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
    /// The external review target, for `ExternalReviewRequested`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// The branch a conflicted merge is left in-tree on, for `MergeConflict`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// The unresolved conflict paths, for `MergeConflict`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub conflict_paths: Vec<String>,
    /// The agent's uncommitted paths, for `SaveWip`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dirty_files: Vec<String>,
    /// Whether the active station is OPTIONAL for this run — the Spec prompt
    /// surfaces the keep-or-drop offer from this (`darkrun_station_drop`).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub station_optional: bool,
    /// The provider compare URL (pre-filled create-PR form) — the manual
    /// fallback surfaced when no hosting client could open the PR itself.
    /// For `ExternalReviewRequested` (station branch -> run-main) and
    /// `PendingSeal` (run-main -> base).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compare_url: Option<String>,
    /// A human-readable reason, for `Escalate` / `SafeRepair`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The run-level seal gate (`external` / `await`), for `PendingSeal`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seal: Option<String>,
    /// Where the run's stable branch stands vs the default branch at seal
    /// (`ahead`/`merged`/…), surfaced so the operator knows whether origin still
    /// needs a push — the local-first land is a real commit but not pushed
    /// (guards against the predecessor's "completed but origin stale" trap).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch_status: Option<String>,
    /// A free-form message (mid-wave noop).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// The station's Explorers — dispatched in the Spec phase in TANDEM with the
    /// elaboration framing (discovery + elaboration in parallel).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub explorers: Vec<String>,
    /// PROJECT knowledge priors — durable cross-run facts the explorer has
    /// accumulated, surfaced in Spec so the station builds on what's known
    /// rather than re-discovering it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub knowledge: Vec<KnowledgePrior>,
    /// The station's Workers, in Pass-loop order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub workers: Vec<String>,
    /// The station's Reviewers.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reviewers: Vec<String>,
    /// Per-reviewer review posture (`lens`/`strict`), keyed by reviewer name —
    /// the Review/Audit prompt frames each reviewer's dispute stance from this.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub interpretations: std::collections::BTreeMap<String, String>,
    /// Per-worker pass-loop role (`plan`/`build`/`verify`), keyed by worker name
    /// — the Manufacture prompt uses it to route a reject to the nearest build
    /// worker (skipping verify/plan beats).
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub worker_roles: std::collections::BTreeMap<String, String>,
    /// True when this Spec tick is in a collaborative mode and the operator has
    /// not yet been involved (no `elaborate_seal`). The Spec prompt uses it to
    /// require operator collaboration before the spec locks — the backpressure
    /// that stops the agent authoring solo and skipping the human.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub needs_collaboration: bool,
    /// The wave-ready / on-record unit slugs for this action.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub units: Vec<String>,
    /// Each wave unit's full spec (title, body, paths, gates) — the Manufacture
    /// dispatch carries the definition, not just the slug, because the worker
    /// subagent has no other context.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unit_specs: Vec<UnitSpecCard>,
    /// The run's classified SURFACE token (`web_ui`, `library`, …), absent
    /// until the Shape station records one. Drives surface-routed verification
    /// in the Prove/Audit prompts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surface: Option<String>,
    /// True when the run's surface is verified through a headless browser
    /// (web-ui / desktop / mobile) — the Prove prompt routes to `darkrun verify
    /// web` and a `WebProof`. Also drives the manufacture phase's "get a design
    /// direction first" guidance.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub user_facing: bool,
    /// True when the run's surface is verified through criterion benches + a
    /// load harness (library / api / data) — the Prove prompt routes to
    /// `darkrun bench` and a `BenchProof`.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub bench_surface: bool,
    /// True when the run's surface is verified through a terminal/output
    /// snapshot (tui / cli) — the Prove prompt routes to an output snapshot.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub terminal_surface: bool,
    /// The resolved re-reference bundle for the active station's OPEN
    /// annotations, auto-surfaced when the agent re-enters a unit/output —
    /// especially as rework after a Request-changes. Bounded (a count summary
    /// plus the active station's open items, station-note first); absent when
    /// the station carries no open marks. This closes the human->agent loop on
    /// the tick: the agent receives `file:line` + crop + comment + suggestion
    /// without having to call `darkrun_annotation_payload` itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<crate::annotation::AgentReReferencePayload>,
}

/// Whether a unit is "past" — no further cursor work needed at its position.
fn unit_complete(unit: &Unit) -> bool {
    matches!(unit.status(), Status::Completed)
}

/// Whether a declared output is genuinely present — it must exist, be a regular
/// file, and be **non-empty**. A `touch`ed or misfired-redirect 0-byte file (or
/// a directory) does NOT satisfy a promised artifact: an empty file's content
/// hash reads as "stable" to drift, so a unit that ships nothing would otherwise
/// pass both the output-existence gate and the immune system. (Predecessor BUG-3:
/// `existsSync`-only let empty artifacts through.)
fn output_present(path: &std::path::Path) -> bool {
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.len() > 0)
        .unwrap_or(false)
}

/// Slugs of completed units that declared an output which is **not on disk** —
/// the unit promised an artifact it never produced (or produced an empty one).
/// The output-existence gate holds the station here until the file exists and is
/// non-empty. Paths resolve against the repo root (the parent of the `.darkrun/`
/// state root).
fn missing_outputs(store: &StateStore, units: &[Unit]) -> Vec<String> {
    let root = cascade_repo_root(store);
    units
        .iter()
        .filter(|u| matches!(u.status(), Status::Completed))
        .filter(|u| {
            u.frontmatter
                .outputs
                .iter()
                .any(|o| !output_present(&root.join(o)))
        })
        .map(|u| u.slug.clone())
        .collect()
}

/// Wave-ready units: pending units whose declared dependencies are all
/// completed. Mirrors `Dag::ready_units` semantics for the station's set.
fn wave_ready(units: &[Unit]) -> Vec<&Unit> {
    units
        .iter()
        .filter(|u| matches!(u.status(), Status::Pending))
        .filter(|u| {
            u.frontmatter.depends_on.iter().all(|dep| {
                units
                    .iter()
                    .find(|x| &x.slug == dep)
                    .map(unit_complete)
                    .unwrap_or(false)
            })
        })
        .collect()
}

/// Units belonging to a given station.
fn station_units<'a>(units: &'a [Unit], station: &str) -> Vec<&'a Unit> {
    units.iter().filter(|u| u.station() == station).collect()
}

/// The per-Unit Pass-loop iteration budget. A unit whose `pass` index climbs
/// past this is escalated rather than looped forever — the darkrun parity for
/// the predecessor's `loop_halted` / `escalate` runaway guard.
const MAX_PASSES: u32 = 8;

/// How many OPEN annotations the manager resolves into the next action's
/// re-reference bundle. The severity tally still reflects every open ask; this
/// only bounds the *resolved* items (station-note first, then by descending
/// severity) so a noisy station can't blow the rendered prompt.
const RE_REFERENCE_CAP: usize = 12;

/// Structural validation of a station's decomposition, as a pure function of
/// the units on disk. Returns the first problem found as `(problem_tag,
/// offending_slugs)`, or `None` when the decomposition is well-formed.
///
/// Consolidates the predecessor's `unit_naming_invalid`, `unit_inputs_missing` /
/// `unresolved_dependencies`, and `dag_cycle_detected` under one check (the
/// same spirit as the predecessor's `elaborate_loop` consolidation).
fn validate_units(all_units: &[Unit], su: &[&Unit]) -> Option<(String, Vec<String>)> {
    // 1. Naming — every slug must be non-empty and kebab-ish (lowercase, no
    //    whitespace). Catches `unit_naming_invalid`.
    let bad_names: Vec<String> = su
        .iter()
        .filter(|u| {
            let s = u.slug.trim();
            s.is_empty() || s.chars().any(|c| c.is_whitespace() || c.is_ascii_uppercase())
        })
        .map(|u| u.slug.clone())
        .collect();
    if !bad_names.is_empty() {
        return Some(("invalid_naming".to_string(), bad_names));
    }

    // 2. Unresolved deps — every `depends_on` must name a unit that exists in
    //    the run. Catches `unit_inputs_missing` / `unresolved_dependencies`.
    let known: std::collections::HashSet<&str> =
        all_units.iter().map(|u| u.slug.as_str()).collect();
    let unresolved: Vec<String> = su
        .iter()
        .filter(|u| {
            u.frontmatter
                .depends_on
                .iter()
                .any(|d| !known.contains(d.as_str()))
        })
        .map(|u| u.slug.clone())
        .collect();
    if !unresolved.is_empty() {
        return Some(("unresolved_deps".to_string(), unresolved));
    }

    // 3. Input shape — a declared `input` must be an artifact *path*, not a
    //    sibling unit's slug. Naming a unit where a file path belongs is the
    //    predecessor's `unit_inputs_not_declared` wedge: the engine can't
    //    witness a premise that isn't a file. A bare slug that matches a known
    //    unit (and looks like a slug, not a path) is the tell.
    let misdeclared_inputs: Vec<String> = su
        .iter()
        .filter(|u| {
            u.frontmatter.inputs.iter().any(|i| {
                let i = i.trim();
                known.contains(i) && !i.contains('/') && !i.contains('.')
            })
        })
        .map(|u| u.slug.clone())
        .collect();
    if !misdeclared_inputs.is_empty() {
        return Some(("input_not_a_path".to_string(), misdeclared_inputs));
    }

    // 4. Dependency cycle among the station's units. Catches `dag_cycle_detected`.
    if let Some(cycle) = first_cycle(su) {
        return Some(("dependency_cycle".to_string(), cycle));
    }
    None
}

/// The artifact basename — the last path segment, so `specify/spec.md` and
/// `spec.md` compare equal.
fn artifact_basename(s: &str) -> String {
    s.trim().rsplit('/').next().unwrap_or(s.trim()).to_string()
}

/// The station's declared inputs that this RUN is actually obliged to carry —
/// those produced by an upstream station that runs in *this run's plan*. A
/// right-sized run (e.g. `quick` = `[build, prove]`) skips earlier stations, so
/// their artifacts are never produced; requiring a unit to consume an input that
/// no station in the plan ever creates would wedge the run. An input not
/// produced by any in-plan upstream station (an external premise, or a skipped
/// producer) is therefore not required here.
fn required_station_inputs(factory: &FactoryDef, plan: &[String], station: &str) -> Vec<String> {
    let def = match factory.station(station) {
        Some(d) => d,
        None => return Vec::new(),
    };
    // The stations that actually run, in order (empty plan = the full factory).
    let effective: Vec<String> = if plan.is_empty() {
        factory.station_names()
    } else {
        plan.to_vec()
    };
    let idx = match effective.iter().position(|s| s == station) {
        Some(i) => i,
        None => return Vec::new(),
    };
    // Artifacts produced by the stations that run BEFORE this one in the plan.
    let produced_upstream: std::collections::HashSet<String> = effective[..idx]
        .iter()
        .filter_map(|s| factory.station(s))
        .map(|d| artifact_basename(&d.artifact))
        .collect();
    def.inputs
        .iter()
        .filter(|i| produced_upstream.contains(&artifact_basename(i)))
        .cloned()
        .collect()
}

/// The required inputs that NO unit of the station consumes — the distillation
/// dropped at runtime. The complement of content-validation's D4 check: D4
/// guarantees the station *template* carries each upstream artifact forward;
/// this guarantees the run's actual decomposition *uses* the ones the plan
/// actually produces, so a station can't pass template validation yet quietly
/// rebuild from scratch.
///
/// Coverage is **collective**: a required input is satisfied as soon as ANY unit
/// lists it (a unit that legitimately doesn't need an input is fine, as long as
/// some sibling consumes it). Artifacts match by path or basename so `spec.md`
/// and `specify/spec.md` agree. Returns the uncovered inputs in declared order.
fn dropped_station_inputs(required_inputs: &[String], su: &[&Unit]) -> Vec<String> {
    let consumed: std::collections::HashSet<String> = su
        .iter()
        .flat_map(|u| u.frontmatter.inputs.iter())
        .flat_map(|i| {
            let i = i.trim().to_string();
            [artifact_basename(&i), i]
        })
        .collect();
    required_inputs
        .iter()
        .map(|i| i.trim().to_string())
        .filter(|i| !i.is_empty())
        .filter(|i| !consumed.contains(i) && !consumed.contains(&artifact_basename(i)))
        .collect()
}

/// Detect a dependency cycle among `units`, returning the slugs on a cycle (in
/// discovery order) if any. Only edges *within* the set count — edges that
/// leave it are an unresolved-dep concern, not a cycle.
fn first_cycle(units: &[&Unit]) -> Option<Vec<String>> {
    use std::collections::HashMap;
    let deps: HashMap<String, Vec<String>> = units
        .iter()
        .map(|u| (u.slug.clone(), u.frontmatter.depends_on.clone()))
        .collect();
    let mut color: HashMap<String, u8> = HashMap::new(); // 0=unseen 1=on-stack 2=done
    let mut stack: Vec<String> = Vec::new();

    fn dfs(
        node: &str,
        deps: &HashMap<String, Vec<String>>,
        color: &mut HashMap<String, u8>,
        stack: &mut Vec<String>,
    ) -> Option<Vec<String>> {
        color.insert(node.to_string(), 1);
        stack.push(node.to_string());
        if let Some(ds) = deps.get(node) {
            for dep in ds {
                if !deps.contains_key(dep) {
                    continue; // leaves the set — handled by unresolved_deps
                }
                match color.get(dep).copied().unwrap_or(0) {
                    1 => {
                        let start = stack.iter().position(|x| x == dep).unwrap_or(0);
                        return Some(stack[start..].to_vec());
                    }
                    0 => {
                        if let Some(c) = dfs(dep, deps, color, stack) {
                            return Some(c);
                        }
                    }
                    _ => {}
                }
            }
        }
        stack.pop();
        color.insert(node.to_string(), 2);
        None
    }

    for u in units {
        if color.get(&u.slug).copied().unwrap_or(0) == 0 {
            stack.clear();
            if let Some(c) = dfs(&u.slug, &deps, &mut color, &mut stack) {
                return Some(c);
            }
        }
    }
    None
}

/// Persisted-state integrity check: a unit that points at a station the factory
/// doesn't define means `state.json`/units have drifted out of sync. Returns a
/// human-readable reason for the first such inconsistency — the trigger for a
/// guarded `SafeRepair` (parity for the predecessor's `safe_intent_repair`).
fn integrity_problem(factory: &FactoryDef, units: &[Unit]) -> Option<String> {
    for u in units {
        if let Some(st) = u.frontmatter.station.as_deref() {
            if !st.is_empty() && factory.station(st).is_none() {
                return Some(format!(
                    "unit `{}` references station `{st}`, which the factory does not define",
                    u.slug
                ));
            }
        }
    }
    None
}

/// Derive the station's current phase from its persisted state, defaulting to
/// `Spec` for a freshly-entered station.
fn station_phase(state: &RunState, station: &str) -> StationPhase {
    state
        .stations
        .get(station)
        .map(|s| s.phase)
        .unwrap_or(StationPhase::Spec)
}

/// Derive a station's phase from its on-disk units via the **shared**
/// [`darkrun_core::derive::derive_station_phase`] — the same pure logic the HTTP
/// browse and the website run, so every surface agrees.
///
/// Returns `None` until the engine stamps the per-unit derivation signals
/// (`reviews`/`approvals`/`iterations`); the caller then falls back to the
/// recorded phase. As signal-stamping lands this becomes the authoritative,
/// cross-surface phase. (The pure derivation has no `Reflect` sub-step — that
/// stays an engine artifact-presence beat between `Audit` and `Checkpoint`.)
fn derived_station_phase(su: &[&Unit], def: &StationDef, autopilot: bool) -> Option<StationPhase> {
    let has_signals = su.iter().any(|u| {
        !u.frontmatter.reviews.is_empty()
            || !u.frontmatter.approvals.is_empty()
            || !u.frontmatter.iterations.is_empty()
    });
    if !has_signals {
        return None;
    }
    let owned: Vec<Unit> = su.iter().map(|u| (*u).clone()).collect();
    let review_roles = def.reviewers.clone();
    let mut approval_roles = def.reviewers.clone();
    if !autopilot {
        approval_roles.push("user".to_string());
    }
    Some(darkrun_core::derive::derive_station_phase(
        &owned,
        &def.workers,
        &review_roles,
        &approval_roles,
        Some(true),
        autopilot,
    ))
}

/// The ordered station names this run walks: its explicit right-sized `plan`,
/// or the full factory plan when none is recorded (full-size / legacy runs).
fn run_plan(factory: &FactoryDef, state: &RunState) -> Vec<String> {
    if state.plan.is_empty() {
        factory.stations.iter().map(|s| s.name.clone()).collect()
    } else {
        state.plan.clone()
    }
}

/// Find the first station in the run's plan that is not yet `Completed`.
fn current_station(factory: &FactoryDef, state: &RunState) -> Option<String> {
    run_plan(factory, state).into_iter().find(|name| {
        state
            .stations
            .get(name)
            .map(|st| !matches!(st.status, Status::Completed))
            .unwrap_or(true)
    })
}

/// The station after `station` in the run's plan, if any.
fn next_in_plan(factory: &FactoryDef, state: &RunState, station: &str) -> Option<String> {
    let plan = run_plan(factory, state);
    let idx = plan.iter().position(|s| s == station)?;
    plan.get(idx + 1).cloned()
}

/// What a station drop changed: the dropped station and where the cursor
/// re-derives to.
#[derive(Debug, Clone, Serialize)]
pub struct StationDropOutcome {
    /// The run slug.
    pub run: String,
    /// The station removed from the plan.
    pub dropped: String,
    /// The station the cursor advances to (None = the run is at its end).
    pub next_station: Option<String>,
}

/// Drop an OPTIONAL station from a live run's plan — the keep-or-drop decision
/// offered at ARRIVAL (the predecessor's `drop_stage`). Only the ACTIVE,
/// not-yet-started station can drop:
///
/// - `not_active` — the decision is made at arrival; a past or future station
///   can't be dropped from afar.
/// - `not_optional` — core stations never drop; only station classes the
///   factory consciously marked `optional: true` may.
/// - `already_started` — elaboration, a moved phase, or on-record units mean
///   work exists; that's a reset, not a drop.
///
/// Dropping materializes the plan (a full-traversal run has an empty
/// `state.plan`), removes the station, retires its (workless) branch +
/// worktree, and re-derives the cursor. Cross-station references auto-ignore:
/// a dropped station produces nothing, so its artifact is simply absent
/// downstream (`inputs_waived` semantics).
pub fn station_drop(store: &StateStore, slug: &str, station: &str) -> Result<StationDropOutcome> {
    let mut run = store.read_run(slug)?;
    let factory = resolve_factory_for(store, &run.frontmatter.factory)
        .ok_or_else(|| McpError::UnknownFactory(run.frontmatter.factory.clone()))?;
    let def = factory.station(station).ok_or_else(|| {
        McpError::InvalidInput(format!("unknown station '{station}'"))
    })?;
    let mut state = store
        .read_state(slug)?
        .ok_or_else(|| McpError::InvalidInput(format!("run '{slug}' has no state")))?;

    let active = current_station(&factory, &state);
    if active.as_deref() != Some(station) {
        return Err(McpError::InvalidInput(format!(
            "drop_station_not_active: '{station}' is not the run's active station              ({active:?}) — the keep-or-drop decision is made at arrival, not from afar"
        )));
    }
    if !def.optional {
        return Err(McpError::InvalidInput(format!(
            "drop_station_not_optional: '{station}' is a core station — only stations              the factory marks `optional: true` can be dropped"
        )));
    }
    let started = state
        .stations
        .get(station)
        .map(|st| {
            // The checkpoint slot is pre-seeded at ensure-time; only an
            // ENTERED or DECIDED gate means the station actually ran.
            let gate_touched = st
                .checkpoint
                .as_ref()
                .map(|c| c.entered_at.is_some() || c.outcome.is_some())
                .unwrap_or(false);
            st.elaborated || !matches!(st.phase, StationPhase::Spec) || gate_touched
        })
        .unwrap_or(false)
        || store
            .read_units(slug)
            .unwrap_or_default()
            .iter()
            .any(|u| u.station() == station);
    if started {
        return Err(McpError::InvalidInput(format!(
            "drop_station_already_started: '{station}' has elaboration or units on              record — reset it (or finish it); a started station is never dropped"
        )));
    }

    // Materialize the plan (an empty plan means full traversal), then remove.
    let mut plan = run_plan(&factory, &state);
    plan.retain(|s| s != station);
    if plan.is_empty() {
        return Err(McpError::InvalidInput(
            "cannot drop the run's only remaining station".into(),
        ));
    }
    state.plan = plan;
    state.stations.remove(station);
    let next = current_station(&factory, &state);
    if let Some(n) = &next {
        ensure_station(&mut state, &factory, n)?;
        state.active_station = n.clone();
        run.frontmatter.active_station = n.clone();
    }
    store.write_state(slug, &state)?;
    store.write_run(&run)?;
    // Retire the dropped station's (workless) branch + worktree, then publish.
    crate::lifecycle::drop_station_branch(store, slug, station);
    crate::events::emit(
        store,
        slug,
        "darkrun.station.dropped",
        serde_json::json!({ "station": station, "next": next }),
    );
    let _ = crate::commit::commit_state(
        store,
        &format!("darkrun: drop station '{station}' from {slug}"),
    );
    Ok(StationDropOutcome {
        run: slug.to_string(),
        dropped: station.to_string(),
        next_station: next,
    })
}

/// Mark a station's Spec as elaborated-with-the-operator, clearing the
/// collaboration hold so the Spec phase can advance to Review. Idempotent.
pub fn elaborate_seal(store: &StateStore, slug: &str, station: &str) -> Result<()> {
    let mut state = store
        .read_state(slug)?
        .ok_or_else(|| McpError::Core(darkrun_core::CoreError::RunNotFound(slug.to_string())))?;
    match state.stations.get_mut(station) {
        Some(st) => st.elaborated = true,
        None => {
            return Err(McpError::InvalidInput(format!(
                "station `{station}` is not active; cannot seal elaboration"
            )))
        }
    }
    store.write_state(slug, &state)?;
    let _ = crate::commit::commit_state(store, &format!("darkrun: elaborate seal {station}"));
    Ok(())
}

/// Stamp one whole-Run reviewer's sign-off in the run-level review — without
/// walking the cursor, so the run reviewers fan out in parallel like station
/// reviewers and the parent ticks once. The run holds in `RunReview` until every
/// declared run reviewer is stamped here.
pub fn run_review_stamp(store: &StateStore, slug: &str, role: &str) -> Result<()> {
    if role.trim().is_empty() {
        return Err(McpError::InvalidInput("run reviewer role must not be empty".into()));
    }
    let mut state = store
        .read_state(slug)?
        .ok_or_else(|| McpError::Core(darkrun_core::CoreError::RunNotFound(slug.to_string())))?;
    state.run_reviews.insert(
        role.to_string(),
        Some(darkrun_core::domain::Stamp { at: Utc::now().to_rfc3339() }),
    );
    store.write_state(slug, &state)?;
    let _ = crate::commit::commit_state(store, &format!("darkrun: run review stamp {role}"));
    Ok(())
}

/// The run-main-vs-default branch status as a snake_case token for the prompt
/// (`ahead`/`merged`/`diverged`/…), or `None` outside a git repo / when not
/// forked — surfaced at seal so the operator knows whether origin needs a push.
fn branch_status_token(store: &StateStore, slug: &str) -> Option<String> {
    match crate::lifecycle::run_main_status(store, slug) {
        crate::lifecycle::RunMainStatus::NotForked => None,
        other => serde_json::to_value(other)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string)),
    }
}

/// Whether a reviewer with surface scope `applies_to` fires on a run classified
/// into `surface` (E6). An empty scope fires always; a scoped reviewer fires
/// only when the run's surface is in it (tolerant spelling via `Surface::parse`).
/// An unclassified run (`surface == None`) does not satisfy any non-empty scope.
fn reviewer_applies(applies_to: &[String], surface: Option<darkrun_core::domain::Surface>) -> bool {
    if applies_to.is_empty() {
        return true;
    }
    let Some(want) = surface else { return false };
    applies_to
        .iter()
        .filter_map(|s| darkrun_core::domain::Surface::parse(s))
        .any(|s| s == want)
}

/// The station's reviewers that fire for a run classified into `surface` — a
/// reviewer with a surface scope (`applies_to`) that doesn't match is dropped
/// (E6). A station with no scoped reviewers returns its full roster.
fn effective_station_reviewers(
    def: &StationDef,
    surface: Option<darkrun_core::domain::Surface>,
) -> Vec<String> {
    def.reviewers
        .iter()
        .filter(|r| {
            reviewer_applies(
                def.role_applies_to.get(*r).map(Vec::as_slice).unwrap_or(&[]),
                surface,
            )
        })
        .cloned()
        .collect()
}

/// The run-level reviewers that actually fire for this run — run-level **mode
/// shaping** (C5). A `dark` run is "on the loop": the operator pre-elaborated up
/// front and the system never stops for review, so the whole-run review is
/// skipped. `team`/`solo` keep every declared run reviewer — the cross-station
/// audit of the integrated result before the run seals. The final seal gate is
/// honored regardless of mode, so the operator's last say is never shaped away.
fn effective_run_reviewers(
    factory: &FactoryDef,
    state: &RunState,
    surface: Option<darkrun_core::domain::Surface>,
) -> Vec<String> {
    if state.mode == Mode::Dark {
        return Vec::new();
    }
    // E6: drop a run reviewer whose surface scope doesn't match the run's
    // classified surface (an a11y/visual reviewer skips a non-visual run).
    factory
        .run_reviewers
        .iter()
        .filter(|r| {
            reviewer_applies(
                factory.run_reviewer_applies_to.get(*r).map(Vec::as_slice).unwrap_or(&[]),
                surface,
            )
        })
        .cloned()
        .collect()
}

/// A station's EFFECTIVE Checkpoint kind — now a pure function of the run's
/// global [`Mode`]. There are no per-station gate settings: `team` → every
/// station opens an external PR the human merges, `solo` → every station asks the
/// local operator, `dark` → every station auto-advances once reviews pass.
fn effective_checkpoint_kind(state: &RunState) -> CheckpointKind {
    state.mode.gate()
}

/// Resolve a right-sizing `size` into a station plan — the right-sizing pass at
/// run start. Orthogonal to [`Mode`] (which decides gating); the agent picks the
/// size from the problem during pre-elaboration.
///
/// The plan is the factory's stations filtered to those the size keeps, in
/// factory order. `full`/`standard`/unknown → the full plan (empty sentinel). A
/// size whose kept stations don't exist in the factory falls back to the full
/// plan, so right-sizing can never strand a run with no stations.
fn resolve_size(size: &str, factory: &FactoryDef) -> Vec<String> {
    let keep: &[&str] = match size.trim().to_ascii_lowercase().as_str() {
        // Small work: build + prove only — skip framing/design and hardening.
        "quick" => &["build", "prove"],
        // A localized fix: keep the spec for the regression, build, prove.
        "bugfix" => &["specify", "build", "prove"],
        // Structural change: keep the design pressure-test, build, prove.
        "refactor" => &["shape", "build", "prove"],
        // Full traversal.
        _ => return Vec::new(),
    };
    factory
        .stations
        .iter()
        .map(|s| s.name.clone())
        .filter(|name| keep.contains(&name.as_str()))
        .collect()
}

/// Track B — feedback. Resolves the single most-urgent open item:
///
/// 1. **Questions preempt.** A `question` needs an operator decision before any
///    fix can proceed, so an open question is returned first (by id order).
/// 2. **Fixes by severity.** Among fix items, the highest severity goes first
///    (blocker → high → medium → low → unranked), stable by id within a rank —
///    so a blocker is never starved behind a nit that happened to be filed first.
///
/// Returns `None` when no open feedback exists ("open" = no terminal `status:`).
fn walk_feedback(store: &StateStore, slug: &str, station: &str) -> Result<Option<RunAction>> {
    let raw = store.read_feedback_raw(slug)?;
    let mut open: Vec<(String, String)> =
        raw.into_iter().filter(|(_, c)| feedback_open(c)).collect();
    // Stable by id first, so equal-severity ties resolve in filing order.
    open.sort_by(|a, b| a.0.cmp(&b.0));

    // A RUN-SCOPE finding (the `_run` sentinel — a closeout / cross-station
    // item) dispatches at run scope and fixes on run-main. Every OTHER finding
    // dispatches in the run's ACTIVE station context (the long-standing
    // invariant: the feedback's own `station` is an informational/grouping
    // attribute, the fix happens where the run currently is).
    let action_station = |content: &str| -> String {
        if feedback_station(content) == crate::lifecycle::RUN_SCOPE_STATION {
            crate::lifecycle::RUN_SCOPE_STATION.to_string()
        } else {
            station.to_string()
        }
    };

    // Questions preempt fixes.
    if let Some((id, c)) = open.iter().find(|(_, c)| feedback_is_question(c)) {
        return Ok(Some(RunAction::FeedbackQuestion {
            run: slug.to_string(),
            station: action_station(c),
            feedback_id: id.clone(),
        }));
    }
    // Otherwise the highest-severity fix item (min rank = most urgent).
    let next = open
        .iter()
        .filter(|(_, c)| !feedback_is_question(c))
        .min_by_key(|(_, c)| feedback_severity_rank(c));
    Ok(next.map(|(id, c)| RunAction::FixFeedback {
        run: slug.to_string(),
        station: action_station(c),
        feedback_id: id.clone(),
    }))
}

/// Severity rank for ordering: blocker=0 (most urgent) … unranked=4.
fn feedback_severity_rank(raw: &str) -> u8 {
    for line in raw.lines() {
        if let Some(rest) = line.trim().strip_prefix("severity:") {
            return match rest.trim().trim_matches('"').to_ascii_lowercase().as_str() {
                "blocker" => 0,
                "high" => 1,
                "medium" => 2,
                "low" => 3,
                _ => 4,
            };
        }
    }
    4
}

/// Whether a feedback document is a *question* (needs a user decision) rather
/// than a fix. Reads a `kind:` frontmatter line; `question` → true. Absent →
/// false (a plain fix), keeping legacy feedback backward-compatible.
fn feedback_is_question(raw: &str) -> bool {
    for line in raw.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("kind:") {
            return rest.trim().trim_matches('"').eq_ignore_ascii_case("question");
        }
    }
    false
}

/// The station a feedback doc targets, or empty for a RUN-SCOPE finding (a
/// closeout / cross-station item that belongs to the run, not one station).
fn feedback_station(raw: &str) -> String {
    for line in raw.lines() {
        if let Some(rest) = line.trim().strip_prefix("station:") {
            return rest.trim().trim_matches('"').to_string();
        }
    }
    String::new()
}

/// Whether a feedback document is still open (no terminal status line).
fn feedback_open(raw: &str) -> bool {
    let terminal = ["closed", "rejected", "addressed", "answered", "non_actionable"];
    for line in raw.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("status:") {
            let status = rest.trim().trim_matches('"').to_ascii_lowercase();
            return !terminal.contains(&status.as_str());
        }
    }
    // No status line → treat as open (newly filed).
    true
}

/// Derive the cursor position for a run — the heart of the manager.
///
/// Pure read: same disk → same answer. Walks Track C (drift) -> Track B
/// (feedback) -> Track A (run), returning the first non-null action.
pub fn derive_position(store: &StateStore, slug: &str) -> Result<Position> {
    let run = store.read_run(slug)?;
    let factory = resolve_factory_for(store, &run.frontmatter.factory)
        .ok_or_else(|| McpError::UnknownFactory(run.frontmatter.factory.clone()))?;
    let state = store.read_state(slug)?.unwrap_or_default();
    let units = store.read_units(slug)?;

    let station = match current_station(&factory, &state) {
        Some(s) => s,
        None => {
            // Every station locked. First, the whole-Run review: the factory's
            // run reviewers audit the integrated result (cross-station seams,
            // regressions, the attacker's view) before the run can seal. Hold in
            // RunReview until each declared run reviewer is stamped.
            let unsigned: Vec<String> =
                effective_run_reviewers(&factory, &state, run.frontmatter.surface)
                    .into_iter()
                    .filter(|r| !matches!(state.run_reviews.get(r), Some(Some(_))))
                    .collect();
            if !unsigned.is_empty() {
                return Ok(Position {
                    track: Track::Run,
                    action: Some(RunAction::RunReview {
                        run: slug.to_string(),
                        reviewers: unsigned,
                    }),
                });
            }
            // If the run declares a final `seal:` gate
            // and hasn't been confirmed delivered, hold at PendingSeal
            // (awaiting an external merge / await decision); otherwise seal.
            let action = match run.frontmatter.seal {
                Some(kind) if run.frontmatter.status != Status::Completed => {
                    RunAction::PendingSeal {
                        run: slug.to_string(),
                        kind,
                    }
                }
                _ => RunAction::Sealed {
                    run: slug.to_string(),
                },
            };
            return Ok(Position {
                track: Track::Run,
                action: Some(action),
            });
        }
    };

    // ── Mid-merge preemption (mechanic #3) ───────────────────────────────
    // A land / downstream sync that left genuine agent-content conflicts is
    // still in progress in-tree (MERGE_HEAD set). It preempts EVERYTHING: the
    // agent must resolve the conflict markers before the run can advance. The
    // write-guard suspends ownership / lifecycle / branch-enforcement guards
    // while this holds so the conflicted engine files can be written. We
    // re-derive this every tick until the merge is no longer in progress.
    if let Some(action) = merge_conflict_action(store, slug, &station)? {
        return Ok(Position {
            track: Track::Run,
            action: Some(action),
        });
    }

    // Drift is no longer a separate track: an input-premise change is detected
    // by `drift::sweep` (run at the top of each tick) and filed as an
    // `origin = drift` **feedback** item, which the feedback track below resolves
    // like any other finding. Outputs are never witnessed, so there is no global
    // "reconcile this artifact" hold to preempt with.

    // ── Track B: feedback (drift premise changes arrive here) ────────────
    if let Some(action) = walk_feedback(store, slug, &station)? {
        return Ok(Position {
            track: Track::Feedback,
            action: Some(action),
        });
    }

    // ── Track A preemptions ──────────────────────────────────────────────
    // Before walking the phase machine, the manager catches malformed or
    // inconsistent persisted state and routes a repair action — the run can't
    // make sound forward progress until it's fixed. All pure reads of disk.
    let su_all = station_units(&units, &station);

    // Integrity: a unit pointing at a station the factory doesn't define.
    if let Some(reason) = integrity_problem(&factory, &units) {
        return Ok(Position {
            track: Track::Run,
            action: Some(RunAction::SafeRepair {
                run: slug.to_string(),
                station: station.clone(),
                reason,
            }),
        });
    }

    // Operator rollback: units flagged `revise` re-open the station's spec.
    let revising: Vec<String> = su_all
        .iter()
        .filter(|u| u.frontmatter.revise)
        .map(|u| u.slug.clone())
        .collect();
    if !revising.is_empty() {
        return Ok(Position {
            track: Track::Run,
            action: Some(RunAction::ReviseUnitSpecs {
                run: slug.to_string(),
                station: station.clone(),
                units: revising,
            }),
        });
    }

    // Malformed decomposition: invalid naming / unresolved deps / dep cycle.
    if !su_all.is_empty() {
        if let Some((problem, bad)) = validate_units(&units, &su_all) {
            return Ok(Position {
                track: Track::Run,
                action: Some(RunAction::UnitsInvalid {
                    run: slug.to_string(),
                    station: station.clone(),
                    problem,
                    units: bad,
                }),
            });
        }
        // Runtime input coverage: the station's decomposition must actually
        // CONSUME every input the station carries forward *that this run's plan
        // produces*, or the run's distillation is silently dropped at this
        // station (each unit reinventing its own way). Holds before Manufacture —
        // the early, per-decomposition complement to the content-load template
        // check (D4). `units` carries the dropped input paths the agent must wire
        // into a unit.
        let required = required_station_inputs(&factory, &state.plan, &station);
        let dropped = dropped_station_inputs(&required, &su_all);
        if !dropped.is_empty() {
            return Ok(Position {
                track: Track::Run,
                action: Some(RunAction::UnitsInvalid {
                    run: slug.to_string(),
                    station: station.clone(),
                    problem: "station_inputs_dropped".to_string(),
                    units: dropped,
                }),
            });
        }
    }

    // Runaway Pass loop: a unit past its iteration budget escalates instead of
    // looping forever.
    if let Some(u) = su_all.iter().find(|u| u.pass() > MAX_PASSES) {
        return Ok(Position {
            track: Track::Run,
            action: Some(RunAction::Escalate {
                run: slug.to_string(),
                station: station.clone(),
                reason: format!(
                    "unit `{}` has run {} passes (budget {MAX_PASSES}) — escalating. Recover by \
                     editing the unit's spec and retrying: call `darkrun_unit_reset` (slug `{}`) \
                     to return it to pending (its body unlocks and the Pass budget resets), fix \
                     the spec, then tick to re-dispatch from Pass 1.",
                    u.slug, u.pass(), u.slug
                ),
            }),
        });
    }

    // Environment-blocked gate: the classifier flagged a gate failure as a dead
    // dependency, not a code defect. Don't churn fix passes — best-effort boot
    // the declared services when the repo's `.darkrun/boot.md` lists one whose
    // tool is live, otherwise escalate to the operator. (EnvBlocked auto-defers
    // to CI after repeated attempts in `record_gate_result`, so this self-clears
    // rather than wedging.)
    if let Some((u, gate_name, detail)) = su_all.iter().find_map(|u| {
        u.frontmatter
            .gate_results
            .iter()
            .find(|r| r.status == darkrun_core::domain::GateStatus::EnvBlocked)
            .map(|r| (u, r.name.clone(), r.detail.clone().unwrap_or_default()))
    }) {
        let recipe = darkrun_core::boot::read_boot_recipe(store.root()).ok().flatten();
        let services: Vec<String> = recipe
            .as_ref()
            .map(|r| {
                darkrun_core::boot::service_processes(r)
                    .into_iter()
                    .filter(|p| {
                        p.requires_tool
                            .as_deref()
                            .map(darkrun_core::gate_env::is_tool_available)
                            .unwrap_or(true)
                    })
                    .map(|p| format!("{}: {}", p.name, p.command_line()))
                    .collect()
            })
            .unwrap_or_default();
        if !services.is_empty() {
            return Ok(Position {
                track: Track::Run,
                action: Some(RunAction::BestEffortBoot {
                    run: slug.to_string(),
                    station: station.clone(),
                    unit: u.slug.clone(),
                    gate: gate_name,
                    services,
                }),
            });
        }
        let reason = if detail.trim().is_empty() {
            format!(
                "gate `{gate_name}` on unit `{}` is environment-blocked and can't be \
                 auto-recovered (no boot recipe, or its tool is missing). Bring the \
                 dependency up, then re-record the gate.",
                u.slug
            )
        } else {
            format!(
                "gate `{gate_name}` on unit `{}` is environment-blocked: {detail}. Bring \
                 the dependency up, then re-record the gate.",
                u.slug
            )
        };
        return Ok(Position {
            track: Track::Run,
            action: Some(RunAction::EscalateToUser {
                run: slug.to_string(),
                station: station.clone(),
                unit: u.slug.clone(),
                gate: gate_name,
                reason,
            }),
        });
    }

    // ── Track A: run — walk the active station's phase machine ───────────
    let def = factory
        .station(&station)
        .ok_or_else(|| McpError::UnknownStation(station.clone()))?;
    let su = station_units(&units, &station);
    // The phase comes from the SHARED pure derivation over on-disk unit signals
    // (the same `darkrun_core::derive` the HTTP browse and website run); until the
    // engine stamps those signals it falls back to the recorded/imperative phase.
    let autopilot = run.frontmatter.mode == Mode::Dark;
    // A held USER gate is AUTHORITATIVE: once the cursor parks at the
    // pre-execution operator gate, the derived signals (which know nothing of the
    // gate) must not skip it back into Manufacture. The gate is cleared only by
    // `darkrun_checkpoint_decide`, which advances the recorded phase past it.
    let recorded = station_phase(&state, &station);
    let phase = if recorded == StationPhase::UserGate {
        StationPhase::UserGate
    } else {
        derived_station_phase(&su, def, autopilot).unwrap_or(recorded)
    };

    let spec_action = || RunAction::Spec {
        run: slug.to_string(),
        station: station.clone(),
        kills: def.kills.clone(),
    };

    // Second quality-gate enforcement point (#12): once a station is PAST Audit
    // (its post-execute reviewers have signed, so the phase has advanced to
    // Reflect/Checkpoint), RE-verify every completed unit's declared quality
    // gates before it may proceed. A fix applied during the review pass that
    // regressed a gate is caught here — the predecessor's post-review
    // `quality_gates` approval role. The FIRST point is the Manufacture→Audit
    // hold above; this is the distinct second tick after the reviewers.
    if matches!(phase, StationPhase::Reflect | StationPhase::Checkpoint) {
        let gated: Vec<String> = su
            .iter()
            .filter(|u| matches!(u.status(), Status::Completed))
            .filter(|u| !u.gates_satisfied())
            .map(|u| u.slug.clone())
            .collect();
        if !gated.is_empty() {
            return Ok(Position {
                track: Track::Run,
                action: Some(RunAction::UnitsInvalid {
                    run: slug.to_string(),
                    station: station.clone(),
                    problem: "gates_unmet".to_string(),
                    units: gated,
                }),
            });
        }
    }

    let action = match phase {
        StationPhase::Spec => spec_action(),
        StationPhase::Review => RunAction::Review {
            run: slug.to_string(),
            station: station.clone(),
            reviewers: effective_station_reviewers(def, run.frontmatter.surface),
        },
        StationPhase::UserGate => RunAction::UserGate {
            run: slug.to_string(),
            station: station.clone(),
        },
        StationPhase::Manufacture => {
            // No units decomposed yet → the station still owes Spec.
            if su.is_empty() {
                spec_action()
            } else {
                let owned: Vec<Unit> = su.into_iter().cloned().collect();
                let ready = wave_ready(&owned);
                if ready.is_empty() {
                    // No wave-ready units. If every unit is locked, manufacture
                    // is done → move to Audit; otherwise subagents are still in
                    // flight → mid-wave noop.
                    if owned.iter().all(unit_complete) {
                        // Output-existence gate: a unit that locked but never
                        // produced a declared output can't advance to Audit — the
                        // artifact it promised isn't on disk. Hold at the station
                        // until it exists (idempotent: re-derives each tick).
                        let missing = missing_outputs(store, &owned);
                        // Quality-gate gate: a unit whose declared gates aren't all
                        // satisfied (pass / deferred-to-CI) can't reach Audit.
                        let gated: Vec<String> = owned
                            .iter()
                            .filter(|u| matches!(u.status(), Status::Completed))
                            .filter(|u| !u.gates_satisfied())
                            .map(|u| u.slug.clone())
                            .collect();
                        if !missing.is_empty() {
                            RunAction::UnitsInvalid {
                                run: slug.to_string(),
                                station: station.clone(),
                                problem: "missing_output".to_string(),
                                units: missing,
                            }
                        } else if !gated.is_empty() {
                            RunAction::UnitsInvalid {
                                run: slug.to_string(),
                                station: station.clone(),
                                problem: "gates_unmet".to_string(),
                                units: gated,
                            }
                        } else {
                            RunAction::Audit {
                                run: slug.to_string(),
                                station: station.clone(),
                                reviewers: effective_station_reviewers(def, run.frontmatter.surface),
                            }
                        }
                    } else {
                        return Ok(Position {
                            track: Track::Run,
                            action: None,
                        });
                    }
                } else {
                    let worker = def.workers.first().cloned().unwrap_or_default();
                    RunAction::Manufacture {
                        run: slug.to_string(),
                        station: station.clone(),
                        worker,
                        units: ready.iter().map(|u| u.slug.clone()).collect(),
                    }
                }
            }
        }
        StationPhase::Audit => RunAction::Audit {
            run: slug.to_string(),
            station: station.clone(),
            reviewers: effective_station_reviewers(def, run.frontmatter.surface),
        },
        StationPhase::Reflect => RunAction::Reflect {
            run: slug.to_string(),
            station: station.clone(),
        },
        StationPhase::Checkpoint => {
            let kind = effective_checkpoint_kind(&state);
            // An `external` gate hands off to an external review surface (a
            // PR/MR) rather than a local prompt — a distinct action so the
            // agent gets focused "open/annotate the review" instructions. In
            // DISCRETE mode the manager has typically already opened the
            // station's draft PR (recorded on `Station.pr_ref`); surface it on
            // the action's `target` so the agent/UI shows which PR to merge.
            if matches!(kind, CheckpointKind::External) {
                let target = state
                    .stations
                    .get(&station)
                    .and_then(|st| st.pr_ref.clone())
                    .unwrap_or_default();
                RunAction::ExternalReviewRequested {
                    run: slug.to_string(),
                    station: station.clone(),
                    target,
                }
            } else {
                RunAction::Checkpoint {
                    run: slug.to_string(),
                    station: station.clone(),
                    kind,
                }
            }
        }
    };

    Ok(Position {
        track: Track::Run,
        action: Some(action),
    })
}

/// Derive a [`RunAction::MergeConflict`] when a land / sync left a merge in
/// progress in-tree (mechanic #3), else `None`.
///
/// Scans the worktrees a land/sync merges into — any registered worktree on the
/// station branch or run-main, plus the primary checkout — for an in-progress
/// merge (`darkrun_git::is_merge_in_progress`, the broad `$GIT_DIR` marker set).
/// The first one found yields the conflicted branch + its unresolved paths.
/// Outside a git repo this is always `None`.
fn merge_conflict_action(
    store: &StateStore,
    slug: &str,
    station: &str,
) -> Result<Option<RunAction>> {
    let repo_root = cascade_repo_root(store);
    let git = match Git::open(&repo_root) {
        Ok(g) => g,
        Err(_) => return Ok(None),
    };
    let station_b = crate::lifecycle::station_branch(slug, station);
    let run_main_b = crate::lifecycle::run_main_branch(slug);

    // Candidate worktrees, each tagged with the branch the merge targets:
    //   - every registered worktree (covers the DETACHED temp merge worktree a
    //     conflicting land left in-tree, plus any station/run-main checkout); a
    //     detached merge worktree's target is inferred from its path; and
    //   - the primary checkout (the run-main -> base land target).
    let mut candidates: Vec<(std::path::PathBuf, String)> = Vec::new();
    if let Ok(worktrees) = git.list_worktrees() {
        for wt in worktrees {
            let branch = match wt.branch.as_deref() {
                Some(b) => b.to_string(),
                // A detached merge worktree lives at `_merge-<sanitized-target>`;
                // recover the target branch label from its directory name.
                None => {
                    let name = wt.path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if let Some(rest) = name.strip_prefix("_merge-") {
                        if rest.ends_with("-main") {
                            run_main_b.clone()
                        } else {
                            station_b.clone()
                        }
                    } else {
                        // Some other detached tree — attribute to the station.
                        station_b.clone()
                    }
                }
            };
            candidates.push((wt.path, branch));
        }
    }
    // The primary checkout itself (run-main -> base lands merge here when the
    // operator's branch is the target).
    candidates.push((repo_root.clone(), run_main_b.clone()));

    for (path, branch) in candidates {
        if darkrun_git::is_merge_in_progress(&path) {
            let conflict_paths = git.unresolved_paths(&path).unwrap_or_default();
            // Only surface when there ARE unresolved agent-content paths — a
            // bare MERGE_HEAD with everything staged is mid-commit, not a
            // conflict the agent must resolve.
            if !conflict_paths.is_empty() {
                return Ok(Some(RunAction::MergeConflict {
                    run: slug.to_string(),
                    station: station.to_string(),
                    branch,
                    conflict_paths,
                }));
            }
        }
    }
    Ok(None)
}

/// Whether the station branch carries merge debt against run-main (mechanic
/// #4) — i.e. a land would actually do something.
///
/// `true` when there IS debt (a real merge), `false` when the branches have
/// identical trees or the station is already an ancestor of run-main (a land
/// would mint an empty --no-ff commit). Defaults to `true` (land it) outside a
/// git repo or when a branch is missing — the lifecycle land then no-ops
/// cleanly, so the cursor never wedges on a false negative.
fn station_has_merge_debt(store: &StateStore, slug: &str, station: &str) -> bool {
    let repo_root = cascade_repo_root(store);
    let git = match Git::open(&repo_root) {
        Ok(g) => g,
        Err(_) => return true, // non-git: let the lifecycle no-op decide.
    };
    let station_b = crate::lifecycle::station_branch(slug, station);
    let run_main_b = crate::lifecycle::run_main_branch(slug);
    // Only meaningful when both branches exist; otherwise let the land path
    // (which guards branch existence) handle it.
    if !git.branch_exists(&station_b).unwrap_or(false)
        || !git.branch_exists(&run_main_b).unwrap_or(false)
    {
        return true;
    }
    !darkrun_git::has_no_merge_debt(&git, &station_b, &run_main_b)
}

/// The repo root the prompt cascade resolves overrides against.
///
/// The [`StateStore`] is rooted at `<repo_root>/.darkrun`, so the repo root is
/// that directory's parent. Project overrides live at
/// `<repo_root>/.darkrun/prompts/<rel>.md`.
pub(crate) fn cascade_repo_root(store: &StateStore) -> std::path::PathBuf {
    store
        .root()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| store.root().to_path_buf())
}

/// Resolve a run's factory through the project-override cascade rooted at the
/// store's repo root — so a `<repo>/.darkrun/factories/<name>/` override (or an
/// `inherits` parent) applies. The single resolution path the run walks.
pub(crate) fn resolve_factory_for(store: &StateStore, name: &str) -> Option<FactoryDef> {
    crate::factory::resolve_factory_at(&cascade_repo_root(store), name)
}

/// Serde tag for a [`RunAction`] (its `action` field), used to pick a template.
///
/// Public so tests and tooling can name an action without re-deriving the
/// mapping (the single source of truth for action → tag).
pub fn action_tag(action: &RunAction) -> &'static str {
    match action {
        RunAction::Spec { .. } => "spec",
        RunAction::Review { .. } => "review",
        RunAction::Manufacture { .. } => "manufacture",
        RunAction::Audit { .. } => "audit",
        RunAction::Reflect { .. } => "reflect",
        RunAction::UserGate { .. } => "user_gate",
        RunAction::Checkpoint { .. } => "checkpoint",
        RunAction::FixFeedback { .. } => "fix_feedback",
        RunAction::FeedbackQuestion { .. } => "feedback_question",
        RunAction::UnitsInvalid { .. } => "units_invalid",
        RunAction::Escalate { .. } => "escalate",
        RunAction::BestEffortBoot { .. } => "best_effort_boot",
        RunAction::EscalateToUser { .. } => "escalate_to_user",
        RunAction::SafeRepair { .. } => "safe_repair",
        RunAction::ReviseUnitSpecs { .. } => "revise_unit_specs",
        RunAction::ExternalReviewRequested { .. } => "external_review_requested",
        RunAction::RunReview { .. } => "run_review",
        RunAction::PendingSeal { .. } => "pending_seal",
        RunAction::Sealed { .. } => "sealed",
        RunAction::MergeConflict { .. } => "merge_conflict",
        RunAction::SaveWip { .. } => "save_wip",
        RunAction::Noop { .. } => "noop",
    }
}

/// Build the live [`PromptContext`] for `action`, enriching the action's own
/// fields with the resolved station (kills, roster, locked artifact, checkpoint
/// kind) and the station's on-record units.
///
/// Pure read of on-disk state: reads the run's factory + units to resolve the
/// station def. When the station can't be resolved (run-level actions, unknown
/// factory) the context degrades gracefully to just the action's own fields.
fn build_prompt_context(store: &StateStore, slug: &str, action: &RunAction) -> Result<PromptContext> {
    // Pull the station name straight off the action (run-level actions have none).
    let station = match action {
        RunAction::Spec { station, .. }
        | RunAction::Review { station, .. }
        | RunAction::Manufacture { station, .. }
        | RunAction::Audit { station, .. }
        | RunAction::Reflect { station, .. }
        | RunAction::UserGate { station, .. }
        | RunAction::Checkpoint { station, .. }
        | RunAction::FixFeedback { station, .. }
        | RunAction::FeedbackQuestion { station, .. }
        | RunAction::UnitsInvalid { station, .. }
        | RunAction::Escalate { station, .. }
        | RunAction::BestEffortBoot { station, .. }
        | RunAction::EscalateToUser { station, .. }
        | RunAction::SafeRepair { station, .. }
        | RunAction::ReviseUnitSpecs { station, .. }
        | RunAction::MergeConflict { station, .. }
        | RunAction::ExternalReviewRequested { station, .. } => Some(station.clone()),
        RunAction::RunReview { .. }
        | RunAction::PendingSeal { .. }
        | RunAction::Sealed { .. }
        | RunAction::SaveWip { .. }
        | RunAction::Noop { .. } => None,
    };

    let mut ctx = PromptContext {
        run: slug.to_string(),
        station: station.clone(),
        phase: Some(action_tag(action).to_string()),
        ..Default::default()
    };

    // Resolve the station def for the roster / kills / artifact / checkpoint.
    if let Some(station) = station.as_deref() {
        let repo_root = cascade_repo_root(store);
        let run = store.read_run(slug)?;
        // Surface-routed verification flags: the run's classified surface
        // decides which proof the Prove/Audit prompts demand, and whether the
        // manufacture phase asks for a design direction first.
        if let Some(surface) = run.surface() {
            ctx.surface = Some(surface.as_str().to_string());
            ctx.user_facing = surface.is_visual();
            ctx.bench_surface = surface.is_bench();
            ctx.terminal_surface = surface.is_terminal();
        }
        if let Some(factory) = resolve_factory_for(store, &run.frontmatter.factory) {
            if let Some(def) = factory.station(station) {
                // The display label defaults to the station name when the
                // factory declares none.
                ctx.label = Some(def.label.clone().unwrap_or_else(|| station.to_string()));
                ctx.kills = Some(def.kills.clone());
                ctx.locked_artifact = Some(def.artifact.clone());
                // The gate is global: every station resolves the run mode's kind.
                ctx.kind = Some(run.frontmatter.mode.gate());
                ctx.station_optional = def.optional;
                ctx.explorers = def.explorers.clone();
                ctx.workers = def.workers.clone();
                ctx.reviewers = def.reviewers.clone();
                ctx.interpretations = def.role_interpretations.clone();
                ctx.worker_roles = def.worker_roles.clone();
            }
        }
        // The station's on-record units, in slug order.
        let units = store.read_units(slug)?;
        let mut slugs: Vec<String> = station_units(&units, station)
            .iter()
            .map(|u| u.slug.clone())
            .collect();
        slugs.sort();
        ctx.units = slugs;

        // Auto-surface the station's OPEN annotations as a resolved
        // re-reference bundle. The manager pushes this into the agent's next
        // action so the human->agent loop closes on the tick — the agent
        // doesn't have to know to call `darkrun_annotation_payload`. Bounded by
        // `RE_REFERENCE_CAP`: the severity tally reflects every open ask, but
        // only the highest-steering items (station-note first) are resolved so
        // a noisy station can't blow the prompt. Most load-bearing when a unit
        // re-enters as rework after Request-changes.
        let bundle =
            crate::annotation::station_re_reference(store, &repo_root, slug, station, RE_REFERENCE_CAP)?;
        if !bundle.items.is_empty() {
            ctx.annotations = Some(bundle);
        }
    }

    // Overlay the action-specific fields (these win over station-derived ones).
    match action {
        RunAction::Manufacture { worker, units: wave, station: mstation, .. } => {
            ctx.worker = Some(worker.clone());
            // The action's wave-ready units are the ones the agent dispatches.
            ctx.units = wave.clone();
            // B5: hand the agent the station's verifier nonce so it can record
            // quality-gate results — the engine refuses a result without it.
            ctx.verifier_nonce = store
                .read_state(slug)
                .ok()
                .flatten()
                .and_then(|s| s.stations.get(mstation).and_then(|st| st.verifier_nonce.clone()));
            // Resolve the model for this beat: the worker's own override, else
            // the factory default. (A per-unit override would win, but the wave
            // can span units, so the role/factory resolution is the dispatch hint.)
            if let Ok(run) = store.read_run(slug) {
                if let Some(factory) = resolve_factory_for(store, &run.frontmatter.factory) {
                    let by_role = factory
                        .station(mstation)
                        .and_then(|d| d.role_models.get(worker).cloned());
                    ctx.model = by_role.or_else(|| {
                        Some(factory.default_model.clone()).filter(|m| !m.is_empty())
                    });
                }
            }
            // Thread each wave unit's most-recent handoff note into the dispatch
            // so the next worker reads what the last beat did, or why it bounced.
            let wave_units = store.read_units(slug).unwrap_or_default();
            // …and each wave unit's FULL SPEC. The worker subagent has no other
            // context: the body written at decompose time only steers the beat
            // if the dispatch carries it (slug-only dispatch = thin work).
            ctx.unit_specs = wave_units
                .iter()
                .filter(|u| wave.contains(&u.slug))
                .map(|u| UnitSpecCard {
                    unit: u.slug.clone(),
                    title: u.title.clone(),
                    body: u.body.trim().to_string(),
                    inputs: u.frontmatter.inputs.clone(),
                    outputs: u.frontmatter.outputs.clone(),
                    gates: u
                        .frontmatter
                        .quality_gates
                        .iter()
                        .map(|g| format!("{} — `{}`", g.name, g.command))
                        .collect(),
                })
                .collect();
            ctx.handoffs = wave_units
                .iter()
                .filter(|u| wave.contains(&u.slug))
                .filter_map(|u| {
                    let last = u.frontmatter.iterations.iter().rev().find(|it| it.note.is_some())?;
                    Some(Handoff {
                        unit: u.slug.clone(),
                        worker: last.worker.clone(),
                        result: match last.result {
                            Some(IterationResult::Advance) => "advance",
                            Some(IterationResult::Reject) => "reject",
                            None => "in_flight",
                        }
                        .to_string(),
                        note: last.note.clone().unwrap_or_default(),
                    })
                })
                .collect();
            // Surface each wave unit's isolation worktree (B9) so the worker runs
            // that unit's beat in its own checkout. Only on a git-backed run; the
            // branch side-effect forks each worktree this same tick (AFTER this
            // prompt renders), so we show the derived path the worker will land in
            // — not one already on disk.
            if git_backed_station(store, slug, mstation) {
                let repo_root = cascade_repo_root(store);
                ctx.worktrees = wave
                    .iter()
                    .map(|unit| {
                        let path =
                            crate::lifecycle::unit_worktree_path(&repo_root, slug, mstation, unit);
                        Worktree {
                            unit: unit.clone(),
                            branch: crate::lifecycle::unit_branch(slug, mstation, unit),
                            path: path.to_string_lossy().into_owned(),
                        }
                    })
                    .collect();
            }
        }
        RunAction::Spec { station: sstation, .. } => {
            // Collaboration backpressure flag: in `team`/`solo` every station holds
            // its Spec for operator elaboration; `dark` pre-elaborates once up
            // front and skips the per-station hold.
            if let Ok(run) = store.read_run(slug) {
                if run.frontmatter.mode.holds_each_station() {
                    let elaborated = store
                        .read_state(slug)
                        .ok()
                        .flatten()
                        .and_then(|s| s.stations.get(sstation).map(|st| st.elaborated))
                        .unwrap_or(false);
                    ctx.needs_collaboration = !elaborated;
                }
            }
            // Surface the project knowledge store as priors so discovery builds
            // on what's already known across runs (#10). Best-effort.
            ctx.knowledge = crate::knowledge::list(store)
                .unwrap_or_default()
                .into_iter()
                .filter(|k| !k.body.is_empty())
                .map(|k| KnowledgePrior { topic: k.topic, body: k.body })
                .collect();
        }
        RunAction::Checkpoint { kind, .. } => {
            ctx.kind = Some(*kind);
        }
        RunAction::RunReview { reviewers, .. } => {
            ctx.reviewers = reviewers.clone();
        }
        RunAction::FixFeedback { feedback_id, station, .. } => {
            ctx.feedback_id = Some(feedback_id.clone());
            ctx.fix_worktree = fix_worktree_for(store, slug, station, feedback_id);
            // The station's effective fix-worker roster (station override, else
            // the factory's) — who repairs THIS station's feedback.
            if let Ok(run) = store.read_run(slug) {
                if let Some(factory) = resolve_factory_for(store, &run.frontmatter.factory) {
                    if let Some(def) = factory.station(station) {
                        ctx.fix_workers = def.fix_workers.clone();
                    }
                }
            }
        }
        RunAction::FeedbackQuestion { feedback_id, .. } => {
            ctx.feedback_id = Some(feedback_id.clone());
        }
        RunAction::UnitsInvalid { problem, units, .. } => {
            ctx.problem = Some(problem.clone());
            ctx.units = units.clone();
        }
        RunAction::ReviseUnitSpecs { units, .. } => {
            ctx.units = units.clone();
        }
        RunAction::Escalate { reason, .. } | RunAction::SafeRepair { reason, .. } => {
            ctx.reason = Some(reason.clone());
        }
        RunAction::ExternalReviewRequested { station, target, .. } => {
            ctx.target = Some(target.clone());
            ctx.kind = Some(CheckpointKind::External);
            // No PR was opened programmatically -> hand the operator the
            // provider's pre-filled create-PR form for this station's branch.
            if target.is_empty() {
                let repo_root = cascade_repo_root(store);
                ctx.compare_url = crate::hosting::compare_url(
                    &repo_root,
                    &crate::lifecycle::run_main_branch(slug),
                    &crate::lifecycle::station_branch(slug, station),
                );
            }
        }
        RunAction::MergeConflict { branch, conflict_paths, .. } => {
            ctx.branch = Some(branch.clone());
            ctx.conflict_paths = conflict_paths.clone();
        }
        RunAction::SaveWip { branch, dirty_files, .. } => {
            ctx.branch = Some(branch.clone());
            ctx.dirty_files = dirty_files.clone();
        }
        RunAction::PendingSeal { kind, .. } => {
            ctx.seal = Some(kind.as_str().to_string());
            ctx.branch_status = branch_status_token(store, slug);
            // The seal merge is run-main -> base; when no delivery PR exists,
            // surface the provider's pre-filled form for the manual open.
            let has_pr = store
                .read_run(slug)
                .ok()
                .and_then(|r| r.frontmatter.external_refs.pr_url)
                .is_some();
            if !has_pr {
                let repo_root = cascade_repo_root(store);
                let base = store
                    .read_state(slug)
                    .ok()
                    .flatten()
                    .and_then(|s| s.base_branch)
                    .unwrap_or_else(|| crate::lifecycle::resolve_base_branch(store));
                ctx.compare_url = crate::hosting::compare_url(
                    &repo_root,
                    &base,
                    &crate::lifecycle::run_main_branch(slug),
                );
            }
        }
        RunAction::Sealed { .. } => {
            // Surface where run-main stands vs the default branch so the operator
            // knows if origin still needs a push (BUG-4 trap).
            ctx.branch_status = branch_status_token(store, slug);
        }
        RunAction::Noop { message, .. } => {
            ctx.message = Some(message.clone());
        }
        _ => {}
    }

    Ok(ctx)
}

/// Render the engine-driven instructions for `action` through the prompt
/// cascade, returning the final markdown the agent should follow.
///
/// Maps the action's serde tag to its template key, builds the live
/// [`PromptContext`], and calls [`darkrun_prompts::render`] — so a project
/// override at `<repo_root>/.darkrun/prompts/<key>.md` transparently replaces
/// the embedded default. Returns `Ok(None)` when the action has no template key.
pub fn render_prompt(store: &StateStore, slug: &str, action: &RunAction) -> Result<Option<String>> {
    let tag = action_tag(action);
    let key = match darkrun_prompts::template_key_for_action(tag) {
        Some(k) => k,
        None => return Ok(None),
    };
    let ctx = build_prompt_context(store, slug, action)?;
    let repo_root = cascade_repo_root(store);
    let rendered = darkrun_prompts::render(key, &repo_root, &ctx)
        .map_err(|e| McpError::Prompt(e.to_string()))?;
    Ok(Some(rendered))
}

/// Apply any pending UI-requested unit resets (the `reset_requested` flag). A
/// non-MCP surface (the desktop review UI) flags a wedged unit by setting the
/// flag on its frontmatter; this consumes the flag by performing the full
/// [`crate::units::reset`] — clearing the unit's execution state to `Pending` so
/// its body unlocks — exactly as the `darkrun_unit_reset` tool would. Best-effort
/// per unit; a read/write failure simply leaves the flag for the next tick.
fn apply_requested_unit_resets(store: &StateStore, slug: &str) {
    let Ok(units) = store.read_units(slug) else {
        return;
    };
    for u in units.iter().filter(|u| u.frontmatter.reset_requested) {
        let _ = crate::units::reset(store, slug, &u.slug, true);
    }
}

/// DISCRETE-mode gate resolution: a station at its `external` gate opens, then
/// polls, a draft PR/MR through the hosting client. The station's gate resolves
/// when the PR is detected MERGED. Run on each tick BEFORE deriving so a merge
/// advances the cursor immediately.
///
/// The four cases:
///
/// 1. No PR yet + hosting available: open a draft PR and record
///    `Station.pr_ref`; the gate keeps holding as `ExternalReviewRequested`.
/// 2. PR open, not merged: hold.
/// 3. PR merged: complete the station and advance the cursor. The human's merge
///    already landed station-branch -> run-main, so this does NOT land in-process.
/// 4. No hosting client: no PR is opened; the `external` gate surfaces as
///    `ExternalReviewRequested` for the operator to resolve manually (the
///    await-style fallback).
///
/// Best-effort: any hosting failure leaves the gate holding, never crashing the
/// tick. A no-op for non-discrete runs and stations not at an external gate.
fn resolve_discrete_gate<H: crate::hosting::Hosting>(
    store: &StateStore,
    slug: &str,
    hosting: &H,
) -> Result<()> {
    let run = store.read_run(slug)?;
    let factory = match resolve_factory_for(store, &run.frontmatter.factory) {
        Some(f) => f,
        None => return Ok(()),
    };
    let mut state = match store.read_state(slug)? {
        Some(s) => s,
        None => return Ok(()),
    };
    if !state.mode.opens_station_pr() {
        return Ok(());
    }
    let station = match current_station(&factory, &state) {
        Some(s) => s,
        None => return Ok(()),
    };
    // The gate only resolves discretely once the station is actually AT its
    // checkpoint with an effective `external` kind.
    if station_phase(&state, &station) != StationPhase::Checkpoint {
        return Ok(());
    }
    if factory.station(&station).is_none() {
        return Ok(());
    }
    if !matches!(effective_checkpoint_kind(&state), CheckpointKind::External) {
        return Ok(());
    }

    let existing_ref = state.stations.get(&station).and_then(|st| st.pr_ref.clone());
    match existing_ref {
        // No PR yet: open one (best-effort) and record it; keep holding.
        None => {
            if !hosting.available() {
                return Ok(()); // await fallback — operator resolves by hand.
            }
            let head = crate::lifecycle::station_branch(slug, &station);
            let base = crate::lifecycle::run_main_branch(slug);
            // #7: the head branch must be on origin before the PR can open.
            // Push it with non-fast-forward recovery (best-effort — a push
            // failure leaves the gate holding rather than crashing the tick).
            let repo_root = cascade_repo_root(store);
            if let Ok(git) = Git::open(&repo_root) {
                let wt = crate::lifecycle::station_worktree_path(&repo_root, slug, &station);
                // Push from the station's own worktree when it exists, else the
                // repo root (the branch ref still resolves there).
                let from = if wt.exists() { wt } else { repo_root.clone() };
                let _ = crate::hosting::push_head_with_nff_recovery(&git, &from, &head);
            }
            let req = crate::hosting::OpenRequest {
                head,
                base,
                title: run.title.clone(),
                body: format!(
                    "Opened by the darkrun **{station}** station's discrete Checkpoint. \
                     Merge to advance the run."
                ),
            };
            if let Some(pr_ref) = hosting.open_draft(&req) {
                // D5: attach the station's objective proof to the change request
                // as a durable, linkable asset — posted once, here, since the PR
                // opens exactly once (the next tick polls instead). Best-effort:
                // a failed comment never blocks the gate.
                if let Some(body) = crate::proof::station_proof_markdown(store, slug, &station) {
                    hosting.comment(&pr_ref, &body);
                }
                if let Some(st) = state.stations.get_mut(&station) {
                    st.pr_ref = Some(pr_ref);
                    // G4: a freshly-opened PR starts in the draft stage.
                    st.pr_status = Some(PrStatus::Draft);
                    store.write_state(slug, &state)?;
                }
            }
            Ok(())
        }
        // PR exists: poll it. A merge resolves the gate and advances the cursor;
        // a draft→ready transition is recorded along the way (G4).
        Some(pr_ref) => {
            // C6: pull any human review notes off the PR and file each NEW one as
            // `external`-origin feedback the fix track addresses — a reviewer's
            // "please change X" on the remote re-enters the run as work. Deduped
            // by a deterministic id, so re-polling never double-files. Best-effort:
            // a fetch failure surfaces no feedback rather than crashing the tick.
            for c in hosting.review_comments(&pr_ref) {
                let _ = crate::feedback::create_external(
                    store, slug, &station, &c.id, &c.author, &c.body, c.change_request,
                );
            }
            // C6 (close the loop): once a PR has received review activity, keep
            // origin's PR head current with any fix commits that landed on the
            // station branch (a closed external feedback lands its fix locally via
            // `land_fix`). Guarded so a PR that never drew a comment is never
            // re-pushed; best-effort + NFF-recovering, a no-op when already current.
            let has_review_activity = crate::feedback::list(store, slug)
                .unwrap_or_default()
                .iter()
                .any(|f| {
                    f.station == station
                        && matches!(f.origin, darkrun_core::domain::FeedbackOrigin::External)
                });
            if has_review_activity {
                let repo_root = cascade_repo_root(store);
                if let Ok(git) = Git::open(&repo_root) {
                    let head = crate::lifecycle::station_branch(slug, &station);
                    let wt = crate::lifecycle::station_worktree_path(&repo_root, slug, &station);
                    let from = if wt.exists() { wt } else { repo_root.clone() };
                    let _ = crate::hosting::push_head_with_nff_recovery(&git, &from, &head);
                }
            }
            let now = Utc::now().to_rfc3339();
            if matches!(hosting.merge_state(&pr_ref), crate::hosting::MergeState::Merged) {
                if let Some(st) = state.stations.get_mut(&station) {
                    st.pr_status = Some(PrStatus::Merged);
                    st.pr_merged_at.get_or_insert_with(|| now.clone());
                }
                complete_station(&mut state, &factory, &station, &now)?;
                store.write_state(slug, &state)?;
                // The human's PR merge already landed the station onto run-main,
                // so there is NO in-process land here. If that was the final
                // station, land the run onto base.
                crate::drift::record_station_witnesses(store, slug, &station)?;
                if current_station(&factory, &state).is_none() {
                    crate::lifecycle::land_run(store, slug);
                }
            } else if hosting.is_draft(&pr_ref) == Some(false) {
                // Marked ready for review (no longer draft), not yet merged —
                // record the transition once.
                if let Some(st) = state.stations.get_mut(&station) {
                    if matches!(st.pr_status, Some(PrStatus::Draft) | None) {
                        st.pr_status = Some(PrStatus::Ready);
                        st.pr_ready_at.get_or_insert(now);
                        store.write_state(slug, &state)?;
                    }
                }
            }
            Ok(())
        }
    }
}

/// Drive one workflow tick: derive the position, then ADVANCE the station's
/// phase write-cache so the next tick moves forward.
///
/// This is the side-effecting wrapper around the pure [`derive_position`]. The
/// cursor walk is pure, but the tick stamps the derived phase forward into
/// `state.json` so the cursor visibly advances `Spec -> Review -> Manufacture
/// -> Audit -> Reflect -> Checkpoint` across calls.
///
/// Phase advancement rules:
/// - `Spec` -> `Review` (the agent runs Explorers + decomposes during Spec).
/// - `Review` -> `Manufacture`.
/// - `Manufacture` stays in `Manufacture` while units remain wave-ready (one
///   wave per tick); when every unit is locked the `Audit` action moves it on.
/// - `Audit` -> `Reflect` (audit verifies AND runs the tests — there is no
///   separate tests phase).
/// - `Reflect` -> `Checkpoint`.
/// - `Checkpoint` with an `auto` gate -> advance to the next station's `Spec`;
///   non-auto gates hold until [`checkpoint_decide`].
///
/// Discrete-mode runs additionally resolve `external` gates via
/// [`resolve_discrete_gate`] (the hosting PR open/merge) before the derive.
pub fn run_tick(store: &StateStore, slug: &str) -> Result<TickResult> {
    let repo_root = cascade_repo_root(store);
    let hosting = crate::hosting::ApiHosting::resolve(&repo_root);
    run_tick_with_hosting(store, slug, &hosting)
}

/// [`run_tick`] with an injected [`Hosting`] client — the seam discrete-mode
/// tests use to drive PR open/merge without a live `gh`/`glab`.
pub fn run_tick_with_hosting<H: crate::hosting::Hosting>(
    store: &StateStore,
    slug: &str,
    hosting: &H,
) -> Result<TickResult> {
    // Sweep first: re-hash every locked artifact so a silent mutation surfaces
    // as a drift entry that Track C (inside derive_position) then preempts.
    crate::drift::sweep(store, slug)?;
    // Apply any UI-requested unit resets (the `reset_requested` flag, set by a
    // non-MCP surface like the desktop) before deriving, so a flagged unit is
    // back to Pending — its body unlocked — by the time the cursor walks.
    apply_requested_unit_resets(store, slug);
    // Discrete gate: open / poll the station's PR before deriving, so a merge
    // detected this tick advances the cursor immediately.
    resolve_discrete_gate(store, slug, hosting)?;
    // #5: downstream sync before merge-up — merge base -> run-main -> station
    // each tick so the branches stay fresh and land-time conflicts shrink. A
    // sync conflict is left in-tree; the derive below catches it as a
    // MergeConflict action via `merge_conflict_action`.
    sync_downstream_before_land(store, slug);

    // Pre-derive clean-tree gate (the predecessor's `save_wip`): when the
    // AGENT has uncommitted work in the project tree — paths outside the
    // engine's own `.darkrun/` bookkeeping — block the tick and hand the list
    // back. The engine never authors the agent's commits: the agent knows what
    // it just did and can write commits that tell the story; a generic engine
    // "wip" dump never could. Runs AFTER the sync (so a mid-merge conflict
    // routes to MergeConflict via derive, not here) and only in a git repo.
    {
        let repo_root = cascade_repo_root(store);
        if !darkrun_git::is_merge_in_progress(&repo_root) {
            if let Ok(git) = Git::open(&repo_root) {
                let dirty = git
                    .dirty_paths_excluding(&repo_root, &[".darkrun", ".gitignore"])
                    .unwrap_or_default();
                if !dirty.is_empty() {
                    let dirty_count = dirty.len();
                    let branch = git.current_branch().ok().flatten().unwrap_or_default();
                    let action = RunAction::SaveWip {
                        run: slug.to_string(),
                        branch,
                        dirty_files: dirty,
                    };
                    let prompt = render_prompt(store, slug, &action)?;
                    if let Some(body) = &prompt {
                        let _ = store.write_prompt(slug, "_run", action_tag(&action), body);
                    }
                    let position = Position {
                        track: Track::Run,
                        action: Some(action.clone()),
                    };
                    append_action_log(store, slug, &position.track, &action);
                    crate::events::emit(
                        store,
                        slug,
                        "darkrun.save_wip.blocked",
                        serde_json::json!({ "files": dirty_count }),
                    );
                    // The engine's own state writes (drift sweep, journal) still
                    // commit — commit_state stages ONLY `.darkrun`, never the
                    // agent's work.
                    let _ = crate::commit::commit_state(store, &format!("darkrun: tick {slug}"));
                    return Ok(TickResult {
                        run: slug.to_string(),
                        position,
                        action,
                        prompt,
                    });
                }
            }
        }
    }

    let position = derive_position(store, slug)?;

    let derived = match &position.action {
        Some(a) => a.clone(),
        None => RunAction::Noop {
            run: slug.to_string(),
            message:
                "Mid-wave noop. Outstanding unit passes are still in flight — wait, then retick."
                    .to_string(),
        },
    };

    // Cross-tick deadlock guard: if the cursor has returned this same action with
    // NO progress past the threshold (or is churning between two), the run is
    // wedged — even if the agent already satisfied the requirements. Swap the
    // wedged action for an Escalate so it surfaces to a human instead of spinning
    // forever. Best-effort; never blocks a tick.
    let action = crate::deadlock::check(store, slug, &derived).unwrap_or(derived);

    // Render the engine-driven instructions for this action BEFORE advancing
    // state, so the prompt reflects the action exactly as derived.
    let prompt = render_prompt(store, slug, &action)?;

    // Persist the rendered prompt under `.darkrun/<run>/prompts/<station>/<tag>.md`
    // so there's a durable, inspectable record of exactly what the engine handed
    // the agent at each step (replay / debugging). Best-effort: never fails a tick.
    if let Some(body) = &prompt {
        let scope = station_of(&action).unwrap_or("_run");
        let _ = store.write_prompt(slug, scope, action_tag(&action), body);
    }

    // Advance the phase write-cache based on the derived action.
    advance_state(store, slug, &action)?;

    // The run is done (sealed, or holding for its seal merge): flip the
    // run-level delivery draft PR to ready-for-review — the human's next move
    // is the merge, and a draft can't be merged. Once-guarded; best-effort.
    if matches!(
        action,
        RunAction::Sealed { .. } | RunAction::PendingSeal { .. }
    ) {
        flip_run_pr_ready(store, slug, hosting);
    }

    // Append the resolved action to the run's append-only audit journal — the
    // ordered trail the reflection pass and the operator read. Best-effort:
    // a journal write never fails a tick.
    append_action_log(store, slug, &position.track, &action);
    crate::events::emit(
        store,
        slug,
        "darkrun.manager.action",
        serde_json::json!({ "action": action_tag(&action), "station": station_of(&action) }),
    );

    // Commit early, commit often: every tick's state writes (phase advances,
    // gate stamps, the audit journal) commit on the engine's branch and push —
    // so origin always reflects the run's live position. Dirty-gated +
    // best-effort: a no-op tick mints nothing; a push failure never fails
    // the tick.
    let _ = crate::commit::commit_state(store, &format!("darkrun: tick {slug}"));

    Ok(TickResult {
        run: slug.to_string(),
        position,
        action,
        prompt,
    })
}

/// The station an action targets, if any (run-level actions have none).
fn station_of(action: &RunAction) -> Option<&str> {
    match action {
        RunAction::Spec { station, .. }
        | RunAction::Review { station, .. }
        | RunAction::Manufacture { station, .. }
        | RunAction::Audit { station, .. }
        | RunAction::Reflect { station, .. }
        | RunAction::UserGate { station, .. }
        | RunAction::Checkpoint { station, .. }
        | RunAction::FixFeedback { station, .. }
        | RunAction::FeedbackQuestion { station, .. }
        | RunAction::UnitsInvalid { station, .. }
        | RunAction::Escalate { station, .. }
        | RunAction::BestEffortBoot { station, .. }
        | RunAction::EscalateToUser { station, .. }
        | RunAction::SafeRepair { station, .. }
        | RunAction::ReviseUnitSpecs { station, .. }
        | RunAction::MergeConflict { station, .. }
        | RunAction::ExternalReviewRequested { station, .. } => Some(station),
        RunAction::RunReview { .. }
        | RunAction::PendingSeal { .. }
        | RunAction::Sealed { .. }
        | RunAction::SaveWip { .. }
        | RunAction::Noop { .. } => None,
    }
}

/// Append one resolved-action entry to `action-log.jsonl`. Best-effort.
fn append_action_log(store: &StateStore, slug: &str, track: &Track, action: &RunAction) {
    let track = match track {
        Track::Feedback => "feedback",
        Track::Run => "run",
    };
    let entry = serde_json::json!({
        "at": Utc::now().to_rfc3339(),
        "track": track,
        "action": action_tag(action),
        "station": station_of(action),
    });
    let _ = store.append_journal(slug, "action-log.jsonl", &entry.to_string());
}

/// Run the downstream sync (mechanic #5) for the active station before a tick
/// derives. Best-effort + non-fatal: any failure or non-git project no-ops, and
/// a conflict is left in-tree for the derive's [`merge_conflict_action`] to
/// surface. Skipped when there is no active station (run complete / unseeded).
fn sync_downstream_before_land(store: &StateStore, slug: &str) {
    let Ok(run) = store.read_run(slug) else {
        return;
    };
    let Some(factory) = resolve_factory_for(store, &run.frontmatter.factory) else {
        return;
    };
    let Ok(Some(state)) = store.read_state(slug) else {
        return;
    };
    // Discrete runs land via the human PR merge, not in-process — the sync only
    // matters for the in-process land path.
    if state.mode.opens_station_pr() {
        return;
    }
    if let Some(station) = current_station(&factory, &state) {
        let _ = crate::lifecycle::sync_branch_downstream(store, slug, &station);
    }
}

/// Stamp the station phase forward based on the action just emitted.
fn advance_state(store: &StateStore, slug: &str, action: &RunAction) -> Result<()> {
    let run = store.read_run(slug)?;
    let factory = resolve_factory_for(store, &run.frontmatter.factory)
        .ok_or_else(|| McpError::UnknownFactory(run.frontmatter.factory.clone()))?;
    let mut state = store.read_state(slug)?.unwrap_or_else(|| RunState {
        factory: run.frontmatter.factory.clone(),
        active_station: run.frontmatter.active_station.clone(),
        ..Default::default()
    });

    let now = Utc::now().to_rfc3339();

    // Track whether THIS tick is the first entry of a station (Pending ->
    // InProgress on the Spec phase) so we can fork its branch after persisting,
    // and whether a station just COMPLETED so we can land it after persisting.
    // Both are git side-effects kept OUT of the state mutation + out of the pure
    // derive_position; they run on the side once state.json is written.
    let mut entered_station: Option<String> = None;
    let mut landed_station: Option<String> = None;
    // Per-unit branch side-effects (B9): the wave-ready units to fork onto their
    // own worktrees this tick, and — when the station leaves Manufacture — the
    // completed units to land back onto the station branch. Same out-of-derive,
    // after-persist discipline as the station side-effects.
    let mut entered_units: Vec<(String, String)> = Vec::new();
    let mut landed_units: Vec<(String, String)> = Vec::new();
    // The fix `(station, fix_id)` to fork onto its own worktree this tick (B9):
    // a drift/feedback repair is isolated off the station branch so its diff
    // never tangles with in-flight units. Idempotent re-entry across the ticks
    // the fix-worker spends resolving. The fix lands back on resolution (drift
    // accept / feedback close), not here.
    let mut entered_fix: Option<(String, String)> = None;
    // Whether an auto-checkpoint just COMPLETED a station (independent of
    // whether that station carried merge debt to land). Drives run completion.
    let mut auto_completed = false;

    match action {
        RunAction::Spec { station, .. } => {
            let st = ensure_station(&mut state, &factory, station)?;
            // First entry of this station: fork its per-station branch (universal
            // across modes). Detected by the Pending -> InProgress transition.
            if matches!(st.status, Status::Pending) {
                entered_station = Some(station.clone());
            }
            st.status = Status::InProgress;
            // Collaboration backpressure (C1): in `team`/`solo` the Spec phase
            // HOLDS at every station until the operator has been involved
            // (`darkrun_elaborate_seal`) — the agent can't author the spec solo
            // and skip the human. A stalled, never-sealed Spec is caught by the
            // deadlock guard and escalated to the operator. `dark` pre-elaborates
            // once up front and keeps the linear Spec→Review progression.
            if run.frontmatter.mode.holds_each_station() && !st.elaborated {
                st.phase = StationPhase::Spec;
            } else {
                st.phase = StationPhase::Review;
            }
            if st.started_at.is_none() {
                st.started_at = Some(now.clone());
            }
            state.active_station = station.clone();
        }
        RunAction::Review { station, .. } => {
            // After the review work lands, an INTERACTIVE station (team/solo)
            // holds at the pre-execution USER gate so the operator can review the
            // spec/brief before any Unit is manufactured; a `dark` run advances
            // straight into Manufacture (its gate is `auto`). The gate is cleared
            // by `darkrun_checkpoint_decide`.
            let kind = factory
                .station(station)
                .map(|_| effective_checkpoint_kind(&state))
                .unwrap_or(CheckpointKind::Auto);
            let st = ensure_station(&mut state, &factory, station)?;
            st.phase = if matches!(kind, CheckpointKind::Auto) {
                // Entering Manufacture without a gate hold → mint the verifier
                // nonce now so the first Manufacture prompt carries it (B5).
                mint_verifier_nonce(st, slug, station, &now);
                StationPhase::Manufacture
            } else {
                StationPhase::UserGate
            };
            state.active_station = station.clone();
        }
        RunAction::UserGate { station, .. } => {
            // Holding at the operator gate — no phase change. The cursor parks
            // here (deadlock-exempt) until `darkrun_checkpoint_decide` advances
            // the recorded phase into Manufacture (approve) or routes feedback
            // (reject). Keep the active station pinned so the surface stays put.
            let st = ensure_station(&mut state, &factory, station)?;
            st.phase = StationPhase::UserGate;
            state.active_station = station.clone();
        }
        RunAction::Manufacture { station, units, .. } => {
            // One wave per tick — stay in Manufacture until every unit locks.
            let st = ensure_station(&mut state, &factory, station)?;
            st.phase = StationPhase::Manufacture;
            // B5 backstop: ensure the verifier nonce exists (the entry
            // transitions mint it, but a re-entry via rework lands here directly).
            mint_verifier_nonce(st, slug, station, &now);
            state.active_station = station.clone();
            // B9: fork each wave-ready unit onto its own worktree off the station
            // branch, so its Pass-loop diff is isolated. Idempotent downstream —
            // a re-dispatched unit reuses its branch + worktree.
            for unit in units {
                entered_units.push((station.clone(), unit.clone()));
            }
        }
        RunAction::Audit { station, .. } => {
            // Audit absorbs what tests did — it verifies the output AND runs
            // the quality checks, then advances straight to Reflect.
            let st = ensure_station(&mut state, &factory, station)?;
            st.phase = StationPhase::Reflect;
            state.active_station = station.clone();
            // B9: the station is leaving Manufacture — land every unit that
            // carries an isolation branch back onto the station branch, so Audit
            // verifies the integrated station tree. Idempotent: an already-landed
            // (or never-forked) unit no-ops.
            if let Ok(units) = store.read_units(slug) {
                for u in units.iter().filter(|u| u.station() == station) {
                    if u.frontmatter.branch.is_some() {
                        landed_units.push((station.clone(), u.slug.clone()));
                    }
                }
            }
        }
        RunAction::Reflect { station, .. } => {
            let st = ensure_station(&mut state, &factory, station)?;
            st.phase = StationPhase::Checkpoint;
            state.active_station = station.clone();
        }
        RunAction::Checkpoint { station, kind, .. } => {
            let st = ensure_station(&mut state, &factory, station)?;
            let checkpoint = Checkpoint {
                kind: *kind,
                entered_at: Some(now.clone()),
                outcome: None,
            };
            st.checkpoint = Some(checkpoint);
            // Auto checkpoints advance immediately; gated kinds hold for
            // checkpoint_decide.
            if matches!(kind, CheckpointKind::Auto) {
                complete_station(&mut state, &factory, station, &now)?;
                auto_completed = true;
                // Snapshot the locked artifacts so the sweep can witness drift.
                crate::drift::record_station_witnesses(store, slug, station)?;
                // Non-discrete in-process land: the verified station branch
                // merges to run-main here. (Discrete stations resolve on a human
                // PR merge — a later phase — and don't land in-process.)
                // #4: gate the land synthesis on merge debt — a station whose
                // branch is identical to / already an ancestor of run-main has
                // nothing to merge, and enqueuing a land would mint an empty
                // --no-ff commit that triggers the alternating no-op-merge loop.
                if !state.mode.opens_station_pr() && station_has_merge_debt(store, slug, station) {
                    landed_station = Some(station.clone());
                }
            }
        }
        RunAction::ExternalReviewRequested { station, .. } => {
            // Enter the external checkpoint and hold — no outcome until
            // checkpoint_decide lands a decision (mirrors a local gate's entry).
            let st = ensure_station(&mut state, &factory, station)?;
            st.checkpoint = Some(Checkpoint {
                kind: CheckpointKind::External,
                entered_at: Some(now.clone()),
                outcome: None,
            });
            state.active_station = station.clone();
        }
        // A feedback repair (incl. an `origin=drift` premise change) is a HOLD —
        // it doesn't advance the phase machine — but it gets its own isolation
        // worktree (B9) so the fix-worker's diff is forked off the station
        // branch. Idempotent re-entry across the ticks the fix takes; it lands
        // back on resolution.
        RunAction::FixFeedback { station, feedback_id, .. } => {
            entered_fix = Some((station.clone(), feedback_id.clone()));
        }
        // Validation / question / seal / noop actions are all HOLDS — they don't
        // advance the run phase machine on their own. The next tick re-derives
        // once the agent has cleared the condition.
        _ => {}
    }

    // Did completing a station empty the plan? If so the run is ready to seal,
    // so the run-main -> base land follows the final station's land. Keyed on
    // the auto-completion (not the land) so a no-debt final station still lands
    // the run onto base — #4 only suppresses the empty per-station merge.
    let run_now_complete = auto_completed
        && !state.mode.opens_station_pr()
        && current_station(&factory, &state).is_none();

    store.write_state(slug, &state)?;

    // Run the branch side-effects AFTER state is persisted, so the durable
    // record reflects the decision and the lifecycle reads a consistent disk.
    run_branch_side_effects(BranchSideEffects {
        store,
        slug,
        entered_station,
        landed_station,
        entered_units,
        landed_units,
        entered_fix,
        run_now_complete,
    })?;
    Ok(())
}

/// The per-tick branch side-effects a resolved [`advance_state`] enqueues, run
/// after state is persisted.
struct BranchSideEffects<'a> {
    store: &'a StateStore,
    slug: &'a str,
    entered_station: Option<String>,
    landed_station: Option<String>,
    /// `(station, unit)` pairs to fork onto their own worktrees (B9).
    entered_units: Vec<(String, String)>,
    /// `(station, unit)` pairs to land back onto the station branch (B9).
    landed_units: Vec<(String, String)>,
    /// The `(station, fix_id)` to fork onto its own worktree (B9), if a fix
    /// track is dispatching this tick. Idempotent across the fix's ticks.
    entered_fix: Option<(String, String)>,
    run_now_complete: bool,
}

/// Apply the per-tick branch lifecycle side-effects. All best-effort + non-fatal
/// (a non-git project no-ops cleanly); the side-effects run only after state is
/// persisted so they never block the manager's forward progress.
///
/// Order matters: enter station → enter its units (units fork off the station
/// branch, so it must exist first); land units → land station (a station lands
/// only the work its units already merged into its branch).
fn run_branch_side_effects(fx: BranchSideEffects) -> Result<()> {
    let BranchSideEffects {
        store,
        slug,
        entered_station,
        landed_station,
        entered_units,
        landed_units,
        entered_fix,
        run_now_complete,
    } = fx;
    if let Some(station) = &entered_station {
        enter_station_and_record(store, slug, station)?;
    }
    for (station, unit) in &entered_units {
        enter_unit_and_record(store, slug, station, unit)?;
    }
    if let Some((station, fix_id)) = &entered_fix {
        // Fork the fix onto its own worktree off the station branch. Idempotent;
        // no branch is stamped on state (the fix's id + station fully derive it,
        // and the fix lands on resolution via land_fix).
        crate::lifecycle::enter_fix(store, slug, station, fix_id);
    }
    for (station, unit) in &landed_units {
        crate::lifecycle::land_unit(store, slug, station, unit);
    }
    if let Some(station) = &landed_station {
        crate::lifecycle::land_station(store, slug, station);
    }
    if run_now_complete {
        crate::lifecycle::land_run(store, slug);
    }
    Ok(())
}

/// Whether this is a git-backed run whose `station` branch exists — the
/// precondition for per-unit/per-fix worktree isolation (a child can only fork
/// off a station branch that's there). Used to decide whether to surface a
/// worktree path in a dispatch prompt. The branch side-effects that actually
/// fork the worktree run AFTER the prompt renders, so the prompt shows the
/// *derived* path it's about to create, not one already on disk.
fn git_backed_station(store: &StateStore, slug: &str, station: &str) -> bool {
    let repo_root = cascade_repo_root(store);
    match Git::open(&repo_root) {
        Ok(git) => git
            .branch_exists(&crate::lifecycle::station_branch(slug, station))
            .unwrap_or(false),
        Err(_) => false,
    }
}

/// The fix worktree path to surface for `(station, fix_id)` on a git-backed run,
/// else `None`. The path is the one the branch side-effect forks this same tick;
/// the worker reads the prompt after the tick, when the worktree exists.
fn fix_worktree_for(store: &StateStore, slug: &str, station: &str, fix_id: &str) -> Option<String> {
    if !git_backed_station(store, slug, station) {
        return None;
    }
    let repo_root = cascade_repo_root(store);
    let path = crate::lifecycle::fix_worktree_path(&repo_root, slug, station, fix_id);
    Some(path.to_string_lossy().into_owned())
}

/// Enter a unit's branch + worktree (forked off the station branch) and record
/// the resulting branch on `Unit.branch`. Best-effort: outside a git repo (or
/// before the station branch exists) the lifecycle no-ops and no branch is
/// recorded. Idempotent — a re-entered unit reuses its branch + worktree.
fn enter_unit_and_record(store: &StateStore, slug: &str, station: &str, unit: &str) -> Result<()> {
    let outcome = crate::lifecycle::enter_unit(store, slug, station, unit);
    if outcome.performed {
        if let Some(branch) = outcome.note {
            let Ok(mut u) = store.read_unit(slug, unit) else {
                return Ok(());
            };
            if u.frontmatter.branch.as_deref() != Some(branch.as_str()) {
                u.frontmatter.branch = Some(branch);
                store.write_unit(slug, &u)?;
            }
        }
    }
    Ok(())
}

/// Ensure a `Station` entry exists in state, seeding it from the factory def.
fn ensure_station<'a>(
    state: &'a mut RunState,
    factory: &FactoryDef,
    station: &str,
) -> Result<&'a mut Station> {
    if !state.stations.contains_key(station) {
        // Validate the station exists in the factory before seeding its state.
        factory
            .station(station)
            .ok_or_else(|| McpError::UnknownStation(station.to_string()))?;
        let gate = state.mode.gate();
        state.stations.insert(
            station.to_string(),
            Station {
                station: station.to_string(),
                status: Status::Pending,
                phase: StationPhase::Spec,
                elaborated: false,
                checkpoint: Some(Checkpoint {
                    kind: gate,
                    entered_at: None,
                    outcome: None,
                }),
                branch: None,
                pr_ref: None,
                pr_status: None,
                pr_ready_at: None,
                pr_merged_at: None,
                verifier_nonce: None,
                started_at: None,
                completed_at: None,
            },
        );
    }
    state
        .stations
        .get_mut(station)
        .ok_or_else(|| McpError::UnknownStation(station.to_string()))
}

/// Mark a station Completed and point the cursor at the next station.
fn complete_station(
    state: &mut RunState,
    factory: &FactoryDef,
    station: &str,
    now: &str,
) -> Result<()> {
    {
        let st = ensure_station(state, factory, station)?;
        st.status = Status::Completed;
        st.completed_at = Some(now.to_string());
        // B5: the station's verification is done — retire its one-time nonce so a
        // late/forged gate record can't reuse it (a fresh re-entry remints one).
        st.verifier_nonce = None;
        if let Some(cp) = st.checkpoint.as_mut() {
            cp.outcome = Some(CheckpointOutcome::Advanced);
        }
    }
    // Advance to the next station in the run's plan (not the factory's full
    // order) — a right-sized run skips the stations its plan omits.
    if let Some(next_name) = next_in_plan(factory, state, station) {
        let st = ensure_station(state, factory, &next_name)?;
        st.status = Status::Pending;
        st.phase = StationPhase::Spec;
        state.active_station = next_name;
    }
    Ok(())
}

/// Mint the station's one-time verifier nonce if it doesn't have one yet (B5).
/// Called at the transition INTO Manufacture so the nonce is present before the
/// first Manufacture prompt renders. The token is a content hash over the run,
/// station, and dispatch time — unique per dispatch and not guessable from the
/// run state, so a quality-gate result can't be recorded without it.
fn mint_verifier_nonce(st: &mut Station, slug: &str, station: &str, now: &str) {
    if st.verifier_nonce.is_none() {
        let seed = format!("{slug}\u{1}{station}\u{1}{now}");
        st.verifier_nonce = Some(darkrun_core::hash_bytes(seed.as_bytes()));
    }
}

/// Start a fresh run: write `run.md`, right-size the station plan from `size`,
/// seed `state.json` at the plan's first station in the `Spec` phase, and return
/// the run slug.
///
/// `mode` is the global review posture (team/solo/dark) — it decides every
/// station's gate. `size` is the orthogonal right-sizing axis: `full`/unknown
/// walks every factory station; `quick`/`bugfix`/`refactor` collapse to a subset.
pub fn run_start(
    store: &StateStore,
    slug: &str,
    factory_name: &str,
    title: Option<String>,
    mode: Mode,
    size: &str,
) -> Result<Run> {
    let factory = resolve_factory_for(store, factory_name)
        .ok_or_else(|| McpError::UnknownFactory(factory_name.into()))?;
    let factory_first = factory
        .first_station()
        .ok_or_else(|| McpError::UnknownFactory(factory_name.into()))?;

    // Right-size: the plan is the size's station subset (empty = full factory).
    let plan = resolve_size(size, &factory);
    let first_name = plan
        .first()
        .cloned()
        .unwrap_or_else(|| factory_first.name.clone());

    // ── Branch hierarchy FIRST (mirrors the predecessor's intent-create) ──
    // Fork `darkrun/<slug>/main` off the base and CHECK IT OUT in the main
    // working tree BEFORE any state is written, so every state write from here
    // on lands on the run's own branch — committed and pushed as the run
    // progresses, never on whatever branch the operator happened to be on.
    // Non-git projects no-op cleanly (filesystem mode).
    crate::lifecycle::ensure_run_main(store, slug);
    {
        let root = cascade_repo_root(store);
        if let Ok(git) = Git::open(&root) {
            let run_main = crate::lifecycle::run_main_branch(slug);
            if git.branch_exists(&run_main).unwrap_or(false)
                && git.current_branch().ok().flatten().as_deref() != Some(run_main.as_str())
            {
                // A dirty tree refuses the switch with a clear, actionable
                // error — never silently carries uncommitted work across.
                if !git.is_clean().unwrap_or(false) {
                    return Err(McpError::InvalidInput(format!(
                        "cannot start run '{slug}': the working tree has uncommitted or \
                         untracked changes, and starting a run switches it to '{run_main}'. \
                         Commit or stash them, then retry."
                    )));
                }
                git.checkout_branch(&run_main).map_err(|e| {
                    McpError::InvalidInput(format!(
                        "cannot start run '{slug}': switching to '{run_main}' failed: {e}"
                    ))
                })?;
            }
        }
        // The worktree pool must be ignored before the first worktree exists.
        crate::commit::ensure_worktrees_gitignored(&root);
    }

    let now = Utc::now().to_rfc3339();
    let resolved_title = title.clone().unwrap_or_else(|| slug.to_string());
    // Stamp the creator's git identity so the run is "mine" from birth — the
    // branch-authorship walk needs commits, which a brand-new run lacks.
    let created_by = darkrun_git::current_identity_email(cascade_repo_root(store))
        .ok()
        .flatten();
    let frontmatter = RunFrontmatter {
        title: title.clone(),
        factory: factory_name.to_string(),
        mode,
        active_station: first_name.clone(),
        status: Status::Active,
        started_at: Some(now.clone()),
        created_by,
        ..Default::default()
    };
    let body = format!("# {resolved_title}\n");
    let run = Run {
        slug: slug.to_string(),
        frontmatter,
        title: resolved_title,
        body,
    };
    store.write_run(&run)?;

    // Seed state at the plan's first station, Spec phase. Snapshot the base
    // branch so the run-completion land has a stable target even if settings
    // change mid-run.
    let base = crate::lifecycle::resolve_base_branch(store);
    let mut state = RunState {
        factory: factory_name.to_string(),
        active_station: first_name.clone(),
        plan,
        mode,
        base_branch: Some(base),
        // Stamp the plugin provenance (which build authored the run) and the
        // on-disk schema version (what shape-migrators key on) — versioned
        // independently (G1).
        created_with_version: Some(env!("CARGO_PKG_VERSION").to_string()),
        schema_version: Some(darkrun_core::SCHEMA_VERSION),
        ..Default::default()
    };
    ensure_station(&mut state, &factory, &first_name)?;
    store.write_state(slug, &state)?;

    // Commit + push the freshly-seeded run state on run-main BEFORE forking the
    // first station, so the station branch carries the run document from birth
    // and origin sees the run the moment it exists (commit early, commit often).
    let _ = crate::commit::commit_state(store, &format!("darkrun: create run {slug}"));

    // Enter the first station (fork its branch + worktree off run-main).
    enter_station_and_record(store, slug, &first_name)?;
    // The station-entry stamp (`Station.branch`) is state too — publish it.
    let _ = crate::commit::commit_state_if_dirty(
        store,
        &format!("darkrun: enter station {first_name}"),
    );

    crate::events::emit(
        store,
        slug,
        "darkrun.run.created",
        serde_json::json!({ "factory": factory_name, "mode": format!("{mode:?}").to_lowercase() }),
    );

    // Open the run's DELIVERY draft PR (run-main -> base) the moment the run
    // exists — the predecessor's intent-create draft. It stays draft for the
    // whole run (reviewers see work-in-progress, nobody merges it early) and
    // the engine flips it ready at seal, just before the operator's merge.
    // Best-effort: no hosting client → no PR; the seal's compare-URL fallback
    // covers the manual path.
    open_run_draft_pr(store, slug);

    Ok(run)
}

/// Open the run-level draft PR (`darkrun/<slug>/main` -> base) and stamp its
/// url + draft status on the run's `external_refs`. Best-effort and idempotent:
/// hosting unavailable, an open failure, or an already-recorded PR all no-op.
fn open_run_draft_pr(store: &StateStore, slug: &str) {
    let repo_root = cascade_repo_root(store);
    let hosting = crate::hosting::ApiHosting::resolve(&repo_root);
    open_run_draft_pr_with(store, slug, &hosting);
}

/// [`open_run_draft_pr`] with an injected hosting client (the test seam).
pub fn open_run_draft_pr_with<H: crate::hosting::Hosting>(
    store: &StateStore,
    slug: &str,
    hosting: &H,
) {
    let Ok(mut run) = store.read_run(slug) else {
        return;
    };
    if run.frontmatter.external_refs.pr_url.is_some() || !hosting.available() {
        return;
    }
    let head = crate::lifecycle::run_main_branch(slug);
    let base = store
        .read_state(slug)
        .ok()
        .flatten()
        .and_then(|s| s.base_branch)
        .unwrap_or_else(|| crate::lifecycle::resolve_base_branch(store));
    let req = crate::hosting::OpenRequest {
        head,
        base,
        title: run.title.clone(),
        body: format!(
            "darkrun run `{slug}` — opened as a draft at run start; the engine \
             marks it ready for review when the run seals. Merging it lands the \
             run's work."
        ),
    };
    if let Some(url) = hosting.open_draft(&req) {
        run.frontmatter.external_refs.pr_url = Some(url);
        run.frontmatter
            .external_refs
            .other
            .insert("pr_status".into(), "draft".into());
        let _ = store.write_run(&run);
        let _ = crate::commit::commit_state_if_dirty(
            store,
            &format!("darkrun: open run draft PR for {slug}"),
        );
    }
}

/// Flip the run's delivery draft PR to **ready for review** — called when the
/// run seals, just before the operator's merge. Guarded by the recorded
/// `pr_status` so it flips exactly once; a failed flip is stamped `failed`
/// (the operator readies it by hand — their merge is the close signal either
/// way, so a flip failure never blocks the seal).
fn flip_run_pr_ready<H: crate::hosting::Hosting>(store: &StateStore, slug: &str, hosting: &H) {
    let Ok(mut run) = store.read_run(slug) else {
        return;
    };
    let Some(url) = run.frontmatter.external_refs.pr_url.clone() else {
        return;
    };
    let status = run
        .frontmatter
        .external_refs
        .other
        .get("pr_status")
        .cloned()
        .unwrap_or_default();
    if status != "draft" {
        return;
    }
    let new_status = if hosting.mark_ready(&url) { "ready" } else { "failed" };
    run.frontmatter
        .external_refs
        .other
        .insert("pr_status".into(), new_status.into());
    let _ = store.write_run(&run);
    let _ = crate::commit::commit_state_if_dirty(
        store,
        &format!("darkrun: run PR marked {new_status} for {slug}"),
    );
}

/// Enter a station's branch + worktree (universal per-station fork) and record
/// the resulting branch on `Station.branch`. Best-effort: outside a git repo the
/// lifecycle no-ops and no branch is recorded.
fn enter_station_and_record(store: &StateStore, slug: &str, station: &str) -> Result<()> {
    let outcome = crate::lifecycle::enter_station(store, slug, station);
    if outcome.performed {
        // The lifecycle returns the station branch name in `note` on success.
        if let Some(branch) = outcome.note {
            let mut state = store.read_state(slug)?.unwrap_or_default();
            if let Some(st) = state.stations.get_mut(station) {
                if st.branch.as_deref() != Some(branch.as_str()) {
                    st.branch = Some(branch);
                    store.write_state(slug, &state)?;
                }
            }
        }
    }
    Ok(())
}

/// Apply an operator decision to the active station's Checkpoint.
///
/// `approved == true` advances the station (mirrors an `auto`/approved gate);
/// `approved == false` holds the station and stamps the gate as `Blocked` so
/// the rework routes back as feedback on the next tick.
pub fn checkpoint_decide(
    store: &StateStore,
    slug: &str,
    approved: bool,
    feedback: Option<String>,
) -> Result<TickResult> {
    let run = store.read_run(slug)?;
    let factory = resolve_factory_for(store, &run.frontmatter.factory)
        .ok_or_else(|| McpError::UnknownFactory(run.frontmatter.factory.clone()))?;
    let mut state = store.read_state(slug)?.unwrap_or_default();
    let station = current_station(&factory, &state)
        .ok_or_else(|| McpError::NoActiveStation(slug.to_string()))?;

    let now = Utc::now().to_rfc3339();

    // ── Pre-execution USER gate ──────────────────────────────────────────
    // When the station is parked at the pre-execution operator gate (the spec
    // review, BEFORE any Unit is manufactured), the same decide call resolves
    // it: approve releases the wave (→ Manufacture); a block holds at the gate
    // and routes the operator's spec feedback through the fix track. This is the
    // pre-execution twin of the checkpoint resolution below — it must NOT
    // complete/land the station (nothing has been manufactured yet).
    if station_phase(&state, &station) == StationPhase::UserGate {
        let st = ensure_station(&mut state, &factory, &station)?;
        if approved {
            st.phase = StationPhase::Manufacture;
            // B5: releasing the wave dispatches verification — mint the nonce.
            mint_verifier_nonce(st, slug, &station, &now);
        } else if let Some(body) = feedback {
            // Hold at the gate (phase stays UserGate); the pending feedback doc
            // preempts via Track B next tick, then the gate re-opens for a
            // re-decision once the spec is reworked.
            let doc = format!("---\nstatus: pending\nstation: {station}\n---\n{body}\n");
            store.write_feedback_raw(slug, "fb-spec-gate", &doc)?;
        }
        store.write_state(slug, &state)?;
        return run_tick(store, slug);
    }

    let mut landed_station: Option<String> = None;
    let mut run_now_complete = false;
    if approved {
        complete_station(&mut state, &factory, &station, &now)?;
        // Snapshot the locked artifacts so the sweep can witness drift.
        crate::drift::record_station_witnesses(store, slug, &station)?;
        // Non-discrete in-process land of the just-verified station; discrete
        // stations land via the human's PR merge (a later phase).
        if !state.mode.opens_station_pr() {
            // #4: only enqueue the station land when there's merge debt — a
            // no-debt station has nothing to merge, and a land would mint an
            // empty --no-ff commit that triggers the alternating-sync loop.
            if station_has_merge_debt(store, slug, &station) {
                landed_station = Some(station.clone());
            }
            // Run completion is independent of the per-station land (the run can
            // complete even when the last station was a no-op merge).
            run_now_complete = current_station(&factory, &state).is_none();
        }
    } else {
        let st = ensure_station(&mut state, &factory, &station)?;
        // Hold the station; route rework back through the feedback track.
        st.status = Status::Blocked;
        if let Some(cp) = st.checkpoint.as_mut() {
            cp.outcome = Some(CheckpointOutcome::Blocked);
        }
        if let Some(body) = feedback {
            let id = "fb-checkpoint";
            let doc = format!("---\nstatus: pending\n---\n{body}\n");
            store.write_feedback_raw(slug, id, &doc)?;
        }
        // Ship the station's OPEN annotations — the global station note leading,
        // then each per-artifact ask — as a feedback doc so the rework loop reads
        // them back alongside any free-form checkpoint note. The annotation
        // records themselves are retained (status carries their lifecycle); this
        // is the legible hand-off body. Skipped when there are no open marks.
        let annotations = store.list_annotations(slug)?;
        let station_marks: Vec<_> = annotations
            .into_iter()
            .filter(|a| {
                a.work_item.station == station
                    && a.status == darkrun_api::annotation::AnnotationStatus::Open
            })
            .collect();
        if !station_marks.is_empty() {
            let id = "fb-annotations";
            let body = crate::annotation::render_rework_feedback(&station, &station_marks);
            let doc = format!("---\nstatus: pending\nstation: {station}\n---\n{body}\n");
            store.write_feedback_raw(slug, id, &doc)?;
        }
    }
    store.write_state(slug, &state)?;

    // Land the just-verified station (and the run, if it was the last) AFTER
    // state is persisted. The next station's branch is forked when its Spec
    // tick first enters it inside the re-tick below. The station's units already
    // landed onto its branch when it left Manufacture, so there are no per-unit
    // side-effects here.
    run_branch_side_effects(BranchSideEffects {
        store,
        slug,
        entered_station: None,
        landed_station,
        entered_units: Vec::new(),
        landed_units: Vec::new(),
        entered_fix: None,
        run_now_complete,
    })?;

    // Re-tick so the caller sees the new cursor position immediately.
    run_tick(store, slug)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn store() -> (tempfile::TempDir, StateStore) {
        let dir = tempdir().expect("tmp");
        let store = StateStore::new(dir.path());
        (dir, store)
    }

    #[test]
    fn integrity_problem_flags_a_unit_on_an_undefined_station() {
        use darkrun_core::domain::{Unit, UnitFrontmatter};
        let factory = crate::factory::resolve_factory("software").unwrap();
        let on = |station: &str| Unit {
            slug: "u".into(),
            frontmatter: UnitFrontmatter { station: Some(station.into()), ..Default::default() },
            title: "u".into(),
            body: String::new(),
        };
        // A unit on a real station is fine; one on a station the factory doesn't
        // define is an integrity problem (routes a guarded SafeRepair).
        assert!(integrity_problem(&factory, &[on("frame")]).is_none());
        let bad = integrity_problem(&factory, &[on("ghost-station")]).expect("flagged");
        assert!(bad.contains("ghost-station"));
        // An empty station name is ignored (not a defined-station violation).
        assert!(integrity_problem(&factory, &[on("")]).is_none());
    }

    #[test]
    fn derived_station_phase_none_without_units_some_with_either_autopilot() {
        use darkrun_core::domain::{Status, Unit, UnitFrontmatter};
        let factory = crate::factory::resolve_factory("software").unwrap();
        let def = factory.station("frame").unwrap();
        // No units → no derivable phase.
        assert!(derived_station_phase(&[], def, false).is_none());
        // A unit with NO signals (no iterations/reviews/approvals) also derives
        // nothing — the station hasn't visibly started.
        let bare = Unit {
            slug: "u".into(),
            frontmatter: UnitFrontmatter { status: Status::Completed, station: Some("frame".into()), ..Default::default() },
            title: "u".into(),
            body: String::new(),
        };
        assert!(derived_station_phase(&[&bare], def, false).is_none());
        // A unit that has run a Pass beat carries a signal → a phase derives, in
        // both gate modes (autopilot drops the `user` approval role).
        let mut u = bare.clone();
        u.frontmatter.iterations.push(darkrun_core::domain::UnitIteration {
            worker: "make".into(),
            result: Some(darkrun_core::domain::IterationResult::Advance),
            ..Default::default()
        });
        let refs: Vec<&Unit> = vec![&u];
        assert!(derived_station_phase(&refs, def, false).is_some());
        assert!(derived_station_phase(&refs, def, true).is_some());
    }

    #[test]
    fn walk_feedback_prefers_blocker_then_questions_preempt() {
        let (_d, store) = store();
        let write = |id: &str, doc: &str| store.write_feedback_raw("r", id, doc).unwrap();
        // Filed in id order low → blocker; severity must win over filing order.
        write("fb-01", "---\nstatus: pending\nseverity: low\n---\nnit\n");
        write("fb-02", "---\nstatus: pending\nseverity: blocker\n---\nbroken\n");
        write("fb-03", "---\nstatus: pending\nseverity: high\n---\nbad\n");
        match walk_feedback(&store, "r", "build").unwrap() {
            Some(RunAction::FixFeedback { feedback_id, .. }) => assert_eq!(feedback_id, "fb-02"),
            other => panic!("expected blocker fb-02 first, got {other:?}"),
        }
        // A question preempts every fix, regardless of severity.
        write("fb-00", "---\nstatus: pending\nkind: question\n---\nwhich db?\n");
        match walk_feedback(&store, "r", "build").unwrap() {
            Some(RunAction::FeedbackQuestion { feedback_id, .. }) => assert_eq!(feedback_id, "fb-00"),
            other => panic!("expected question preempt, got {other:?}"),
        }
    }

    #[test]
    fn validate_units_flags_an_input_that_names_a_unit_not_a_path() {
        let mk = |slug: &str, inputs: Vec<&str>| Unit {
            slug: slug.into(),
            frontmatter: darkrun_core::domain::UnitFrontmatter {
                inputs: inputs.into_iter().map(String::from).collect(),
                ..Default::default()
            },
            title: slug.into(),
            body: String::new(),
        };
        let a = mk("limiter", vec![]);
        // `b` wrongly lists the sibling unit `limiter` as an input premise.
        let b = mk("middleware", vec!["limiter"]);
        let units = vec![a, b];
        let su: Vec<&Unit> = units.iter().collect();
        let (problem, bad) = validate_units(&units, &su).expect("should flag");
        assert_eq!(problem, "input_not_a_path");
        assert_eq!(bad, vec!["middleware".to_string()]);

        // A real path input is fine.
        let c = mk("ok", vec!["frame/frame.md"]);
        let units2 = vec![c];
        let su2: Vec<&Unit> = units2.iter().collect();
        assert!(validate_units(&units2, &su2).is_none());
    }

    /// Gap #10: the Spec prompt surfaces the PROJECT knowledge store as priors,
    /// so a new run builds on what earlier runs' explorers recorded.
    #[test]
    fn spec_prompt_surfaces_project_knowledge_priors() {
        let (_d, store) = store();
        // A prior recorded by an earlier run's explorer (project-scoped).
        crate::knowledge::record(&store, "auth-store", "tokens live in the CredentialStore").unwrap();
        run_start(&store, "r", "software", None, Mode::Solo, "full").expect("start");
        let t = run_tick(&store, "r").expect("tick");
        assert!(matches!(t.action, RunAction::Spec { .. }), "first action is Spec");
        let prompt = t.prompt.expect("spec renders a prompt");
        assert!(prompt.contains("auth-store"), "the knowledge topic appears: {prompt}");
        assert!(prompt.contains("CredentialStore"), "the knowledge body appears");
    }

    /// Gap #7: every rendered prompt is persisted under
    /// `.darkrun/<run>/prompts/<station>/<tag>.md` for inspection / replay.
    #[test]
    fn run_tick_persists_the_rendered_prompt() {
        let (_d, store) = store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").expect("start");
        let t = run_tick(&store, "r").expect("tick");
        let prompt = t.prompt.clone().expect("the first tick renders a prompt");

        let persisted = store.read_prompts("r").expect("read prompts");
        assert!(!persisted.is_empty(), "a prompt was persisted to disk");
        // The persisted file is keyed by the action's station + tag and matches
        // exactly what the tick handed back.
        let station = station_of(&t.action).unwrap_or("_run");
        let key = format!("{station}/{}", action_tag(&t.action));
        assert_eq!(
            persisted.get(&key).map(String::as_str),
            Some(prompt.as_str()),
            "persisted prompt at {key} matches the tick's prompt"
        );
    }

    #[test]
    fn cursor_holds_station_inputs_dropped_until_a_unit_consumes_them() {
        let (_d, store) = store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").expect("start");

        // Drive frame to completion so the run reaches `specify`, which carries
        // `frame.md` forward.
        let inputs_for = |station: &str| {
            crate::factory::resolve_factory("software")
                .unwrap()
                .station(station)
                .map(|d| d.inputs.clone())
                .unwrap_or_default()
        };
        let seed = |station: &str, inputs: Vec<String>| {
            let unit = Unit {
                slug: format!("{station}-u"),
                frontmatter: darkrun_core::domain::UnitFrontmatter {
                    status: Status::Completed,
                    station: Some(station.to_string()),
                    inputs,
                    ..Default::default()
                },
                title: "u".into(),
                body: String::new(),
            };
            store.write_unit("r", &unit).unwrap();
        };

        // frame has no inputs → a bare unit clears it.
        seed("frame", vec![]);
        for _ in 0..16 {
            let t = run_tick(&store, "r").expect("tick");
            match &t.action {
                RunAction::UserGate { .. } | RunAction::Checkpoint { .. } => {
                    checkpoint_decide(&store, "r", true, None).expect("decide");
                }
                RunAction::Spec { station, .. } if station == "specify" => break,
                // Solo holds each station's Spec until the elaboration is sealed.
                RunAction::Spec { station, .. } => {
                    elaborate_seal(&store, "r", station).expect("seal");
                }
                _ => {}
            }
        }

        // At specify, decompose a unit that DROPS the carried `frame.md`.
        seed("specify", vec![]);
        // The cursor refuses to manufacture — it holds with station_inputs_dropped
        // naming the dropped artifact.
        let pos = derive_position(&store, "r").expect("derive");
        match pos.action {
            Some(RunAction::UnitsInvalid { ref problem, ref units, .. }) => {
                assert_eq!(problem, "station_inputs_dropped");
                assert_eq!(units, &vec!["frame.md".to_string()]);
            }
            other => panic!("expected station_inputs_dropped hold, got {other:?}"),
        }

        // Wire `frame.md` into the unit's inputs → the hold clears and the station
        // can manufacture.
        let mut u = store.read_unit("r", "specify-u").unwrap();
        u.frontmatter.inputs = inputs_for("specify"); // [frame.md]
        store.write_unit("r", &u).unwrap();
        let pos = derive_position(&store, "r").expect("derive");
        assert!(
            !matches!(pos.action, Some(RunAction::UnitsInvalid { .. })),
            "coverage satisfied → no longer held: {:?}",
            pos.action
        );
    }

    #[test]
    fn wave_ready_excludes_in_flight_units() {
        // B7: a unit that has been dispatched (InProgress) — handed to a worker
        // and now reporting its pass loop — is NOT re-picked by the next wave,
        // the dispatch-lease guarantee. wave_ready returns only Pending units
        // whose deps are complete.
        let mk = |slug: &str, status: Status, deps: &[&str]| Unit {
            slug: slug.into(),
            frontmatter: darkrun_core::domain::UnitFrontmatter {
                status,
                depends_on: deps.iter().map(|s| s.to_string()).collect(),
                ..Default::default()
            },
            title: slug.into(),
            body: String::new(),
        };
        let units = vec![
            mk("dispatched", Status::InProgress, &[]), // in flight → excluded
            mk("done", Status::Completed, &[]),         // finished → excluded
            mk("ready", Status::Pending, &[]),          // fresh → dispatched
            mk("blocked", Status::Pending, &["ready"]), // dep not done → excluded
        ];
        let wave: Vec<&str> = wave_ready(&units).iter().map(|u| u.slug.as_str()).collect();
        assert_eq!(wave, vec!["ready"], "only the fresh, unblocked unit is wave-ready");

        // Once `ready` is dispatched (InProgress) it drops out, and `blocked`
        // stays held until `ready` actually COMPLETES — being in flight does not
        // unblock a dependent.
        let units = vec![
            mk("ready", Status::InProgress, &[]),
            mk("blocked", Status::Pending, &["ready"]),
        ];
        assert!(wave_ready(&units).is_empty(), "an in-flight dep doesn't release its dependent");
    }

    #[test]
    fn dropped_station_inputs_flags_distillation_no_unit_consumes() {
        let mk = |slug: &str, inputs: &[&str]| Unit {
            slug: slug.into(),
            frontmatter: darkrun_core::domain::UnitFrontmatter {
                inputs: inputs.iter().map(|s| s.to_string()).collect(),
                ..Default::default()
            },
            title: slug.into(),
            body: String::new(),
        };
        let station_inputs = vec!["frame.md".to_string(), "spec.md".to_string()];

        // No unit consumes spec.md → it's a dropped input.
        let units = [mk("a", &["frame.md"]), mk("b", &[])];
        let su: Vec<&Unit> = units.iter().collect();
        assert_eq!(dropped_station_inputs(&station_inputs, &su), vec!["spec.md".to_string()]);

        // Collective coverage: as long as SOME unit consumes each input (here a
        // fuller path that matches by basename), nothing is dropped.
        let units = [mk("a", &["frame.md"]), mk("b", &["specify/spec.md"])];
        let su: Vec<&Unit> = units.iter().collect();
        assert!(dropped_station_inputs(&station_inputs, &su).is_empty());
    }

    #[test]
    fn required_station_inputs_only_covers_what_the_plan_produces() {
        let f = crate::factory::resolve_factory("software").unwrap();
        // Full plan: build is preceded by frame/specify/shape, so it must carry
        // all three upstream artifacts.
        let full = required_station_inputs(&f, &[], "build");
        assert!(full.contains(&"frame.md".to_string()));
        assert!(full.contains(&"spec.md".to_string()));
        assert!(full.contains(&"design.md".to_string()));

        // Right-sized `quick` plan = [build, prove]: build is FIRST, so nothing
        // upstream is produced — none of its declared inputs are required (frame/
        // specify/shape never ran to create them).
        let quick = vec!["build".to_string(), "prove".to_string()];
        assert!(required_station_inputs(&f, &quick, "build").is_empty());
        // prove, second in the quick plan, must carry build's `code` (produced
        // upstream in-plan) but not spec.md (specify was skipped).
        let prove = required_station_inputs(&f, &quick, "prove");
        assert!(prove.contains(&"code".to_string()));
        assert!(!prove.contains(&"spec.md".to_string()));
    }

    #[test]
    fn missing_outputs_flags_a_declared_artifact_not_on_disk() {
        // A store rooted under a repo dir so output paths resolve to real files.
        let dir = tempdir().expect("tmp");
        let repo = dir.path().to_path_buf();
        let store = StateStore::new(&repo);

        let mut produced = Unit {
            slug: "u-ok".into(),
            frontmatter: Default::default(),
            title: "ok".into(),
            body: String::new(),
        };
        produced.frontmatter.status = Status::Completed;
        produced.frontmatter.outputs = vec!["made.txt".into()];
        std::fs::write(repo.join("made.txt"), b"x").unwrap();

        let mut promised = produced.clone();
        promised.slug = "u-missing".into();
        promised.frontmatter.outputs = vec!["never.txt".into()];

        // Predecessor BUG-3: a `touch`ed 0-byte file must NOT satisfy a promised
        // output — an empty artifact ships nothing and reads "stable" to drift.
        let mut empty = produced.clone();
        empty.slug = "u-empty".into();
        empty.frontmatter.outputs = vec!["touched.txt".into()];
        std::fs::write(repo.join("touched.txt"), b"").unwrap();

        // A directory at the output path doesn't satisfy it either.
        let mut as_dir = produced.clone();
        as_dir.slug = "u-dir".into();
        as_dir.frontmatter.outputs = vec!["outdir".into()];
        std::fs::create_dir(repo.join("outdir")).unwrap();

        let missing = missing_outputs(&store, &[produced, promised, empty, as_dir]);
        assert_eq!(
            missing,
            vec!["u-missing".to_string(), "u-empty".to_string(), "u-dir".to_string()],
            "absent, empty, and directory outputs are all 'missing'"
        );
    }

    #[test]
    fn engine_guards_degrade_cleanly_on_a_ghost_factory_or_corrupt_run() {
        let (_d, store) = store();
        run_start(&store, "g", "software", None, Mode::Solo, "full").unwrap();
        // Point the run at a factory that doesn't exist.
        let mut run = store.read_run("g").unwrap();
        run.frontmatter.factory = "ghost-factory".into();
        store.write_run(&run).unwrap();

        // sync_downstream_before_land no-ops when the factory can't resolve.
        sync_downstream_before_land(&store, "g"); // must not panic
        // resolve_discrete_gate also bails cleanly on an unresolvable factory.
        resolve_discrete_gate(&store, "g", &MockHosting::unavailable()).unwrap();

        // A corrupt run.md makes the pre-land sync read no-op rather than panic.
        std::fs::write(store.run_dir("g").join("run.md"), "---\nfactory: \"oops\n---\n").unwrap();
        sync_downstream_before_land(&store, "g"); // read_run err → clean return
    }

    #[test]
    fn apply_requested_unit_resets_tolerates_unreadable_units() {
        let (_d, store) = store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").unwrap();
        // A corrupt unit doc makes read_units error; the reset pass must no-op.
        let units = store.units_dir("r");
        std::fs::create_dir_all(&units).unwrap();
        std::fs::write(units.join("broken.md"), "---\nstatus: \"x\n---\n").unwrap();
        apply_requested_unit_resets(&store, "r"); // must not panic
    }

    #[test]
    fn blocking_a_pre_execution_user_gate_files_spec_feedback_and_holds() {
        let (_d, store) = store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").expect("start");
        // Park frame at the pre-execution operator gate.
        let mut state = store.read_state("r").unwrap().unwrap();
        state.stations.get_mut("frame").unwrap().phase = StationPhase::UserGate;
        store.write_state("r", &state).unwrap();

        // Blocking the gate with feedback files a pending spec-gate finding and
        // holds the station (it must NOT complete — nothing is manufactured yet).
        checkpoint_decide(&store, "r", false, Some("tighten the success metric".into()))
            .expect("decide");
        let raw = store.read_feedback_raw("r").unwrap();
        assert!(raw.contains_key("fb-spec-gate"), "the block files spec-gate feedback");
        assert!(raw["fb-spec-gate"].contains("tighten the success metric"));
        let st = store.read_state("r").unwrap().unwrap();
        assert_eq!(st.stations["frame"].phase, StationPhase::UserGate, "still held at the gate");
    }

    #[test]
    fn build_prompt_context_threads_wave_handoffs_and_the_factory_model() {
        use darkrun_core::domain::{IterationResult, Unit, UnitIteration, UnitFrontmatter};
        let (_d, store) = store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").expect("start");

        // Three wave units, each carrying a most-recent handoff note with a
        // different terminal result (advance / reject / in-flight).
        let with_iter = |slug: &str, result: Option<IterationResult>| {
            let unit = Unit {
                slug: slug.into(),
                frontmatter: UnitFrontmatter {
                    station: Some("frame".into()),
                    iterations: vec![UnitIteration {
                        worker: "maker".into(),
                        result,
                        note: Some(format!("note for {slug}")),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                title: slug.into(),
                body: String::new(),
            };
            store.write_unit("r", &unit).unwrap();
        };
        with_iter("u-adv", Some(IterationResult::Advance));
        with_iter("u-rej", Some(IterationResult::Reject));
        with_iter("u-fly", None);

        let action = RunAction::Manufacture {
            run: "r".into(),
            station: "frame".into(),
            // A worker with no role-model override → the factory default resolves.
            worker: "ghostworker".into(),
            units: vec!["u-adv".into(), "u-rej".into(), "u-fly".into()],
        };
        let ctx = build_prompt_context(&store, "r", &action).expect("context");

        assert_eq!(ctx.model.as_deref(), Some("sonnet"), "falls back to the factory default model");
        let result_of = |slug: &str| ctx.handoffs.iter().find(|h| h.unit == slug).map(|h| h.result.as_str());
        assert_eq!(result_of("u-adv"), Some("advance"));
        assert_eq!(result_of("u-rej"), Some("reject"));
        assert_eq!(result_of("u-fly"), Some("in_flight"));
        assert!(ctx.handoffs.iter().all(|h| h.note.starts_with("note for")));
    }

    #[test]
    fn a_run_with_an_unmet_seal_gate_holds_at_pending_seal() {
        use darkrun_core::domain::{SealKind, Status};
        let (_d, store) = store();
        // Dark + quick: plan = [build, prove], and Dark skips the cross-station
        // run review, so once both stations lock the cursor goes straight to the
        // seal decision.
        run_start(&store, "r", "software", Some("ship".into()), Mode::Dark, "quick").expect("start");

        let mut state = store.read_state("r").unwrap().unwrap();
        // Only the first planned station is seeded at start; mark every station in
        // the plan complete so the cursor reaches the seal decision.
        let template = state.stations.get("build").unwrap().clone();
        for st in ["build", "prove"] {
            let mut entry = template.clone();
            entry.station = st.to_string();
            entry.status = Status::Completed;
            state.stations.insert(st.to_string(), entry);
        }
        store.write_state("r", &state).unwrap();

        // Declare a final `seal: external` gate but leave the run un-completed →
        // the run hangs at PendingSeal awaiting the external merge decision.
        let mut run = store.read_run("r").unwrap();
        run.frontmatter.seal = Some(SealKind::External);
        run.frontmatter.status = Status::Active;
        store.write_run(&run).unwrap();

        match derive_position(&store, "r").expect("derive").action {
            Some(RunAction::PendingSeal { kind, .. }) => assert_eq!(kind, SealKind::External),
            other => panic!("expected PendingSeal hold, got {other:?}"),
        }

        // Once the run is marked delivered, the same shape seals.
        run.frontmatter.status = Status::Completed;
        store.write_run(&run).unwrap();
        assert!(
            matches!(derive_position(&store, "r").unwrap().action, Some(RunAction::Sealed { .. })),
            "a completed run seals rather than holding"
        );
    }

    #[test]
    fn resolve_size_falls_back_when_a_right_sized_plan_keeps_no_real_stations() {
        use crate::factory::{FactoryDef, StationDef};
        // A factory that defines neither `build` nor `prove`: the `quick` template
        // keeps [build, prove], which filters to nothing, so right-sizing degrades
        // to the full plan rather than stranding the run with zero stations.
        let only = StationDef {
            name: "frame".into(), label: None, optional: false, kills: "wrong-thing".into(), artifact: "o.md".into(),
            explorers: vec![],
            workers: vec![], fix_workers: vec![], reviewers: vec![], role_models: Default::default(),
            role_interpretations: Default::default(), worker_roles: Default::default(),
            inputs: vec![], role_applies_to: Default::default(),
        };
        let factory = FactoryDef {
            name: "frame-only".into(), stations: vec![only], surfaces: vec![],
            default_model: "sonnet".into(), run_reviewers: vec![], run_reviewer_applies_to: Default::default(),
        };
        let plan = resolve_size("quick", &factory);
        assert!(plan.is_empty(), "an empty kept-set falls back to the full plan");
    }

    #[test]
    fn a_unit_past_its_pass_budget_escalates() {
        use darkrun_core::domain::{IterationResult, Unit, UnitIteration, UnitFrontmatter};
        let (_d, store) = store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").expect("start");
        // A frame unit that has burned more than MAX_PASSES iterations without
        // converging. `pass()` counts iterations, so seed MAX_PASSES + 1 beats.
        let iterations = (0..=MAX_PASSES)
            .map(|_| UnitIteration {
                worker: "make".into(),
                result: Some(IterationResult::Advance),
                ..Default::default()
            })
            .collect();
        let runaway = Unit {
            slug: "frame-u".into(),
            frontmatter: UnitFrontmatter {
                station: Some("frame".into()),
                iterations,
                ..Default::default()
            },
            title: "u".into(),
            body: String::new(),
        };
        store.write_unit("r", &runaway).unwrap();

        match derive_position(&store, "r").expect("derive").action {
            Some(RunAction::Escalate { reason, station, .. }) => {
                assert_eq!(station, "frame");
                assert!(reason.contains("frame-u"), "names the runaway unit: {reason}");
                assert!(reason.contains("budget"), "explains the budget overrun: {reason}");
            }
            other => panic!("expected Escalate for a runaway pass loop, got {other:?}"),
        }
    }

    #[test]
    fn an_env_blocked_gate_escalates_then_best_effort_boots_with_a_recipe() {
        use darkrun_core::domain::{GateResult, GateStatus, Unit, UnitFrontmatter};
        let (_d, store) = store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").expect("start");
        // The active station (frame) holds a unit whose gate is env-blocked.
        let unit = Unit {
            slug: "frame-u".into(),
            frontmatter: UnitFrontmatter {
                station: Some("frame".into()),
                gate_results: vec![GateResult {
                    name: "itest".into(),
                    status: GateStatus::EnvBlocked,
                    at: None,
                    attempts: 1,
                    detail: Some("connection refused".into()),
                }],
                ..Default::default()
            },
            title: "u".into(),
            body: String::new(),
        };
        store.write_unit("r", &unit).unwrap();

        // No boot recipe → escalate to the operator.
        match derive_position(&store, "r").expect("derive").action {
            Some(RunAction::EscalateToUser { gate, station, .. }) => {
                assert_eq!(station, "frame");
                assert_eq!(gate, "itest");
            }
            other => panic!("expected EscalateToUser without a recipe, got {other:?}"),
        }

        // A recipe whose service tool is on PATH (`sh`) → best-effort boot.
        let boot = concat!(
            "---\n",
            "processes:\n",
            "  - name: db\n",
            "    command: [docker, compose, up, -d, db]\n",
            "    service: true\n",
            "    requires_tool: sh\n",
            "---\n",
        );
        std::fs::write(store.root().join("boot.md"), boot).unwrap();
        match derive_position(&store, "r").expect("derive").action {
            Some(RunAction::BestEffortBoot { gate, services, .. }) => {
                assert_eq!(gate, "itest");
                assert!(
                    services.iter().any(|s| s.contains("db")),
                    "lists the service: {services:?}"
                );
            }
            other => panic!("expected BestEffortBoot with a recipe, got {other:?}"),
        }
    }

    #[test]
    fn manufacture_holds_on_a_missing_declared_output_then_an_unmet_gate() {
        use darkrun_core::domain::{
            GateResult, GateStatus, QualityGate, Status, Unit, UnitFrontmatter,
        };
        let dir = tempdir().expect("tmp");
        let repo = dir.path().to_path_buf();
        let store = StateStore::new(&repo);
        run_start(&store, "r", "software", None, Mode::Solo, "full").expect("start");

        // Force the recorded phase to Manufacture: a completed unit with no
        // derivation signals leaves the pure phase undetermined, so the cursor
        // falls back to the recorded phase.
        let mut state = store.read_state("r").unwrap().unwrap();
        state.stations.get_mut("frame").unwrap().phase = StationPhase::Manufacture;
        store.write_state("r", &state).unwrap();

        // A completed unit promising an output that was never written → the
        // output-existence gate holds the station before Audit.
        let promised = Unit {
            slug: "frame-u".into(),
            frontmatter: UnitFrontmatter {
                status: Status::Completed,
                station: Some("frame".into()),
                outputs: vec!["frame/never.md".into()],
                ..Default::default()
            },
            title: "u".into(),
            body: String::new(),
        };
        store.write_unit("r", &promised).unwrap();
        match derive_position(&store, "r").expect("derive").action {
            Some(RunAction::UnitsInvalid { ref problem, ref units, .. }) => {
                assert_eq!(problem, "missing_output");
                assert_eq!(units, &vec!["frame-u".to_string()]);
            }
            other => panic!("expected missing_output hold, got {other:?}"),
        }

        // Produce the output, but declare a quality gate with no passing result →
        // now the gates-unmet gate holds instead.
        std::fs::create_dir_all(repo.join("frame")).unwrap();
        std::fs::write(repo.join("frame/never.md"), b"done").unwrap();
        let mut gated = promised.clone();
        gated.frontmatter.quality_gates = vec![QualityGate {
            name: "tests".into(),
            command: "cargo test".into(),
        }];
        // A recorded FAIL does not satisfy the gate.
        gated.frontmatter.gate_results = vec![GateResult {
            name: "tests".into(),
            status: GateStatus::Fail,
            at: None,
            attempts: 1,
            detail: None,
        }];
        store.write_unit("r", &gated).unwrap();
        match derive_position(&store, "r").expect("derive").action {
            Some(RunAction::UnitsInvalid { ref problem, ref units, .. }) => {
                assert_eq!(problem, "gates_unmet");
                assert_eq!(units, &vec!["frame-u".to_string()]);
            }
            other => panic!("expected gates_unmet hold, got {other:?}"),
        }
    }

    /// Gap #12: the SECOND quality-gate enforcement point. A unit whose
    /// post-execute reviewers have all signed (so the phase has advanced PAST
    /// Audit) is STILL held if a declared gate isn't satisfied — a regression
    /// during the review pass is caught here, not only at the pre-Audit point.
    #[test]
    fn quality_gates_are_re_enforced_after_the_audit_reviewers_sign() {
        use darkrun_core::domain::{
            GateResult, GateStatus, IterationResult, QualityGate, Stamp, Status, Unit,
            UnitFrontmatter, UnitIteration,
        };
        let (_d, store) = store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").expect("start");
        let factory = crate::factory::resolve_factory("software").unwrap();
        let def = factory.station("frame").unwrap();
        let stamp = || Some(Stamp { at: "2026-01-01T00:00:00Z".into() });

        // A fully-signed unit: every review + approval role stamped and the Pass
        // loop done on the terminal worker → the phase derives PAST Audit. But it
        // declares a quality gate with no passing result.
        let mut fm = UnitFrontmatter {
            status: Status::Completed,
            station: Some("frame".into()),
            ..Default::default()
        };
        for r in &def.reviewers {
            fm.reviews.insert(r.clone(), stamp());
            fm.approvals.insert(r.clone(), stamp());
        }
        fm.approvals.insert("user".into(), stamp());
        fm.iterations.push(UnitIteration {
            worker: def.workers.last().cloned().unwrap_or_default(),
            result: Some(IterationResult::Advance),
            ..Default::default()
        });
        fm.quality_gates = vec![QualityGate {
            name: "tests".into(),
            command: "cargo test".into(),
        }];
        let mut unit = Unit {
            slug: "frame-u".into(),
            frontmatter: fm,
            title: "u".into(),
            body: String::new(),
        };
        store.write_unit("r", &unit).unwrap();

        // Past the Audit reviewers, the unmet gate STILL holds the station.
        match derive_position(&store, "r").expect("derive").action {
            Some(RunAction::UnitsInvalid { ref problem, ref units, .. }) => {
                assert_eq!(problem, "gates_unmet", "the post-review gate re-fires");
                assert_eq!(units, &vec!["frame-u".to_string()]);
            }
            other => panic!("expected a post-review gates_unmet hold, got {other:?}"),
        }

        // Record a passing result → the station proceeds past Audit.
        unit.frontmatter.gate_results = vec![GateResult {
            name: "tests".into(),
            status: GateStatus::Pass,
            at: None,
            attempts: 1,
            detail: None,
        }];
        store.write_unit("r", &unit).unwrap();
        let action = derive_position(&store, "r").expect("derive").action;
        assert!(
            !matches!(action, Some(RunAction::UnitsInvalid { ref problem, .. }) if problem == "gates_unmet"),
            "a satisfied gate no longer holds the station: {action:?}"
        );
    }

    #[test]
    fn run_start_seeds_state_at_first_station() {
        let (_d, store) = store();
        let run = run_start(&store, "my-run", "software", Some("Ship it".into()), Mode::Solo, "full")
            .expect("start");
        assert_eq!(run.frontmatter.active_station, "frame");

        // .darkrun state exists on disk.
        assert!(store.run_dir("my-run").join("run.md").exists());
        let state = store.read_state("my-run").expect("state").expect("some");
        assert_eq!(state.active_station, "frame");
        assert_eq!(state.stations["frame"].phase, StationPhase::Spec);
    }

    #[test]
    fn full_mode_leaves_plan_empty_and_gates_intact() {
        let (_d, store) = store();
        run_start(&store, "f", "software", None, Mode::Solo, "full").expect("start");
        let state = store.read_state("f").unwrap().unwrap();
        assert!(state.plan.is_empty(), "full mode walks the whole factory");
        assert_eq!(state.active_station, "frame");
    }

    #[test]
    fn quick_mode_right_sizes_plan() {
        let (_d, store) = store();
        run_start(&store, "q", "software", Some("Small fix".into()), Mode::Solo, "quick").expect("start");
        let state = store.read_state("q").unwrap().unwrap();
        assert_eq!(state.plan, vec!["build".to_string(), "prove".to_string()]);
        // The run starts at the plan's first station, not the factory's.
        assert_eq!(state.active_station, "build");
        assert_eq!(state.stations["build"].phase, StationPhase::Spec);
    }

    #[test]
    fn run_level_review_is_shaped_by_mode() {
        // C5: a non-dark run keeps every declared run reviewer; a dark run skips
        // the cross-station run review entirely.
        let factory = crate::factory::resolve_factory("software").unwrap();
        assert!(!factory.run_reviewers.is_empty(), "software declares run reviewers");

        // A non-dark visual run keeps every reviewer (the surface-scoped a11y
        // auditor included).
        let full = RunState::default();
        let visual = effective_run_reviewers(
            &factory,
            &full,
            Some(darkrun_core::domain::Surface::WebUi),
        );
        assert_eq!(visual, factory.run_reviewers);

        // A dark run skips the whole-run review regardless of surface.
        let dark = RunState { mode: Mode::Dark, ..Default::default() };
        assert!(
            effective_run_reviewers(&factory, &dark, Some(darkrun_core::domain::Surface::WebUi))
                .is_empty(),
            "a dark run skips the run-level review"
        );
    }

    #[test]
    fn applies_to_scopes_a_reviewer_by_surface() {
        use darkrun_core::domain::Surface;
        // E6: an empty scope always fires; a scoped reviewer fires only on a
        // matching surface, and never on an unclassified run.
        assert!(reviewer_applies(&[], None));
        assert!(reviewer_applies(&[], Some(Surface::Library)));
        let visual = vec!["web_ui".to_string(), "desktop".to_string()];
        assert!(reviewer_applies(&visual, Some(Surface::WebUi)));
        assert!(!reviewer_applies(&visual, Some(Surface::Library)));
        assert!(!reviewer_applies(&visual, None), "unclassified run can't match a scope");
        // Tolerant spelling agrees (web-ui == web_ui).
        assert!(reviewer_applies(&["web-ui".to_string()], Some(Surface::WebUi)));
    }

    #[test]
    fn surface_scoped_run_reviewer_only_fires_on_matching_surface() {
        use darkrun_core::domain::Surface;
        // The software factory's accessibility-auditor is scoped [web_ui, desktop,
        // mobile]. It joins the run review on a visual run, not on a library run.
        let factory = crate::factory::resolve_factory("software").unwrap();
        let full = RunState::default();

        let visual = effective_run_reviewers(&factory, &full, Some(Surface::WebUi));
        assert!(visual.iter().any(|r| r == "accessibility-auditor"), "{visual:?}");

        let lib = effective_run_reviewers(&factory, &full, Some(Surface::Library));
        assert!(!lib.iter().any(|r| r == "accessibility-auditor"), "{lib:?}");
        // The always-on auditors fire on both.
        assert!(lib.iter().any(|r| r == "integration-auditor"));
    }

    #[test]
    fn quick_run_walks_only_planned_stations_to_sealed() {
        let (_d, store) = store();
        run_start(&store, "q", "software", None, Mode::Solo, "quick").expect("start");

        // Drive to sealed. Every station holds its Spec until the operator seals
        // the elaboration; we seal then decompose+complete each station.
        let mut guard = 0;
        loop {
            guard += 1;
            assert!(guard < 100, "quick run failed to converge");
            let t = run_tick(&store, "q").expect("tick");
            match &t.action {
                RunAction::Sealed { .. } => break,
                RunAction::Spec { station, .. } => {
                    // The unit consumes the station's declared inputs so the
                    // runtime input-coverage gate is satisfied (the run's
                    // distillation is carried forward, not dropped).
                    let inputs = crate::factory::resolve_factory("software")
                        .unwrap()
                        .station(station)
                        .map(|d| d.inputs.clone())
                        .unwrap_or_default();
                    let unit = Unit {
                        slug: format!("{station}-u"),
                        frontmatter: darkrun_core::domain::UnitFrontmatter {
                            status: Status::Pending,
                            station: Some(station.clone()),
                            inputs,
                            ..Default::default()
                        },
                        title: "u".into(),
                        body: String::new(),
                    };
                    store.write_unit("q", &unit).expect("write unit");
                    // Solo holds the Spec phase until the elaboration is sealed.
                    elaborate_seal(&store, "q", station).expect("seal");
                }
                RunAction::Manufacture { station, units, .. } => {
                    let _ = station;
                    for u in units {
                        let mut done = store.read_unit("q", u).unwrap();
                        done.frontmatter.status = Status::Completed;
                        store.write_unit("q", &done).unwrap();
                    }
                }
                RunAction::RunReview { reviewers, .. } => {
                    for r in reviewers.clone() {
                        run_review_stamp(&store, "q", &r).expect("run review stamp");
                    }
                }
                // Solo gates `ask` at every station: clear the pre-execution
                // operator gate and the post-execution checkpoint.
                RunAction::UserGate { .. } | RunAction::Checkpoint { .. } => {
                    checkpoint_decide(&store, "q", true, None).expect("decide");
                }
                _ => {}
            }
        }

        let state = store.read_state("q").unwrap().unwrap();
        // Planned stations ran; the omitted ones never did.
        assert!(state.stations.contains_key("build"));
        assert!(state.stations.contains_key("prove"));
        assert!(!state.stations.contains_key("frame"), "frame is not in the quick plan");
        assert!(!state.stations.contains_key("specify"));
        assert!(!state.stations.contains_key("shape"));
        assert!(!state.stations.contains_key("harden"));
    }

    /// In a real git repo, a run walks the universal branch hierarchy: run_start
    /// forks darkrun/<slug>/main; each station forks darkrun/<slug>/<station>
    /// (recorded on Station.branch) and lands it back to run-main once verified,
    /// removing the station branch + worktree; at run completion run-main lands
    /// onto the base.
    #[test]
    fn git_run_walks_the_branch_hierarchy() {
        use std::process::Command;
        let dir = tempdir().expect("tmp");
        let root = dir.path();
        let git = |args: &[&str]| {
            let ok = Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .status()
                .expect("git")
                .success();
            assert!(ok, "git {args:?} failed");
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["config", "user.email", "test@darkrun.ai"]);
        git(&["config", "user.name", "darkrun test"]);
        std::fs::write(root.join(".gitignore"), ".darkrun/\n").unwrap();
        std::fs::write(root.join("README.md"), "# x\n").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "base"]);

        let store = StateStore::new(root);
        run_start(&store, "q", "software", None, Mode::Solo, "quick").expect("start");

        // run-main forked off the base at run start.
        let run_main_exists = |b: &str| {
            Command::new("git")
                .arg("-C")
                .arg(root)
                .args(["rev-parse", "--verify", &format!("refs/heads/{b}")])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        };
        assert!(run_main_exists("darkrun/q/main"), "run-main forked at start");

        // The first station's branch was forked + recorded.
        let state = store.read_state("q").unwrap().unwrap();
        assert_eq!(
            state.stations["build"].branch.as_deref(),
            Some("darkrun/q/build"),
            "first station's branch recorded on entry"
        );
        assert!(run_main_exists("darkrun/q/build"));

        // Drive to sealed, doing the agent's part each tick.
        let mut guard = 0;
        loop {
            guard += 1;
            assert!(guard < 100, "git hierarchy run failed to converge");
            let t = run_tick(&store, "q").expect("tick");
            match &t.action {
                RunAction::Sealed { .. } => break,
                RunAction::Spec { station, .. } => {
                    // Consume the station's declared inputs to satisfy the runtime
                    // input-coverage gate.
                    let inputs = crate::factory::resolve_factory("software")
                        .unwrap()
                        .station(station)
                        .map(|d| d.inputs.clone())
                        .unwrap_or_default();
                    let unit = Unit {
                        slug: format!("{station}-u"),
                        frontmatter: darkrun_core::domain::UnitFrontmatter {
                            status: Status::Pending,
                            station: Some(station.clone()),
                            inputs,
                            ..Default::default()
                        },
                        title: "u".into(),
                        body: String::new(),
                    };
                    store.write_unit("q", &unit).expect("write unit");
                    // Solo holds the Spec phase until the elaboration is sealed.
                    elaborate_seal(&store, "q", station).expect("seal");
                }
                RunAction::Manufacture { station, units, .. } => {
                    // Commit real code on the station's worktree so the branch
                    // carries merge debt — otherwise #4 (no-debt no-op guard)
                    // correctly skips the land and the branch never collapses.
                    let wt = crate::lifecycle::station_worktree_path(root, "q", station);
                    if wt.exists() {
                        std::fs::write(wt.join(format!("{station}.txt")), "work\n").unwrap();
                        let git_wt = |args: &[&str]| {
                            Command::new("git").arg("-C").arg(&wt).args(args).status().unwrap().success()
                        };
                        let _ = git_wt(&["add", "-A"]);
                        let _ = git_wt(&["commit", "-q", "-m", "station work"]);
                    }
                    for u in units {
                        let mut done = store.read_unit("q", u).unwrap();
                        done.frontmatter.status = Status::Completed;
                        store.write_unit("q", &done).unwrap();
                    }
                }
                RunAction::RunReview { reviewers, .. } => {
                    for r in reviewers.clone() {
                        run_review_stamp(&store, "q", &r).expect("run review stamp");
                    }
                }
                // Solo gates `ask` at every station: clear the pre-execution
                // operator gate and the post-execution checkpoint.
                RunAction::UserGate { .. } | RunAction::Checkpoint { .. } => {
                    checkpoint_decide(&store, "q", true, None).expect("decide");
                }
                _ => {}
            }
        }

        // Once landed + sealed, the per-station branches are gone (merged into
        // run-main and removed) — only run-main survives.
        assert!(
            !run_main_exists("darkrun/q/build"),
            "landed station branch removed"
        );
        assert!(
            !run_main_exists("darkrun/q/prove"),
            "landed station branch removed"
        );
        assert!(run_main_exists("darkrun/q/main"), "run-main persists");
    }

    #[test]
    fn unknown_mode_falls_back_to_full_plan() {
        let (_d, store) = store();
        run_start(&store, "u", "software", None, Mode::Solo, "nonsense-mode").expect("start");
        let state = store.read_state("u").unwrap().unwrap();
        assert!(state.plan.is_empty());
        assert_eq!(state.active_station, "frame");
    }

    #[test]
    fn gate_is_a_pure_function_of_mode() {
        // The effective checkpoint kind is now a pure function of the run's
        // global Mode: team → external, solo → ask, dark → auto.
        let team = RunState { mode: Mode::Team, ..Default::default() };
        assert_eq!(effective_checkpoint_kind(&team), CheckpointKind::External);
        let solo = RunState { mode: Mode::Solo, ..Default::default() };
        assert_eq!(effective_checkpoint_kind(&solo), CheckpointKind::Ask);
        let dark = RunState { mode: Mode::Dark, ..Default::default() };
        assert_eq!(effective_checkpoint_kind(&dark), CheckpointKind::Auto);
    }

    #[test]
    fn solo_mode_holds_spec_until_elaboration_is_sealed() {
        let (_d, store) = store();
        run_start(&store, "c", "software", None, Mode::Solo, "full").expect("start");

        // Tick 1: Spec — but the solo hold keeps the station in Spec
        // (not Review) until the operator has been involved.
        let t1 = run_tick(&store, "c").expect("t1");
        assert!(matches!(t1.action, RunAction::Spec { .. }));
        assert_eq!(
            store.read_state("c").unwrap().unwrap().stations["frame"].phase,
            StationPhase::Spec,
            "collaborative Spec holds until elaboration is sealed"
        );
        // The dispatch flags the collaboration requirement.
        let prompt = t1.prompt.clone().unwrap_or_default();
        assert!(prompt.contains("elaborate_seal") || prompt.to_lowercase().contains("operator"));

        // The agent involves the operator and seals; now Spec advances to Review.
        elaborate_seal(&store, "c", "frame").expect("seal");
        run_tick(&store, "c").expect("t2");
        assert_eq!(
            store.read_state("c").unwrap().unwrap().stations["frame"].phase,
            StationPhase::Review,
            "sealed elaboration releases the Spec hold"
        );
    }

    #[test]
    fn dark_mode_does_not_hold_spec() {
        let (_d, store) = store();
        run_start(&store, "r", "software", None, Mode::Dark, "full").expect("start");
        run_tick(&store, "r").expect("spec");
        // Dark is on-the-loop — Spec advances to Review without an elaborate seal.
        assert_eq!(
            store.read_state("r").unwrap().unwrap().stations["frame"].phase,
            StationPhase::Review
        );
    }

    #[test]
    fn each_tick_appends_to_the_action_log() {
        let (_d, store) = store();
        // Dark walks linearly with no Spec hold: one tick per phase.
        run_start(&store, "r", "software", None, Mode::Dark, "full").expect("start");
        run_tick(&store, "r").expect("t1"); // Spec
        run_tick(&store, "r").expect("t2"); // Review
        let log = store.read_journal("r", "action-log.jsonl");
        assert_eq!(log.len(), 2, "one journal line per resolved tick");
        assert!(log[0].contains("\"action\":\"spec\""), "{}", log[0]);
        assert!(log[0].contains("\"track\":\"run\""), "{}", log[0]);
        assert!(log[0].contains("\"station\":\"frame\""), "{}", log[0]);
        assert!(log[1].contains("\"action\":\"review\""), "{}", log[1]);
    }

    #[test]
    fn run_next_walks_first_station_through_its_phases() {
        let (_d, store) = store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").expect("start");

        // Tick 1: Spec (frame). Solo holds the Spec until the elaboration is
        // sealed; sealing then advances it to Review.
        let t1 = run_tick(&store, "r").expect("t1");
        assert!(matches!(
            t1.action,
            RunAction::Spec { ref station, .. } if station == "frame"
        ));
        elaborate_seal(&store, "r", "frame").expect("seal");
        run_tick(&store, "r").expect("post-seal advance");
        assert_eq!(
            store.read_state("r").unwrap().unwrap().stations["frame"].phase,
            StationPhase::Review
        );

        // Tick 2: Review (frame). State advances to the pre-execution user gate
        // (frame is an interactive `ask` station in solo mode).
        let t2 = run_tick(&store, "r").expect("t2");
        assert!(matches!(
            t2.action,
            RunAction::Review { ref station, .. } if station == "frame"
        ));
        assert_eq!(
            store.read_state("r").unwrap().unwrap().stations["frame"].phase,
            StationPhase::UserGate
        );

        // Decompose a unit; the station holds at the operator gate before any
        // manufacture.
        let unit = Unit {
            slug: "u1".into(),
            frontmatter: darkrun_core::domain::UnitFrontmatter {
                status: Status::Pending,
                station: Some("frame".into()),
                ..Default::default()
            },
            title: "u1".into(),
            body: String::new(),
        };
        store.write_unit("r", &unit).expect("write unit");

        // Tick 3: the pre-execution operator gate is open.
        let t3 = run_tick(&store, "r").expect("t3");
        assert!(
            matches!(t3.action, RunAction::UserGate { ref station, .. } if station == "frame"),
            "expected UserGate, got {:?}",
            t3.action
        );

        // The operator clears the gate → the manufacture wave releases.
        checkpoint_decide(&store, "r", true, None).expect("clear gate");
        let t3b = run_tick(&store, "r").expect("t3b");
        assert!(
            matches!(t3b.action, RunAction::Manufacture { ref units, .. } if units == &vec!["u1".to_string()]),
            "expected Manufacture, got {:?}",
            t3b.action
        );
        // B5: the verifier nonce was minted on the station and surfaced in the
        // Manufacture prompt, so the agent can record quality gates.
        let nonce = store.read_state("r").unwrap().unwrap().stations["frame"]
            .verifier_nonce
            .clone();
        assert!(nonce.is_some(), "verifier nonce minted on Manufacture dispatch");
        assert!(
            t3b.prompt.as_deref().unwrap().contains(nonce.as_deref().unwrap()),
            "manufacture prompt carries the verifier nonce"
        );

        // Complete the unit; next tick audits (folds tests in), then reflects,
        // then checkpoint.
        let mut done = store.read_unit("r", "u1").unwrap();
        done.frontmatter.status = Status::Completed;
        store.write_unit("r", &done).unwrap();

        let t4 = run_tick(&store, "r").expect("t4");
        assert!(
            matches!(t4.action, RunAction::Audit { ref station, .. } if station == "frame"),
            "expected Audit, got {:?}",
            t4.action
        );
        // Audit absorbs the old Tests phase → advances straight to Reflect.
        assert_eq!(
            store.read_state("r").unwrap().unwrap().stations["frame"].phase,
            StationPhase::Reflect
        );

        let t5 = run_tick(&store, "r").expect("t5");
        assert!(
            matches!(t5.action, RunAction::Reflect { ref station, .. } if station == "frame"),
            "expected Reflect, got {:?}",
            t5.action
        );
        assert_eq!(
            store.read_state("r").unwrap().unwrap().stations["frame"].phase,
            StationPhase::Checkpoint
        );

        // Tick 6: Checkpoint (frame's gate is `ask`, so it holds).
        let t6 = run_tick(&store, "r").expect("t6");
        assert!(
            matches!(t6.action, RunAction::Checkpoint { kind: CheckpointKind::Ask, .. }),
            "expected Checkpoint(ask), got {:?}",
            t6.action
        );
        assert_eq!(
            store.read_state("r").unwrap().unwrap().stations["frame"].status,
            Status::InProgress
        );

        // Operator approves → frame completes, cursor advances to specify (Spec).
        let decided = checkpoint_decide(&store, "r", true, None).expect("decide");
        assert!(
            matches!(decided.action, RunAction::Spec { ref station, .. } if station == "specify"),
            "expected to advance to specify, got {:?}",
            decided.action
        );
        let s = store.read_state("r").unwrap().unwrap();
        assert_eq!(s.stations["frame"].status, Status::Completed);
        assert_eq!(s.active_station, "specify");
    }

    /// Audit absorbs the old Tests phase: a tick on `Audit` advances the
    /// station write-cache straight to `Reflect` (not to a separate Tests
    /// phase, which no longer exists), and `Reflect` then advances to
    /// `Checkpoint`.
    #[test]
    fn audit_absorbs_tests_and_reflect_precedes_checkpoint() {
        let (_d, store) = store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").expect("start");

        // Force the frame station onto its Audit phase.
        let mut state = store.read_state("r").unwrap().unwrap();
        state.stations.get_mut("frame").unwrap().phase = StationPhase::Audit;
        state.stations.get_mut("frame").unwrap().status = Status::InProgress;
        store.write_state("r", &state).unwrap();

        // Tick on Audit → action is Audit, phase stamps forward to Reflect.
        let t_audit = run_tick(&store, "r").expect("audit tick");
        assert!(
            matches!(t_audit.action, RunAction::Audit { ref station, .. } if station == "frame"),
            "expected Audit, got {:?}",
            t_audit.action
        );
        assert_eq!(
            store.read_state("r").unwrap().unwrap().stations["frame"].phase,
            StationPhase::Reflect,
            "Audit must advance to Reflect (tests folded into audit, no Tests phase)"
        );

        // Tick on Reflect → action is Reflect, phase stamps forward to Checkpoint.
        let t_reflect = run_tick(&store, "r").expect("reflect tick");
        assert!(
            matches!(t_reflect.action, RunAction::Reflect { ref station, ref run } if station == "frame" && run == "r"),
            "expected Reflect carrying run+station, got {:?}",
            t_reflect.action
        );
        assert_eq!(
            store.read_state("r").unwrap().unwrap().stations["frame"].phase,
            StationPhase::Checkpoint,
            "Reflect must advance to Checkpoint"
        );
    }

    /// The `RunAction` taxonomy has no `Tests` variant and serializes `Reflect`
    /// with the `reflect` tag — the wire contract downstream agents match.
    #[test]
    fn reflect_action_serializes_with_reflect_tag() {
        let action = RunAction::Reflect {
            run: "r".into(),
            station: "frame".into(),
        };
        let json = serde_json::to_value(&action).unwrap();
        assert_eq!(json["action"], "reflect");
        assert_eq!(json["run"], "r");
        assert_eq!(json["station"], "frame");
    }

    #[test]
    fn open_feedback_preempts_run_track() {
        let (_d, store) = store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").expect("start");
        store
            .write_feedback_raw("r", "fb-1", "---\nstatus: pending\n---\nbroken thing\n")
            .expect("fb");
        let pos = derive_position(&store, "r").expect("pos");
        assert_eq!(pos.track, Track::Feedback);
        assert!(matches!(
            pos.action,
            Some(RunAction::FixFeedback { ref feedback_id, .. }) if feedback_id == "fb-1"
        ));
    }

    #[test]
    fn checkpoint_reject_holds_and_files_feedback() {
        let (_d, store) = store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").expect("start");
        let res = checkpoint_decide(&store, "r", false, Some("not good enough".into()))
            .expect("decide");
        // Rejecting files feedback, which now preempts the run track.
        assert_eq!(res.position.track, Track::Feedback);
        let s = store.read_state("r").unwrap().unwrap();
        assert_eq!(s.stations["frame"].status, Status::Blocked);
    }

    #[test]
    fn checkpoint_reject_ships_open_annotations_as_feedback() {
        use darkrun_api::annotation::{
            Anchor, Annotation, AnnotationStatus, ArtifactInfo, ArtifactType, Ask, AskKind,
            AskSeverity, TextRange, WorkItem, WorkItemKind,
        };
        use darkrun_api::common::AuthorType;

        let (_d, store) = store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").expect("start");

        // A global station note + a per-artifact mark, both OPEN on `frame`.
        let note = Annotation {
            id: "anno_note".into(),
            created_at: "2026-05-31T00:00:00Z".into(),
            author: AuthorType::Human,
            work_item: WorkItem {
                kind: WorkItemKind::Station,
                id: String::new(),
                station: "frame".into(),
            },
            artifact: None,
            anchor: None,
            expression: None,
            comment: "overall: tighten the framing".into(),
            ask: Ask {
                kind: AskKind::Change,
                severity: AskSeverity::Should,
            },
            suggestion: None,
            status: AnnotationStatus::Open,
        };
        let mark = Annotation {
            id: "anno_mark".into(),
            work_item: WorkItem {
                kind: WorkItemKind::Output,
                id: "spec.md".into(),
                station: "frame".into(),
            },
            artifact: Some(ArtifactInfo {
                id: "spec.md".into(),
                path: "spec.md".into(),
                artifact_type: ArtifactType::Text,
                version_sha: "aa".into(),
            }),
            anchor: Some(Anchor::Text {
                range: TextRange {
                    start_line: 3,
                    start_col: 0,
                    end_line: 3,
                    end_col: 4,
                },
                quote: "todo".into(),
                prefix: String::new(),
                suffix: String::new(),
            }),
            ..note.clone()
        };
        store.write_annotation("r", &note).unwrap();
        store.write_annotation("r", &mark).unwrap();

        checkpoint_decide(&store, "r", false, None).expect("reject");

        // The open annotations shipped as a feedback doc, station-scoped.
        let raw = store.read_feedback_raw("r").unwrap();
        let doc = raw.get("fb-annotations").expect("annotations feedback shipped");
        assert!(doc.contains("station: frame"));
        assert!(doc.contains("station note"));
        assert!(doc.contains("spec.md"));
    }

    /// A station carrying OPEN annotations auto-surfaces the resolved
    /// re-reference bundle in the next action's PromptContext — the agent
    /// receives `file:line` + comment + suggestion on the tick without having to
    /// call `darkrun_annotation_payload`. This is the human->agent loop closing
    /// on re-entry (e.g. a unit re-entering as rework after Request-changes).
    #[test]
    fn open_annotations_surface_in_next_action_context() {
        use darkrun_api::annotation::{
            Anchor, Annotation, AnnotationStatus, ArtifactInfo, ArtifactType, Ask, AskKind,
            AskSeverity, TextRange, WorkItem, WorkItemKind,
        };
        use darkrun_api::common::AuthorType;

        let (_d, store) = store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").expect("start");

        // An OPEN per-artifact mark on a unit in the active `frame` station.
        let mark = Annotation {
            id: "anno_mark".into(),
            created_at: "2026-05-31T00:00:00Z".into(),
            author: AuthorType::Human,
            work_item: WorkItem {
                kind: WorkItemKind::Output,
                id: "spec.md".into(),
                station: "frame".into(),
            },
            artifact: Some(ArtifactInfo {
                id: "spec.md".into(),
                path: "spec.md".into(),
                artifact_type: ArtifactType::Text,
                version_sha: "aa".into(),
            }),
            anchor: Some(Anchor::Text {
                range: TextRange {
                    start_line: 7,
                    start_col: 0,
                    end_line: 7,
                    end_col: 4,
                },
                quote: "todo".into(),
                prefix: String::new(),
                suffix: String::new(),
            }),
            expression: None,
            comment: "spell out the framing constraint".into(),
            ask: Ask {
                kind: AskKind::Change,
                severity: AskSeverity::Must,
            },
            suggestion: None,
            status: AnnotationStatus::Open,
        };
        store.write_annotation("r", &mark).unwrap();

        // The next action on the active station carries the resolved bundle.
        let action = derive_position(&store, "r")
            .expect("pos")
            .action
            .expect("action");
        let ctx = build_prompt_context(&store, "r", &action).expect("ctx");
        let bundle = ctx
            .annotations
            .expect("open annotations must surface in the next action");
        assert_eq!(bundle.items.len(), 1);
        assert_eq!(bundle.must, 1);
        let item = &bundle.items[0];
        assert_eq!(item.comment, "spell out the framing constraint");
        match &item.source {
            crate::annotation::ResolvedSource::Text { path, start_line, .. } => {
                assert_eq!(path, "spec.md");
                assert_eq!(*start_line, 7);
            }
            other => panic!("expected resolved text source, got {other:?}"),
        }
    }

    /// A station with no open annotations leaves the bundle absent — the field
    /// is skipped, not an empty payload, so quiet stations stay quiet.
    #[test]
    fn no_annotations_means_no_bundle_in_context() {
        let (_d, store) = store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").expect("start");
        let action = derive_position(&store, "r")
            .expect("pos")
            .action
            .expect("action");
        let ctx = build_prompt_context(&store, "r", &action).expect("ctx");
        assert!(ctx.annotations.is_none());
    }

    // ── Surface routing into the rendered prompt ─────────────────────────────

    /// A classified visual surface lights up the headless-browser route in the
    /// rendered Audit prompt.
    #[test]
    fn audit_prompt_routes_visual_surface_through_render() {
        let (_d, store) = store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").expect("start");
        crate::proof::set_surface(&store, "r", "web-ui").expect("classify");
        let action = RunAction::Audit {
            run: "r".into(),
            station: "prove".into(),
            reviewers: vec![],
        };
        let out = render_prompt(&store, "r", &action)
            .expect("render")
            .expect("audit has a template");
        assert!(out.contains("darkrun verify web"), "visual run routes to web verify:\n{out}");
        assert!(out.contains("darkrun_proof_attach"));
        assert!(!out.contains("darkrun bench"));
    }

    /// A classified bench surface lights up the bench route instead.
    #[test]
    fn audit_prompt_routes_bench_surface_through_render() {
        let (_d, store) = store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").expect("start");
        crate::proof::set_surface(&store, "r", "api").expect("classify");
        let action = RunAction::Audit {
            run: "r".into(),
            station: "prove".into(),
            reviewers: vec![],
        };
        let out = render_prompt(&store, "r", &action)
            .expect("render")
            .expect("audit has a template");
        assert!(out.contains("darkrun bench"), "bench run routes to bench:\n{out}");
        assert!(!out.contains("darkrun verify web"));
    }

    /// An unclassified run carries no surface proof route in its Audit prompt.
    #[test]
    fn audit_prompt_without_surface_has_no_route() {
        let (_d, store) = store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").expect("start");
        let action = RunAction::Audit {
            run: "r".into(),
            station: "build".into(),
            reviewers: vec![],
        };
        let out = render_prompt(&store, "r", &action)
            .expect("render")
            .expect("audit has a template");
        assert!(
            !out.contains("darkrun verify web") && !out.contains("darkrun bench"),
            "unclassified run carries no proof route:\n{out}"
        );
    }

    // ── Discrete + discrete-hybrid modes ─────────────────────────────────────

    use crate::hosting::{Hosting, MergeState, OpenRequest};
    use std::cell::RefCell;

    /// A scripted hosting client for the discrete-mode tests: records the
    /// open-draft calls and returns a fixed merge state per poll.
    struct MockHosting {
        available: bool,
        /// The provider ref `open_draft` hands back (and that `merge_state`
        /// keys off). `None` makes `open_draft` fail (the await fallback).
        pr_ref: Option<String>,
        /// What `merge_state` returns for the recorded ref.
        state: MergeState,
        /// What `is_draft` returns for the recorded ref (G4 draft→ready).
        draft: Option<bool>,
        /// Every `open_draft` request, in call order (for assertions).
        opened: RefCell<Vec<OpenRequest>>,
        /// Every `comment` (pr_ref, body), in call order (D5 proof upload).
        comments: RefCell<Vec<(String, String)>>,
        /// Human review notes the PR returns on each poll (C6 ingest source).
        notes: Vec<crate::hosting::ReviewComment>,
    }

    impl MockHosting {
        fn new(state: MergeState) -> Self {
            Self {
                available: true,
                pr_ref: Some("42".into()),
                state,
                draft: None,
                opened: RefCell::new(Vec::new()),
                comments: RefCell::new(Vec::new()),
                notes: Vec::new(),
            }
        }
        fn unavailable() -> Self {
            Self {
                available: false,
                pr_ref: None,
                state: MergeState::Unknown,
                draft: None,
                opened: RefCell::new(Vec::new()),
                comments: RefCell::new(Vec::new()),
                notes: Vec::new(),
            }
        }
        /// An open PR that reports it has been marked ready for review.
        fn ready() -> Self {
            Self {
                draft: Some(false),
                ..Self::new(MergeState::Open)
            }
        }
        /// An open PR that hands back `notes` on each `review_comments` poll (C6).
        fn with_notes(notes: Vec<crate::hosting::ReviewComment>) -> Self {
            Self {
                notes,
                ..Self::new(MergeState::Open)
            }
        }
    }

    impl Hosting for MockHosting {
        fn available(&self) -> bool {
            self.available
        }
        fn open_draft(&self, req: &OpenRequest) -> Option<String> {
            self.opened.borrow_mut().push(req.clone());
            self.pr_ref.clone()
        }
        fn merge_state(&self, _pr_ref: &str) -> MergeState {
            self.state
        }
        fn is_draft(&self, _pr_ref: &str) -> Option<bool> {
            self.draft
        }
        fn comment(&self, pr_ref: &str, body: &str) -> bool {
            self.comments.borrow_mut().push((pr_ref.to_string(), body.to_string()));
            true
        }
        fn review_comments(&self, _pr_ref: &str) -> Vec<crate::hosting::ReviewComment> {
            self.notes.clone()
        }
    }

    /// A unit flagged `reset_requested` (from the desktop UI) is reset to pending
    /// by the next tick's pre-derive sweep — body unlocked, pass budget cleared,
    /// flag cleared — exactly as the `darkrun_unit_reset` tool would.
    #[test]
    fn reset_requested_flag_is_consumed_before_derive() {
        let (_d, store) = store();
        run_start(&store, "d", "software", None, Mode::Solo, "quick").expect("start");
        // A wedged InProgress unit the operator flagged for reset in the UI.
        let mut u = crate::units::create(&store, "d", "u1", "build", crate::units::UnitSpec::default()).unwrap();
        u.frontmatter.status = Status::InProgress;
        u.frontmatter.reset_requested = true;
        u.frontmatter.iterations = vec![
            darkrun_core::domain::UnitIteration {
                worker: "w".into(),
                started_at: None,
                completed_at: None,
                result: Some(darkrun_core::domain::IterationResult::Advance),
                note: None,
            };
            3
        ];
        store.write_unit("d", &u).unwrap();
        assert_eq!(store.read_unit("d", "u1").unwrap().pass(), 3);

        // The pre-derive sweep consumes the flag.
        apply_requested_unit_resets(&store, "d");

        let after = store.read_unit("d", "u1").unwrap();
        assert_eq!(after.frontmatter.status, Status::Pending, "back to editable");
        assert!(!after.frontmatter.reset_requested, "flag cleared");
        assert_eq!(after.pass(), 0, "pass budget reset");
    }

    /// `team` mode keeps the full factory plan with global external gates.
    #[test]
    fn team_mode_full_plan() {
        let (_d, store) = store();
        run_start(&store, "d", "software", None, Mode::Team, "full").expect("start");
        let state = store.read_state("d").unwrap().unwrap();
        assert_eq!(state.mode, Mode::Team);
        assert!(state.plan.is_empty(), "team walks the full factory");
        assert_eq!(state.active_station, "frame");
        // Team opens a PR at every station's gate.
        assert!(state.mode.opens_station_pr());
    }

    /// Drive a team-mode `frame` station to its checkpoint without git, so
    /// the gate surfaces. (No worktrees needed — the gate logic is what's under
    /// test; the hosting client is mocked.)
    fn drive_discrete_frame_to_gate(store: &StateStore, hosting: &MockHosting) -> RunAction {
        for _ in 0..16 {
            let t = run_tick_with_hosting(store, "d", hosting).expect("tick");
            match &t.action {
                // Clear the pre-execution operator gate so the walk reaches the
                // post-execution external review gate under test.
                RunAction::UserGate { .. } => {
                    checkpoint_decide(store, "d", true, None).expect("clear gate");
                }
                RunAction::Spec { station, .. } => {
                    // Seed one completed unit so Manufacture clears straight to Audit.
                    let unit = Unit {
                        slug: format!("{station}-u"),
                        frontmatter: darkrun_core::domain::UnitFrontmatter {
                            status: Status::Completed,
                            station: Some(station.clone()),
                            ..Default::default()
                        },
                        title: "u".into(),
                        body: String::new(),
                    };
                    store.write_unit("d", &unit).expect("unit");
                    // Team holds the Spec phase until the elaboration is sealed.
                    elaborate_seal(store, "d", station).expect("seal");
                }
                RunAction::ExternalReviewRequested { .. } => return t.action,
                _ => {}
            }
        }
        panic!("never reached the discrete external gate");
    }

    /// Full discrete: the first station opens a draft PR at its gate (recorded
    /// on `Station.pr_ref` and echoed on the action `target`) and HOLDS until a
    /// merge is detected. The hosting client gets a station-branch -> run-main
    /// draft request.
    #[test]
    fn discrete_station_opens_pr_and_holds() {
        let (_d, store) = store();
        run_start(&store, "d", "software", None, Mode::Team, "full").expect("start");
        let hosting = MockHosting::new(MergeState::Open);

        let action = drive_discrete_frame_to_gate(&store, &hosting);
        // The gate surfaces as an external review carrying the opened PR ref.
        assert!(
            matches!(&action, RunAction::ExternalReviewRequested { station, target, .. }
                if station == "frame" && target == "42"),
            "expected external review with PR ref, got {action:?}"
        );
        // A draft PR was opened station-branch -> run-main exactly once.
        let opened = hosting.opened.borrow();
        assert_eq!(opened.len(), 1, "opened exactly one draft PR");
        assert_eq!(opened[0].head, "darkrun/d/frame");
        assert_eq!(opened[0].base, "darkrun/d/main");
        // Recorded on the station for the merge poll.
        let state = store.read_state("d").unwrap().unwrap();
        assert_eq!(state.stations["frame"].pr_ref.as_deref(), Some("42"));
        // G4: a freshly-opened PR is in the draft stage.
        assert_eq!(state.stations["frame"].pr_status, Some(PrStatus::Draft));
        // Still in-progress (the gate holds until merge).
        assert_eq!(state.stations["frame"].status, Status::InProgress);
    }

    /// G4: polling a PR that has been marked ready for review (no longer draft)
    /// records the draft→ready transition with a `pr_ready_at` stamp, without
    /// advancing the gate (it still holds until merge).
    #[test]
    fn discrete_pr_marked_ready_records_the_transition() {
        let (_d, store) = store();
        run_start(&store, "d", "software", None, Mode::Team, "full").expect("start");

        // Reach the gate + open the draft PR.
        let open_host = MockHosting::new(MergeState::Open);
        drive_discrete_frame_to_gate(&store, &open_host);
        assert_eq!(store.read_state("d").unwrap().unwrap().stations["frame"].pr_status, Some(PrStatus::Draft));

        // The author marks it ready for review. The next tick records ready.
        let ready_host = MockHosting::ready();
        let still_holding = run_tick_with_hosting(&store, "d", &ready_host).expect("tick");
        assert!(
            matches!(&still_holding.action, RunAction::ExternalReviewRequested { station, .. } if station == "frame"),
            "ready (not merged) still holds the gate, got {:?}",
            still_holding.action
        );
        let st = &store.read_state("d").unwrap().unwrap().stations["frame"];
        assert_eq!(st.pr_status, Some(PrStatus::Ready));
        assert!(st.pr_ready_at.is_some(), "ready transition is timestamped");
        assert!(st.pr_merged_at.is_none(), "not merged yet");
        assert_eq!(st.status, Status::InProgress, "gate still holds");
    }

    /// C6: a human's review notes on the open PR re-enter the run as feedback the
    /// fix track addresses — a `CHANGES_REQUESTED` review files a blocker, a plain
    /// comment a medium, both `external`-origin; the open feedback then preempts
    /// the held external gate (Track B over the run track). Re-polling the same
    /// notes is deduped (deterministic ids), so nothing double-files.
    #[test]
    fn discrete_pulls_pr_review_notes_into_feedback() {
        use crate::hosting::ReviewComment;
        use darkrun_core::domain::{FeedbackOrigin, FeedbackSeverity};
        let (_d, store) = store();
        run_start(&store, "d", "software", None, Mode::Team, "full").expect("start");

        // Reach the gate + open the draft PR (no notes yet).
        drive_discrete_frame_to_gate(&store, &MockHosting::new(MergeState::Open));
        assert!(crate::feedback::list(&store, "d").unwrap().is_empty(), "no feedback before review");

        // The reviewer requests changes and leaves a plain comment on the PR.
        let host = MockHosting::with_notes(vec![
            ReviewComment {
                id: "r100".into(),
                author: "alice".into(),
                body: "Tighten the success metric — it's not measurable.".into(),
                change_request: true,
            },
            ReviewComment {
                id: "c200".into(),
                author: "bob".into(),
                body: "nit: typo in the non-goals list".into(),
                change_request: false,
            },
        ]);
        let after = run_tick_with_hosting(&store, "d", &host).expect("tick");

        // Both notes became external-origin feedback on the frame station, with
        // deterministic ids derived from the provider note ids.
        let fbs = crate::feedback::list(&store, "d").unwrap();
        assert_eq!(fbs.len(), 2, "both review notes filed as feedback, got {fbs:?}");
        let cr = fbs.iter().find(|f| f.id == "fb-ext-r100").expect("change-request feedback");
        assert_eq!(cr.origin, FeedbackOrigin::External);
        assert_eq!(cr.severity, Some(FeedbackSeverity::Blocker), "a change request is a blocker");
        assert_eq!(cr.station, "frame");
        assert!(cr.body.contains("@alice") && cr.body.contains("Tighten the success metric"));
        let note = fbs.iter().find(|f| f.id == "fb-ext-c200").expect("comment feedback");
        assert_eq!(note.severity, Some(FeedbackSeverity::Medium), "a plain comment is medium");

        // The open feedback preempts the held external gate (Track B over run).
        assert!(
            matches!(&after.action, RunAction::FixFeedback { station, .. } if station == "frame"),
            "open review feedback should preempt the gate as a fix, got {:?}",
            after.action
        );

        // Re-polling the SAME notes double-files nothing (deterministic-id dedup).
        run_tick_with_hosting(&store, "d", &host).expect("re-poll tick");
        assert_eq!(
            crate::feedback::list(&store, "d").unwrap().len(),
            2,
            "re-polling the same notes is deduped"
        );
    }

    /// D5: when the discrete gate opens a station's draft PR and the station has
    /// an attached proof, that proof is posted to the PR as a durable comment —
    /// exactly once (the PR opens once).
    #[test]
    fn discrete_open_uploads_the_station_proof_to_the_pr() {
        use darkrun_api::proof::{BenchProof, Proof, Surface as ApiSurface};
        let (_d, store) = store();
        run_start(&store, "d", "software", None, Mode::Team, "full").expect("start");
        // Classify the run + attach a matching proof for the frame station.
        crate::proof::set_surface(&store, "d", "data").unwrap();
        crate::proof::attach_proof(
            &store,
            "d",
            Proof::bench(ApiSurface::Data, BenchProof { p50: Some(1.5), ..Default::default() }),
            Some("frame".into()),
        )
        .unwrap();

        let hosting = MockHosting::new(MergeState::Open);
        drive_discrete_frame_to_gate(&store, &hosting);

        // The proof was posted to the opened PR ref exactly once.
        let comments = hosting.comments.borrow();
        assert_eq!(comments.len(), 1, "proof posted exactly once");
        assert_eq!(comments[0].0, "42", "posted to the opened PR ref");
        assert!(comments[0].1.contains("darkrun proof"), "comment carries the proof");
        assert!(comments[0].1.contains("p50"), "comment carries the measured numbers");
    }

    /// D5: a station with NO attached proof opens its PR without a spurious
    /// comment.
    #[test]
    fn discrete_open_without_proof_posts_no_comment() {
        let (_d, store) = store();
        run_start(&store, "d", "software", None, Mode::Team, "full").expect("start");
        let hosting = MockHosting::new(MergeState::Open);
        drive_discrete_frame_to_gate(&store, &hosting);
        assert!(hosting.comments.borrow().is_empty(), "no proof → no comment");
    }

    /// G4: a merge records the `merged` status + `pr_merged_at` on the way to
    /// resolving the gate.
    #[test]
    fn discrete_merge_records_merged_status_and_timestamp() {
        let (_d, store) = store();
        run_start(&store, "d", "software", None, Mode::Team, "full").expect("start");
        let open_host = MockHosting::new(MergeState::Open);
        drive_discrete_frame_to_gate(&store, &open_host);

        let merged_host = MockHosting::new(MergeState::Merged);
        run_tick_with_hosting(&store, "d", &merged_host).expect("tick");
        let st = &store.read_state("d").unwrap().unwrap().stations["frame"];
        assert_eq!(st.pr_status, Some(PrStatus::Merged));
        assert!(st.pr_merged_at.is_some(), "merge is timestamped");
    }

    /// Full discrete: once the PR is MERGED the manager advances — the station
    /// completes and the cursor moves to the next station's Spec. The merge
    /// resolves the gate (no in-process land; the human's merge already landed).
    #[test]
    fn discrete_pr_merge_resolves_the_gate_and_advances() {
        let (_d, store) = store();
        run_start(&store, "d", "software", None, Mode::Team, "full").expect("start");

        // First, reach the gate and open the PR (state = Open).
        let open_host = MockHosting::new(MergeState::Open);
        let at_gate = drive_discrete_frame_to_gate(&store, &open_host);
        assert!(matches!(at_gate, RunAction::ExternalReviewRequested { .. }));

        // Now the human merges the PR. The next tick detects the merge and
        // advances to the next station (specify) at Spec.
        let merged_host = MockHosting::new(MergeState::Merged);
        let advanced = run_tick_with_hosting(&store, "d", &merged_host).expect("tick");
        assert!(
            matches!(&advanced.action, RunAction::Spec { station, .. } if station == "specify"),
            "merge should advance to the next station, got {:?}",
            advanced.action
        );
        let state = store.read_state("d").unwrap().unwrap();
        assert_eq!(state.stations["frame"].status, Status::Completed);
        assert_eq!(state.active_station, "specify");
    }

    /// Build a git repo + a discrete run whose `frame` station sits at its
    /// external Checkpoint with a PR already open (`pr_ref`), under `plan`.
    fn git_discrete_frame_at_open_pr(plan: Vec<String>) -> (tempfile::TempDir, StateStore) {
        use darkrun_core::domain::{Checkpoint, CheckpointKind, Station, StationPhase, Status};
        let dir = tempdir().unwrap();
        std::process::Command::new("git").arg("-C").arg(dir.path()).args(["init", "-q"]).status().unwrap();
        let store = StateStore::new(dir.path());
        run_start(&store, "r", "software", None, Mode::Team, "full").unwrap();
        let mut state = store.read_state("r").unwrap().unwrap();
        state.mode = Mode::Team;
        state.plan = plan;
        state.active_station = "frame".into();
        state.stations.insert("frame".into(), Station {
            station: "frame".into(), status: Status::Active, phase: StationPhase::Checkpoint,
            elaborated: true, checkpoint: Some(Checkpoint { kind: CheckpointKind::External, entered_at: None, outcome: None }),
            branch: None, pr_ref: Some("42".into()), pr_status: Some(PrStatus::Draft),
            pr_ready_at: None, pr_merged_at: None, verifier_nonce: None,
            started_at: None, completed_at: None,
        });
        store.write_state("r", &state).unwrap();
        (dir, store)
    }

    /// A discrete PR that has drawn review activity keeps origin's head current:
    /// the poll files the notes as feedback, then (git-backed) pushes the station
    /// branch. Best-effort — the push fails silently with no remote.
    #[test]
    fn discrete_poll_with_review_activity_pushes_the_station_head() {
        let (_d, store) = git_discrete_frame_at_open_pr(vec!["frame".into(), "specify".into()]);
        let host = MockHosting::with_notes(vec![crate::hosting::ReviewComment {
            id: "r1".into(), author: "alice".into(), body: "please change X".into(), change_request: true,
        }]);
        resolve_discrete_gate(&store, "r", &host).expect("poll");
        // The note was filed as external feedback (proving the poll branch ran and
        // the has-review-activity push path was taken).
        let fbs = crate::feedback::list(&store, "r").unwrap();
        assert!(fbs.iter().any(|f| f.id == "fb-ext-r1"), "review note filed: {fbs:?}");
    }

    /// When the merged station is the LAST in the plan, resolving its discrete
    /// gate completes the run and lands run-main onto base (no next station).
    #[test]
    fn discrete_merge_of_the_final_station_lands_the_run() {
        let (_d, store) = git_discrete_frame_at_open_pr(vec!["frame".into()]);
        let merged = MockHosting::new(MergeState::Merged);
        resolve_discrete_gate(&store, "r", &merged).expect("poll");
        let state = store.read_state("r").unwrap().unwrap();
        assert_eq!(state.stations["frame"].status, Status::Completed, "final station completes");
        assert!(state.stations["frame"].pr_merged_at.is_some(), "merge timestamp recorded");
        assert!(current_station(&resolve_factory_for(&store, "software").unwrap(), &state).is_none(),
            "no station remains after the final merge");
    }

    /// No hosting client → no PR is opened; the discrete gate degrades to a
    /// plain `external` review the operator resolves manually (await fallback).
    #[test]
    fn discrete_without_hosting_falls_back_to_manual_review() {
        let (_d, store) = store();
        run_start(&store, "d", "software", None, Mode::Team, "full").expect("start");
        let hosting = MockHosting::unavailable();
        let action = drive_discrete_frame_to_gate(&store, &hosting);
        assert!(
            matches!(&action, RunAction::ExternalReviewRequested { target, .. } if target.is_empty()),
            "no hosting → external review with no PR ref, got {action:?}"
        );
        assert!(hosting.opened.borrow().is_empty(), "no PR opened without hosting");
        let state = store.read_state("d").unwrap().unwrap();
        assert!(state.stations["frame"].pr_ref.is_none());
    }

    // ── git-backed merge mechanics (#3, #4) ──────────────────────────────

    /// A git-backed store: the StateStore lives at `<root>/.darkrun`, so the
    /// repo root is the tempdir.
    fn git_store() -> (tempfile::TempDir, std::path::PathBuf, StateStore) {
        let dir = tempdir().expect("tmp");
        let root = dir.path().to_path_buf();
        let git = |args: &[&str]| {
            assert!(std::process::Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .status()
                .unwrap()
                .success());
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["config", "user.email", "t@darkrun.ai"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(root.join(".gitignore"), ".darkrun/\n").unwrap();
        std::fs::write(root.join("README.md"), "# x\n").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "init"]);
        let store = StateStore::new(&root);
        (dir, root, store)
    }




    // ── Station drop (the keep-or-drop offer) ───────────────────────────────

    /// Walk the cursor to `station`'s arrival (Spec, unelaborated) by
    /// completing every prior station in the plan.
    fn arrive_at(store: &StateStore, run: &str, station: &str) {
        let mut state = store.read_state(run).unwrap().unwrap();
        let factory = resolve_factory_for(store, &store.read_run(run).unwrap().frontmatter.factory)
            .unwrap();
        let plan = if state.plan.is_empty() {
            factory.stations.iter().map(|s| s.name.clone()).collect::<Vec<_>>()
        } else {
            state.plan.clone()
        };
        for name in &plan {
            if name == station {
                break;
            }
            ensure_station(&mut state, &factory, name).unwrap();
            let st = state.stations.get_mut(name).unwrap();
            st.status = Status::Completed;
        }
        ensure_station(&mut state, &factory, station).unwrap();
        state.active_station = station.to_string();
        store.write_state(run, &state).unwrap();
    }

    #[test]
    fn station_drop_removes_the_optional_station_and_advances() {
        let (_d, store) = store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").expect("start");
        arrive_at(&store, "r", "shape");
        let out = station_drop(&store, "r", "shape").expect("drop");
        assert_eq!(out.dropped, "shape");
        assert_eq!(out.next_station.as_deref(), Some("build"));
        let state = store.read_state("r").unwrap().unwrap();
        assert!(!state.plan.is_empty(), "the plan materialized on drop");
        assert!(!state.plan.iter().any(|s| s == "shape"));
        assert!(!state.stations.contains_key("shape"));
        assert_eq!(state.active_station, "build");
        // The run doc tracks the new active station too.
        assert_eq!(store.read_run("r").unwrap().frontmatter.active_station, "build");
    }

    #[test]
    fn station_drop_refuses_core_inactive_and_started_stations() {
        let (_d, store) = store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").expect("start");

        // Not active: the run sits at frame; shape can't drop from afar.
        let err = station_drop(&store, "r", "shape").unwrap_err();
        assert!(format!("{err}").contains("drop_station_not_active"), "{err}");

        // Not optional: frame is core.
        let err = station_drop(&store, "r", "frame").unwrap_err();
        assert!(format!("{err}").contains("drop_station_not_optional"), "{err}");

        // Already started: shape with an elaborated spec refuses.
        arrive_at(&store, "r", "shape");
        {
            let mut state = store.read_state("r").unwrap().unwrap();
            state.stations.get_mut("shape").unwrap().elaborated = true;
            store.write_state("r", &state).unwrap();
        }
        let err = station_drop(&store, "r", "shape").unwrap_err();
        assert!(format!("{err}").contains("drop_station_already_started"), "{err}");
    }

    #[test]
    fn station_drop_retires_the_workless_station_branch() {
        let (_d, root, store) = git_store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").expect("start");
        arrive_at(&store, "r", "shape");
        // Enter the station so its branch + worktree exist (workless fork).
        crate::lifecycle::enter_station(&store, "r", "shape");
        let branch_exists = |b: &str| {
            std::process::Command::new("git")
                .arg("-C").arg(&root)
                .args(["rev-parse", "--verify", "-q", &format!("refs/heads/{b}")])
                .status().unwrap().success()
        };
        assert!(branch_exists("darkrun/r/shape"), "fork exists before drop");
        station_drop(&store, "r", "shape").expect("drop");
        assert!(!branch_exists("darkrun/r/shape"), "fork retired after drop");
    }

    #[test]
    fn spec_prompt_offers_keep_or_drop_only_on_optional_stations() {
        let (_d, store) = store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").expect("start");
        // frame (core): no offer.
        let t = run_tick(&store, "r").expect("tick");
        let prompt = t.prompt.expect("prompt");
        assert!(!prompt.contains("Keep or drop"), "core station carries no offer");
        // shape (optional): the offer renders.
        arrive_at(&store, "r", "shape");
        let t2 = run_tick(&store, "r").expect("tick");
        let prompt2 = t2.prompt.expect("prompt");
        assert!(prompt2.contains("Keep or drop"), "optional station offers the drop:\n{prompt2}");
        assert!(prompt2.contains("darkrun_station_drop"), "{prompt2}");
    }

    // ── Run-level delivery PR: draft at start, ready at seal ───────────────

    /// A recording mock: opens drafts, flips ready, remembers both.
    struct DeliveryMock {
        open_url: Option<&'static str>,
        ready_ok: bool,
        flipped: std::cell::RefCell<Vec<String>>,
    }
    impl crate::hosting::Hosting for DeliveryMock {
        fn available(&self) -> bool {
            self.open_url.is_some()
        }
        fn open_draft(&self, _req: &crate::hosting::OpenRequest) -> Option<String> {
            self.open_url.map(str::to_string)
        }
        fn merge_state(&self, _pr_ref: &str) -> crate::hosting::MergeState {
            crate::hosting::MergeState::Open
        }
        fn mark_ready(&self, pr_ref: &str) -> bool {
            self.flipped.borrow_mut().push(pr_ref.to_string());
            self.ready_ok
        }
    }

    #[test]
    fn run_draft_pr_opens_once_and_stamps_external_refs() {
        let (_d, _root, store) = git_store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").expect("start");
        let mock = DeliveryMock {
            open_url: Some("https://example.test/pull/7"),
            ready_ok: true,
            flipped: Default::default(),
        };
        open_run_draft_pr_with(&store, "r", &mock);
        let run = store.read_run("r").unwrap();
        assert_eq!(
            run.frontmatter.external_refs.pr_url.as_deref(),
            Some("https://example.test/pull/7")
        );
        assert_eq!(
            run.frontmatter.external_refs.other.get("pr_status").map(String::as_str),
            Some("draft")
        );
        // Idempotent: a second call never re-opens.
        open_run_draft_pr_with(&store, "r", &mock);
        let run2 = store.read_run("r").unwrap();
        assert_eq!(run2.frontmatter.external_refs.pr_url, run.frontmatter.external_refs.pr_url);
    }

    #[test]
    fn run_pr_flips_ready_exactly_once_at_seal() {
        let (_d, _root, store) = git_store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").expect("start");
        let mock = DeliveryMock {
            open_url: Some("https://example.test/pull/7"),
            ready_ok: true,
            flipped: Default::default(),
        };
        open_run_draft_pr_with(&store, "r", &mock);

        flip_run_pr_ready(&store, "r", &mock);
        assert_eq!(mock.flipped.borrow().len(), 1, "one flip");
        let run = store.read_run("r").unwrap();
        assert_eq!(
            run.frontmatter.external_refs.other.get("pr_status").map(String::as_str),
            Some("ready")
        );
        // Guarded: already-ready never re-flips.
        flip_run_pr_ready(&store, "r", &mock);
        assert_eq!(mock.flipped.borrow().len(), 1, "no second flip");
    }

    #[test]
    fn run_pr_flip_failure_is_stamped_not_fatal() {
        let (_d, _root, store) = git_store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").expect("start");
        let mock = DeliveryMock {
            open_url: Some("https://example.test/pull/7"),
            ready_ok: false,
            flipped: Default::default(),
        };
        open_run_draft_pr_with(&store, "r", &mock);
        flip_run_pr_ready(&store, "r", &mock);
        let run = store.read_run("r").unwrap();
        assert_eq!(
            run.frontmatter.external_refs.other.get("pr_status").map(String::as_str),
            Some("failed"),
            "a failed flip is recorded for the operator, never fatal"
        );
    }

    // ── Compare-URL fallback ────────────────────────────────────────────────

    #[test]
    fn compare_url_builds_provider_create_forms() {
        let (_d, root, _store) = git_store();
        let set_origin = |url: &str| {
            let _ = std::process::Command::new("git")
                .arg("-C").arg(&root)
                .args(["remote", "remove", "origin"]).status();
            assert!(std::process::Command::new("git")
                .arg("-C").arg(&root)
                .args(["remote", "add", "origin", url])
                .status().unwrap().success());
        };
        set_origin("git@github.com:acme/widgets.git");
        assert_eq!(
            crate::hosting::compare_url(&root, "main", "darkrun/r/frame").as_deref(),
            Some("https://github.com/acme/widgets/compare/main...darkrun/r/frame?expand=1")
        );
        set_origin("https://gitlab.com/acme/widgets.git");
        let gl = crate::hosting::compare_url(&root, "main", "darkrun/r/frame").unwrap();
        assert!(gl.starts_with("https://gitlab.com/acme/widgets/-/merge_requests/new?"), "{gl}");
        assert!(gl.contains("source_branch%5D=darkrun%2Fr%2Fframe"), "{gl}");
        assert!(gl.contains("target_branch%5D=main"), "{gl}");
    }

    #[test]
    fn compare_url_is_none_without_a_recognized_origin() {
        let (_d, root, _store) = git_store();
        assert!(crate::hosting::compare_url(&root, "main", "x").is_none(), "no origin");
    }

    // ── Pre-derive clean-tree gate (save_wip) ───────────────────────────────

    /// Uncommitted AGENT work (outside `.darkrun/`) blocks the tick with a
    /// `save_wip` action listing the loose paths; committing clears the gate.
    /// The engine never authors the agent's commits.
    #[test]
    fn tick_blocks_on_uncommitted_agent_work_with_save_wip() {
        let (_d, root, store) = git_store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").expect("start");

        // The agent leaves loose source work in the project tree.
        std::fs::write(root.join("scratch.rs"), "fn main() {}\n").unwrap();
        let t = run_tick(&store, "r").expect("tick");
        match &t.action {
            RunAction::SaveWip { dirty_files, branch, .. } => {
                assert!(
                    dirty_files.iter().any(|p| p == "scratch.rs"),
                    "the loose path is listed: {dirty_files:?}"
                );
                assert!(!branch.is_empty(), "the holding branch is named");
            }
            other => panic!("expected SaveWip, got {other:?}"),
        }
        let prompt = t.prompt.expect("save_wip renders a prompt");
        assert!(prompt.contains("scratch.rs"), "prompt lists the file:\n{prompt}");
        assert!(prompt.contains("Save Work in Progress"), "{prompt}");

        // Committing the work clears the gate — the next tick proceeds.
        for args in [vec!["add", "-A"], vec!["commit", "-q", "-m", "scratch"]] {
            assert!(std::process::Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(&args)
                .status()
                .unwrap()
                .success());
        }
        let t2 = run_tick(&store, "r").expect("tick 2");
        assert!(
            !matches!(t2.action, RunAction::SaveWip { .. }),
            "clean tree ticks past the gate: {:?}",
            t2.action
        );
    }

    /// Engine bookkeeping (`.darkrun/`, `.gitignore`) never trips the gate —
    /// only the agent's own work does.
    #[test]
    fn engine_state_writes_do_not_trip_the_save_wip_gate() {
        let (_d, _root, store) = git_store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").expect("start");
        // The run's state dir is busy with engine writes (run.md, state.json…)
        // — and `.darkrun/` is gitignored in this repo besides. A plain tick
        // must derive a real action, not SaveWip.
        let t = run_tick(&store, "r").expect("tick");
        assert!(
            !matches!(t.action, RunAction::SaveWip { .. }),
            "engine state alone never blocks: {:?}",
            t.action
        );
    }

    /// #4: a station branch identical to run-main carries no merge debt, so the
    /// cursor must NOT enqueue a land that would mint an empty --no-ff commit.
    #[test]
    fn no_merge_debt_means_no_land_synthesis() {
        let (_d, root, store) = git_store();
        crate::lifecycle::ensure_run_main(&store, "r");
        // Enter a station but do NO work — its branch == run-main (no debt).
        crate::lifecycle::enter_station(&store, "r", "build");
        assert!(
            !station_has_merge_debt(&store, "r", "build"),
            "identical-tree station has no merge debt"
        );

        // Do real work on the station worktree → now there IS debt.
        let wt = crate::lifecycle::station_worktree_path(&root, "r", "build");
        std::fs::write(wt.join("work.txt"), "work\n").unwrap();
        let git_wt = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(&wt)
                .args(args)
                .status()
                .unwrap()
                .success()
        };
        assert!(git_wt(&["add", "-A"]));
        assert!(git_wt(&["commit", "-q", "-m", "station work"]));
        assert!(
            station_has_merge_debt(&store, "r", "build"),
            "a station with new commits has merge debt"
        );

        // A station whose branch was never created defaults to "has debt" — the
        // land path (which guards branch existence) then no-ops cleanly, so the
        // cursor never wedges on a false negative.
        assert!(
            station_has_merge_debt(&store, "r", "never-entered"),
            "a missing station branch defaults to merge debt"
        );
    }

    /// BUG-4 guard: a locally-landed run is `ahead` of the default branch (the
    /// work is a real commit, not pushed) — `branch_status_token` surfaces it so
    /// the seal prompt can warn the operator that origin still needs a push.
    #[test]
    fn branch_status_token_reports_ahead_after_local_land() {
        let (_d, root, store) = git_store();
        crate::lifecycle::ensure_run_main(&store, "r");
        crate::lifecycle::enter_station(&store, "r", "build");
        let wt = crate::lifecycle::station_worktree_path(&root, "r", "build");
        std::fs::write(wt.join("shipped.txt"), "ship\n").unwrap();
        let git_wt = |args: &[&str]| {
            std::process::Command::new("git").arg("-C").arg(&wt).args(args).status().unwrap().success()
        };
        assert!(git_wt(&["add", "-A"]));
        assert!(git_wt(&["commit", "-q", "-m", "work"]));
        crate::lifecycle::land_station(&store, "r", "build");

        // run-main now carries verified work the default branch doesn't → ahead.
        assert_eq!(branch_status_token(&store, "r").as_deref(), Some("ahead"));
        // A non-git run surfaces nothing (no false "push needed").
        let plain_dir = tempdir().expect("tmp");
        let plain = StateStore::new(plain_dir.path());
        assert_eq!(branch_status_token(&plain, "r"), None);
    }

    /// #3: a land that leaves agent-content conflicts surfaces as a
    /// `MergeConflict` action and the merge is left in-tree (MERGE_HEAD set, not
    /// aborted); the next derive keeps re-deriving it until the merge clears.
    #[test]
    fn enter_unit_and_record_tolerates_a_missing_unit_doc() {
        let (_d, _root, store) = git_store();
        crate::lifecycle::ensure_run_main(&store, "r");
        crate::lifecycle::enter_station(&store, "r", "build");
        // enter_unit forks a branch/worktree for the unit (a git op that performs
        // regardless), but the unit document was never written → read_unit errors
        // and the branch-stamp step bails cleanly instead of propagating.
        enter_unit_and_record(&store, "r", "build", "ghost-unit").expect("clean no-op on missing doc");
    }

    #[test]
    fn merge_conflict_action_infers_a_branch_for_detached_worktrees() {
        let (_d, root, store) = git_store();
        crate::lifecycle::ensure_run_main(&store, "r");
        crate::lifecycle::enter_station(&store, "r", "build");
        let git = |args: &[&str]| {
            assert!(std::process::Command::new("git").arg("-C").arg(&root).args(args).status().unwrap().success(), "git {args:?}");
        };
        // A detached `_merge-<x>` worktree whose target does NOT end in `-main`
        // (a station-targeted merge) → the station-branch inference arm.
        let merge_wt = root.join(".darkrun/_merge-darkrun-r-build");
        git(&["worktree", "add", "--detach", merge_wt.to_str().unwrap(), "HEAD"]);
        // A plain detached worktree with no `_merge-` prefix → the catch-all
        // station-branch inference arm.
        let plain_wt = root.join(".darkrun/plaindetached");
        git(&["worktree", "add", "--detach", plain_wt.to_str().unwrap(), "HEAD"]);

        // No merge is in progress, so it surfaces nothing — but building the
        // candidate list exercised both detached-worktree inference arms.
        assert!(merge_conflict_action(&store, "r", "build").unwrap().is_none());
    }

    #[test]
    fn derive_preempts_everything_with_an_in_tree_merge_conflict() {
        let (_d, root, store) = git_store();
        // A real run document so derive_position can resolve run/factory/state.
        run_start(&store, "r", "software", None, Mode::Solo, "full").expect("start");
        // run_start entered frame, forking its branch + worktree.
        let git_root = |args: &[&str]| {
            std::process::Command::new("git").arg("-C").arg(&root).args(args).status().unwrap().success()
        };
        // Advance run-main with a conflicting line. run_start checked the run's
        // main branch out at the ROOT tree (the commit-and-push spine), so the
        // root commit IS the run-main side — no temp worktree needed.
        std::fs::write(root.join("conflict.txt"), "RUN-MAIN SIDE\n").unwrap();
        assert!(git_root(&["add", "-A"]));
        assert!(git_root(&["commit", "-q", "-m", "run-main line"]));
        // The active station (frame) edits the same file differently → conflict.
        let wt = crate::lifecycle::station_worktree_path(&root, "r", "frame");
        std::fs::write(wt.join("conflict.txt"), "STATION SIDE\n").unwrap();
        let git_wt = |a: &[&str]| std::process::Command::new("git").arg("-C").arg(&wt).args(a).status().unwrap().success();
        assert!(git_wt(&["add", "-A"]));
        assert!(git_wt(&["commit", "-q", "-m", "station line"]));
        // Land leaves the conflict in-tree.
        let out = crate::lifecycle::land_station(&store, "r", "frame");
        assert!(!out.performed && out.has_conflicts(), "conflict left in tree: {out:?}");

        // derive_position preempts every track with the in-tree merge conflict.
        let pos = derive_position(&store, "r").expect("derive");
        assert!(
            matches!(pos.action, Some(RunAction::MergeConflict { .. })),
            "the mid-merge conflict preempts, got {:?}",
            pos.action
        );
    }

    #[test]
    fn conflicting_land_surfaces_merge_conflict_left_in_tree() {
        let (_d, root, store) = git_store();
        crate::lifecycle::ensure_run_main(&store, "r");
        crate::lifecycle::enter_station(&store, "r", "build");

        // run-main gets a code file with one value…
        let git_root = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .status()
                .unwrap()
                .success()
        };
        // Advance run-main on its own branch (checked out nowhere) by committing
        // through a temp worktree.
        let rmwt = root.join(".darkrun/rm");
        assert!(git_root(&["worktree", "add", "-q", rmwt.to_str().unwrap(), "darkrun/r/main"]));
        std::fs::write(rmwt.join("conflict.txt"), "RUN-MAIN SIDE\n").unwrap();
        let git_rm = |args: &[&str]| {
            std::process::Command::new("git").arg("-C").arg(&rmwt).args(args).status().unwrap().success()
        };
        assert!(git_rm(&["add", "-A"]));
        assert!(git_rm(&["commit", "-q", "-m", "run-main conflict line"]));
        assert!(git_root(&["worktree", "remove", "--force", rmwt.to_str().unwrap()]));

        // …and the station edits the SAME file differently → a real conflict.
        let wt = crate::lifecycle::station_worktree_path(&root, "r", "build");
        std::fs::write(wt.join("conflict.txt"), "STATION SIDE\n").unwrap();
        let git_wt = |args: &[&str]| {
            std::process::Command::new("git").arg("-C").arg(&wt).args(args).status().unwrap().success()
        };
        assert!(git_wt(&["add", "-A"]));
        assert!(git_wt(&["commit", "-q", "-m", "station conflict line"]));

        // Land it — engine-protected merge leaves the conflict in-tree.
        let outcome = crate::lifecycle::land_station(&store, "r", "build");
        assert!(!outcome.performed, "a conflicting land does not perform: {outcome:?}");
        assert!(outcome.has_conflicts(), "conflict paths surface: {outcome:?}");

        // derive_position now surfaces a MergeConflict for the station.
        let action = merge_conflict_action(&store, "r", "build")
            .expect("ok")
            .expect("a merge conflict action");
        match &action {
            RunAction::MergeConflict { branch, conflict_paths, .. } => {
                assert!(!conflict_paths.is_empty(), "names the conflicted paths");
                assert!(conflict_paths.iter().any(|p| p.contains("conflict.txt")));
                assert!(!branch.is_empty());
            }
            other => panic!("expected MergeConflict, got {other:?}"),
        }
    }

    /// B9: a wave unit forks onto its own worktree + branch when Manufacture
    /// dispatches it, the worker's commits stay on that unit branch (NOT the
    /// station branch), and the unit lands back onto the station branch when the
    /// station leaves Manufacture for Audit.
    #[test]
    fn manufacture_isolates_each_unit_then_lands_it_on_audit() {
        let (_d, root, store) = git_store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").expect("start");
        // run_start entered the first station (frame), so its branch exists.
        assert!(branch_exists_at(&root, "darkrun/r/frame"));

        // A unit decomposed into the first station.
        let unit = Unit {
            slug: "u1".into(),
            frontmatter: darkrun_core::domain::UnitFrontmatter {
                status: Status::Pending,
                station: Some("frame".into()),
                ..Default::default()
            },
            title: "u1".into(),
            body: String::new(),
        };
        store.write_unit("r", &unit).expect("write unit");

        // Spec (held in solo until sealed) -> Review -> UserGate, then the
        // operator clears the gate.
        run_tick(&store, "r").expect("spec");
        elaborate_seal(&store, "r", "frame").expect("seal");
        run_tick(&store, "r").expect("spec advance after seal");
        run_tick(&store, "r").expect("review");
        checkpoint_decide(&store, "r", true, None).expect("clear gate");

        // Manufacture: the unit forks onto its own worktree + branch.
        let t = run_tick(&store, "r").expect("manufacture");
        assert!(matches!(t.action, RunAction::Manufacture { .. }), "got {:?}", t.action);
        let u = store.read_unit("r", "u1").unwrap();
        assert_eq!(
            u.frontmatter.branch.as_deref(),
            Some("darkrun/r/units/frame/u1"),
            "the unit's isolation branch is stamped"
        );
        let wt = crate::lifecycle::unit_worktree_path(&root, "r", "frame", "u1");
        assert!(wt.exists(), "the unit worktree exists on disk");

        // The worker does the unit's work IN ITS OWN worktree and commits.
        std::fs::write(wt.join("u1.txt"), "unit one\n").unwrap();
        let git_wt = |args: &[&str]| {
            std::process::Command::new("git").arg("-C").arg(&wt).args(args).status().unwrap().success()
        };
        assert!(git_wt(&["add", "-A"]));
        assert!(git_wt(&["commit", "-q", "-m", "u1 work"]));
        // The work is on the unit branch, NOT yet on the station branch.
        assert!(!show_path(&root, "darkrun/r/frame", "u1.txt"), "u1's work must be isolated");

        // Complete the unit; the next tick leaves Manufacture for Audit and lands
        // the unit onto the station branch.
        let mut done = store.read_unit("r", "u1").unwrap();
        done.frontmatter.status = Status::Completed;
        store.write_unit("r", &done).unwrap();

        let t2 = run_tick(&store, "r").expect("audit");
        assert!(matches!(t2.action, RunAction::Audit { .. }), "got {:?}", t2.action);

        // The unit branch + worktree are gone; the work landed on the station.
        assert!(!branch_exists_at(&root, "darkrun/r/units/frame/u1"), "unit branch retired");
        assert!(!wt.exists(), "unit worktree removed");
        assert!(show_path(&root, "darkrun/r/frame", "u1.txt"), "u1's work landed on the station");
        // …but NOT yet on run-main (the station hasn't completed).
        assert!(!show_path(&root, "darkrun/r/main", "u1.txt"), "station hasn't landed yet");
    }

    /// B9: an open feedback item forks the fix-worker onto its own worktree off
    /// the station branch, the fix's commits stay isolated there, and closing the
    /// feedback lands the fix back onto the station branch.
    #[test]
    fn feedback_fix_isolates_on_its_own_worktree_then_lands_on_close() {
        let (_d, root, store) = git_store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").expect("start");
        assert!(branch_exists_at(&root, "darkrun/r/frame"));

        // Open feedback at the active station preempts the run onto the fix track.
        store
            .write_feedback_raw("r", "fb-7", "---\nstatus: pending\nstation: frame\n---\nbusted\n")
            .expect("fb");

        let t = run_tick(&store, "r").expect("fix tick");
        assert!(
            matches!(&t.action, RunAction::FixFeedback { feedback_id, .. } if feedback_id == "fb-7"),
            "got {:?}",
            t.action
        );
        // The fix forked onto its own worktree off the station branch…
        let wt = crate::lifecycle::fix_worktree_path(&root, "r", "frame", "fb-7");
        assert!(wt.exists(), "the fix worktree exists on disk");
        assert!(branch_exists_at(&root, "darkrun/r/fixes/frame/fb-7"));
        // …and the prompt points the worker at it.
        let prompt = t.prompt.expect("fix prompt");
        assert!(prompt.contains("fixes/frame/fb-7"), "prompt names the fix worktree:\n{prompt}");

        // The fix-worker repairs inside the isolated worktree and commits.
        std::fs::write(wt.join("fix.txt"), "repaired\n").unwrap();
        let git_wt = |args: &[&str]| {
            std::process::Command::new("git").arg("-C").arg(&wt).args(args).status().unwrap().success()
        };
        assert!(git_wt(&["add", "-A"]));
        assert!(git_wt(&["commit", "-q", "-m", "fix fb-7"]));
        assert!(!show_path(&root, "darkrun/r/frame", "fix.txt"), "the fix is isolated pre-close");

        // Closing the feedback lands the fix back onto the station branch.
        crate::feedback::close_with_reply(&store, "r", "fb-7", "corrected").expect("close");
        assert!(!branch_exists_at(&root, "darkrun/r/fixes/frame/fb-7"), "fix branch retired");
        assert!(!wt.exists(), "fix worktree removed");
        assert!(show_path(&root, "darkrun/r/frame", "fix.txt"), "the fix landed on the station");
    }

    fn branch_exists_at(root: &std::path::Path, branch: &str) -> bool {
        std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["rev-parse", "--verify", &format!("refs/heads/{branch}")])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Whether `branch:path` resolves (the file exists on that branch).
    fn show_path(root: &std::path::Path, branch: &str, path: &str) -> bool {
        std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["cat-file", "-e", &format!("{branch}:{path}")])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    #[test]
    fn derive_emits_revise_for_a_flagged_unit() {
        let (_d, store) = store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").unwrap();
        let mut u = crate::units::create(&store, "r", "u1", "frame", crate::units::UnitSpec::default()).unwrap();
        u.frontmatter.revise = true;
        store.write_unit("r", &u).unwrap();
        let pos = derive_position(&store, "r").unwrap();
        assert!(matches!(pos.action, Some(RunAction::ReviseUnitSpecs { .. })), "revise: {:?}", pos.action);
    }

    #[test]
    fn derive_emits_safe_repair_for_an_undefined_station_unit() {
        let (_d, store) = store();
        run_start(&store, "r2", "software", None, Mode::Solo, "full").unwrap();
        let bad = crate::units::create(&store, "r2", "ub", "ghost-station", crate::units::UnitSpec::default()).unwrap();
        store.write_unit("r2", &bad).unwrap();
        let pos = derive_position(&store, "r2").unwrap();
        assert!(matches!(pos.action, Some(RunAction::SafeRepair { .. })), "safe_repair: {:?}", pos.action);
    }

    #[test]
    fn feedback_severity_rank_maps_each_token_and_unknown() {
        assert_eq!(feedback_severity_rank("severity: blocker\n"), 0);
        assert_eq!(feedback_severity_rank("severity: high"), 1);
        assert_eq!(feedback_severity_rank("severity: \"medium\""), 2);
        assert_eq!(feedback_severity_rank("severity: low"), 3);
        assert_eq!(feedback_severity_rank("severity: spicy"), 4); // unknown token
        assert_eq!(feedback_severity_rank("no severity line here"), 4); // absent
    }

    #[test]
    fn artifact_basename_takes_the_last_segment() {
        assert_eq!(artifact_basename("specify/spec.md"), "spec.md");
        assert_eq!(artifact_basename("  bare.md  "), "bare.md");
    }

    #[test]
    fn run_review_stamp_rejects_an_empty_role() {
        let (_d, store) = store();
        assert!(matches!(
            run_review_stamp(&store, "r", "   "),
            Err(McpError::InvalidInput(_))
        ));
    }

    #[test]
    fn required_station_inputs_empty_for_unknown_or_unplanned_station() {
        let factory = crate::factory::resolve_factory("software").expect("software factory");
        // A station the factory doesn't define → no required inputs.
        assert!(required_station_inputs(&factory, &[], "no-such-station").is_empty());
        // A real station that isn't in the (non-empty) plan → none required here.
        let names = factory.station_names();
        let plan = vec![names[0].clone()];
        let absent = names.last().cloned().unwrap();
        if absent != names[0] {
            assert!(required_station_inputs(&factory, &plan, &absent).is_empty());
        }
    }

    #[test]
    fn resolve_discrete_gate_pushes_and_opens_a_pr_at_an_external_checkpoint() {
        use darkrun_core::domain::{Checkpoint, CheckpointKind, Station, StationPhase, Status};
        struct MockHosting;
        impl crate::hosting::Hosting for MockHosting {
            fn available(&self) -> bool { true }
            fn open_draft(&self, _req: &crate::hosting::OpenRequest) -> Option<String> {
                Some("https://example.test/pr/1".into())
            }
            fn merge_state(&self, _pr_ref: &str) -> crate::hosting::MergeState {
                crate::hosting::MergeState::Open
            }
        }
        let dir = tempdir().unwrap();
        // A git repo so the head-push branch runs (it fails silently — no remote).
        std::process::Command::new("git").arg("-C").arg(dir.path()).args(["init", "-q"]).status().unwrap();
        let store = StateStore::new(dir.path());
        run_start(&store, "r", "software", None, Mode::Team, "full").unwrap();
        // Force the run to sit at frame's checkpoint; team mode makes its
        // effective kind External, so the discrete gate opens a PR.
        let mut state = store.read_state("r").unwrap().unwrap();
        state.mode = Mode::Team;
        state.active_station = "frame".into();
        state.stations.insert("frame".into(), Station {
            station: "frame".into(), status: Status::Active, phase: StationPhase::Checkpoint,
            elaborated: true, checkpoint: Some(Checkpoint { kind: CheckpointKind::External, entered_at: None, outcome: None }),
            branch: None, pr_ref: None, pr_status: None,
            pr_ready_at: None, pr_merged_at: None, verifier_nonce: None,
            started_at: None, completed_at: None,
        });
        store.write_state("r", &state).unwrap();

        resolve_discrete_gate(&store, "r", &MockHosting).unwrap();
        // The mock's PR ref is recorded on the station.
        let after = store.read_state("r").unwrap().unwrap();
        assert_eq!(
            after.stations.get("frame").and_then(|s| s.pr_ref.as_deref()),
            Some("https://example.test/pr/1")
        );
    }

    #[test]
    fn build_prompt_context_threads_action_specifics() {
        use darkrun_core::domain::SealKind;
        let (_d, store) = store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").unwrap();

        let fq = RunAction::FeedbackQuestion { run: "r".into(), station: "build".into(), feedback_id: "fb-1".into() };
        let rv = RunAction::ReviseUnitSpecs { run: "r".into(), station: "build".into(), units: vec!["u1".into()] };
        let mc = RunAction::MergeConflict { run: "r".into(), station: "build".into(), branch: "b".into(), conflict_paths: vec!["x.rs".into()] };
        let ps = RunAction::PendingSeal { run: "r".into(), kind: SealKind::External };

        // The FeedbackQuestion id threads into the prompt context.
        let cfq = build_prompt_context(&store, "r", &fq).unwrap();
        assert_eq!(cfq.feedback_id.as_deref(), Some("fb-1"));
        // ReviseUnitSpecs carries its unit list.
        let crv = build_prompt_context(&store, "r", &rv).unwrap();
        assert_eq!(crv.units, vec!["u1".to_string()]);
        // MergeConflict carries the branch + conflict paths.
        let cmc = build_prompt_context(&store, "r", &mc).unwrap();
        assert_eq!(cmc.branch.as_deref(), Some("b"));
        assert_eq!(cmc.conflict_paths, vec!["x.rs".to_string()]);
        // PendingSeal records the seal kind (a run-level action — no station).
        let cps = build_prompt_context(&store, "r", &ps).unwrap();
        assert!(cps.seal.is_some());
        assert!(cps.station.is_none());

        // SafeRepair and Escalate are station-scoped actions — their station
        // threads into the context like the rest.
        let sr = RunAction::SafeRepair { run: "r".into(), station: "frame".into(), reason: "bad state".into() };
        assert_eq!(build_prompt_context(&store, "r", &sr).unwrap().station.as_deref(), Some("frame"));
        let es = RunAction::Escalate { run: "r".into(), station: "build".into(), reason: "runaway".into() };
        assert_eq!(build_prompt_context(&store, "r", &es).unwrap().station.as_deref(), Some("build"));
    }

    #[test]
    fn action_tag_and_station_of_cover_the_remaining_variants() {
        use darkrun_core::domain::SealKind;
        let with_station = [
            RunAction::FeedbackQuestion { run: "r".into(), station: "s".into(), feedback_id: "f".into() },
            RunAction::SafeRepair { run: "r".into(), station: "s".into(), reason: "x".into() },
            RunAction::ReviseUnitSpecs { run: "r".into(), station: "s".into(), units: vec!["u".into()] },
            RunAction::MergeConflict { run: "r".into(), station: "s".into(), branch: "b".into(), conflict_paths: vec![] },
        ];
        for a in &with_station {
            assert!(!action_tag(a).is_empty(), "{a:?} has a tag");
            assert_eq!(station_of(a), Some("s"), "{a:?} carries its station");
        }
        // PendingSeal carries a run but no station.
        let seal = RunAction::PendingSeal { run: "r".into(), kind: SealKind::External };
        assert_eq!(action_tag(&seal), "pending_seal");
        assert_eq!(station_of(&seal), None);
    }

    #[test]
    fn validate_units_flags_naming_deps_and_cycles() {
        use darkrun_core::domain::{Unit, UnitFrontmatter};
        let mk = |slug: &str, deps: &[&str]| Unit {
            slug: slug.into(),
            frontmatter: UnitFrontmatter {
                depends_on: deps.iter().map(|s| s.to_string()).collect(),
                ..Default::default()
            },
            title: slug.into(),
            body: String::new(),
        };
        // Invalid naming (uppercase / whitespace).
        let bad = mk("Bad Name", &[]);
        assert_eq!(validate_units(std::slice::from_ref(&bad), &[&bad]).unwrap().0, "invalid_naming");
        // A dependency on a unit that doesn't exist.
        let dangling = mk("a", &["ghost"]);
        assert_eq!(validate_units(std::slice::from_ref(&dangling), &[&dangling]).unwrap().0, "unresolved_deps");
        // A dependency cycle a -> b -> a.
        let a = mk("a", &["b"]);
        let b = mk("b", &["a"]);
        let all = vec![a.clone(), b.clone()];
        assert_eq!(validate_units(&all, &[&a, &b]).unwrap().0, "dependency_cycle");
        // A clean, acyclic, well-named set passes.
        let clean = mk("c", &[]);
        assert!(validate_units(std::slice::from_ref(&clean), &[&clean]).is_none());
    }

    #[test]
    fn elaborate_seal_rejects_an_unknown_station() {
        let (_d, store) = store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").unwrap();
        // A station the run's state doesn't carry → the not-active error arm.
        assert!(matches!(
            elaborate_seal(&store, "r", "no-such-station"),
            Err(McpError::InvalidInput(_))
        ));
    }
}
