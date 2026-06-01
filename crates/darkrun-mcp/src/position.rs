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
    Checkpoint, CheckpointKind, CheckpointOutcome, Run, RunFrontmatter, SealKind, Station,
    StationPhase, Status, Unit,
};
use darkrun_core::{RunState, StateStore};
use serde::Serialize;

use crate::error::{McpError, Result};
use crate::factory::{resolve_factory, FactoryDef};

/// Which of the three tracks produced the current action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Track {
    /// Witnessed artifact drift.
    Drift,
    /// Open feedback.
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
    /// Reconcile a witnessed drift event (Track C).
    ResolveDrift {
        run: String,
        station: String,
        path: String,
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
    /// Every station is locked but the run declares a final `seal:` gate — hold
    /// for an external merge / await decision before sealing. Parity for
    /// the predecessor's `pending_seal` / `intent_approved`.
    PendingSeal {
        run: String,
        kind: SealKind,
    },
    /// Every station is locked and the run is sealed.
    Sealed { run: String },
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
#[derive(Debug, Clone, Serialize, Default)]
pub struct PromptContext {
    /// The run slug.
    pub run: String,
    /// The active station name (absent for run-level actions like `sealed`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub station: Option<String>,
    /// The station phase tag, when the action sits on the phase machine.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    /// What the active station eliminates.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kills: Option<String>,
    /// The checkpoint gate kind (`auto`/`ask`/`external`/`await`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<CheckpointKind>,
    /// The durable artifact the station locks on completion.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locked_artifact: Option<String>,
    /// The worker beat to dispatch this manufacture tick.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker: Option<String>,
    /// The open feedback id, for the fix track.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feedback_id: Option<String>,
    /// The drifted artifact path, for the drift track.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// What's structurally wrong, for `UnitsInvalid` (`invalid_naming` etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
    /// The external review target, for `ExternalReviewRequested`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// A human-readable reason, for `Escalate` / `SafeRepair`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The run-level seal gate (`external` / `await`), for `PendingSeal`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seal: Option<String>,
    /// A free-form message (mid-wave noop).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// The station's Workers, in Pass-loop order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub workers: Vec<String>,
    /// The station's Reviewers.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reviewers: Vec<String>,
    /// The wave-ready / on-record unit slugs for this action.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub units: Vec<String>,
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
}

/// Whether a unit is "past" — no further cursor work needed at its position.
fn unit_complete(unit: &Unit) -> bool {
    matches!(unit.status(), Status::Completed)
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

    // 3. Dependency cycle among the station's units. Catches `dag_cycle_detected`.
    if let Some(cycle) = first_cycle(su) {
        return Some(("dependency_cycle".to_string(), cycle));
    }
    None
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

/// Resolve a run-sizing `mode` into `(station_plan, auto_gates)` — the
/// right-sizing pass at run start.
///
/// The plan is the factory's stations filtered to those the mode keeps, in
/// factory order. `full`/`standard`/`continuous`/unknown → the full plan (empty
/// sentinel) with the factory's own gates. A mode whose kept stations don't
/// exist in the factory falls back to the full plan, so right-sizing can never
/// strand a run with no stations. Right-sized modes run with `auto` gates.
fn resolve_template(mode: &str, factory: &FactoryDef) -> (Vec<String>, bool) {
    let keep: &[&str] = match mode.trim().to_ascii_lowercase().as_str() {
        // Small work: build + prove only — skip framing/design and hardening.
        "quick" => &["build", "prove"],
        // A localized fix: keep the spec for the regression, build, prove.
        "bugfix" => &["specify", "build", "prove"],
        // Structural change: keep the design pressure-test, build, prove.
        "refactor" => &["shape", "build", "prove"],
        // Full traversal with the factory's own gates.
        _ => return (Vec::new(), false),
    };
    let plan: Vec<String> = factory
        .stations
        .iter()
        .map(|s| s.name.clone())
        .filter(|name| keep.contains(&name.as_str()))
        .collect();
    if plan.is_empty() {
        (Vec::new(), false)
    } else {
        (plan, true)
    }
}

/// Track B — feedback. Returns a `FixFeedback` action for the first open
/// feedback item, or `None` when no open feedback exists. Feedback is "open"
/// when its `status:` frontmatter line is not a terminal value.
fn walk_feedback(store: &StateStore, slug: &str, station: &str) -> Result<Option<RunAction>> {
    let raw = store.read_feedback_raw(slug)?;
    for (id, content) in raw {
        if feedback_open(&content) {
            // A feedback item that is a *question* needs a user decision, not a
            // code fix — route it to the question half of the track.
            let action = if feedback_is_question(&content) {
                RunAction::FeedbackQuestion {
                    run: slug.to_string(),
                    station: station.to_string(),
                    feedback_id: id,
                }
            } else {
                RunAction::FixFeedback {
                    run: slug.to_string(),
                    station: station.to_string(),
                    feedback_id: id,
                }
            };
            return Ok(Some(action));
        }
    }
    Ok(None)
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
    let factory = resolve_factory(&run.frontmatter.factory)
        .ok_or_else(|| McpError::UnknownFactory(run.frontmatter.factory.clone()))?;
    let state = store.read_state(slug)?.unwrap_or_default();
    let units = store.read_units(slug)?;

    let station = match current_station(&factory, &state) {
        Some(s) => s,
        None => {
            // Every station locked. If the run declares a final `seal:` gate
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

    // ── Track C: drift ───────────────────────────────────────────────────
    // Witnessed artifact drift preempts everything. The sweep that deposits
    // drift entries is a future darkrun-core concern (see `deferred`); until
    // it lands this reads any entries an external sweep left under
    // `.darkrun/<run>/drift/`. With none, the track is a no-op.
    if let Some(entry) = crate::drift::first(store, slug)? {
        let drift_station = if entry.station.is_empty() {
            station.clone()
        } else {
            entry.station.clone()
        };
        return Ok(Position {
            track: Track::Drift,
            action: Some(RunAction::ResolveDrift {
                run: slug.to_string(),
                station: drift_station,
                path: entry.path,
            }),
        });
    }

    // ── Track B: feedback ────────────────────────────────────────────────
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
    }

    // Runaway Pass loop: a unit past its iteration budget escalates instead of
    // looping forever.
    if let Some(u) = su_all.iter().find(|u| u.frontmatter.pass > MAX_PASSES) {
        return Ok(Position {
            track: Track::Run,
            action: Some(RunAction::Escalate {
                run: slug.to_string(),
                station: station.clone(),
                reason: format!(
                    "unit `{}` has run {} passes (budget {MAX_PASSES}) — escalating",
                    u.slug, u.frontmatter.pass
                ),
            }),
        });
    }

    // ── Track A: run — walk the active station's phase machine ───────────
    let phase = station_phase(&state, &station);
    let def = factory
        .station(&station)
        .ok_or_else(|| McpError::UnknownStation(station.clone()))?;
    let su = station_units(&units, &station);

    let spec_action = || RunAction::Spec {
        run: slug.to_string(),
        station: station.clone(),
        kills: def.kills.clone(),
    };

    let action = match phase {
        StationPhase::Spec => spec_action(),
        StationPhase::Review => RunAction::Review {
            run: slug.to_string(),
            station: station.clone(),
            reviewers: def.reviewers.clone(),
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
                        RunAction::Audit {
                            run: slug.to_string(),
                            station: station.clone(),
                            reviewers: def.reviewers.clone(),
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
            reviewers: def.reviewers.clone(),
        },
        StationPhase::Reflect => RunAction::Reflect {
            run: slug.to_string(),
            station: station.clone(),
        },
        StationPhase::Checkpoint => {
            // A right-sized run with `auto_gates` downgrades every gate to
            // `auto`; otherwise the station's factory-defined kind applies.
            let kind = if state.auto_gates {
                CheckpointKind::Auto
            } else {
                def.checkpoint
            };
            // An `external` gate hands off to an external review surface (a
            // PR/MR) rather than a local prompt — a distinct action so the
            // agent gets focused "open/annotate the review" instructions.
            if matches!(kind, CheckpointKind::External) {
                RunAction::ExternalReviewRequested {
                    run: slug.to_string(),
                    station: station.clone(),
                    target: String::new(),
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

/// The repo root the prompt cascade resolves overrides against.
///
/// The [`StateStore`] is rooted at `<repo_root>/.darkrun`, so the repo root is
/// that directory's parent. Project overrides live at
/// `<repo_root>/.darkrun/prompts/<rel>.md`.
fn cascade_repo_root(store: &StateStore) -> std::path::PathBuf {
    store
        .root()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| store.root().to_path_buf())
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
        RunAction::Checkpoint { .. } => "checkpoint",
        RunAction::FixFeedback { .. } => "fix_feedback",
        RunAction::FeedbackQuestion { .. } => "feedback_question",
        RunAction::ResolveDrift { .. } => "resolve_drift",
        RunAction::UnitsInvalid { .. } => "units_invalid",
        RunAction::Escalate { .. } => "escalate",
        RunAction::SafeRepair { .. } => "safe_repair",
        RunAction::ReviseUnitSpecs { .. } => "revise_unit_specs",
        RunAction::ExternalReviewRequested { .. } => "external_review_requested",
        RunAction::PendingSeal { .. } => "pending_seal",
        RunAction::Sealed { .. } => "sealed",
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
        | RunAction::Checkpoint { station, .. }
        | RunAction::FixFeedback { station, .. }
        | RunAction::FeedbackQuestion { station, .. }
        | RunAction::ResolveDrift { station, .. }
        | RunAction::UnitsInvalid { station, .. }
        | RunAction::Escalate { station, .. }
        | RunAction::SafeRepair { station, .. }
        | RunAction::ReviseUnitSpecs { station, .. }
        | RunAction::ExternalReviewRequested { station, .. } => Some(station.clone()),
        RunAction::PendingSeal { .. } | RunAction::Sealed { .. } | RunAction::Noop { .. } => None,
    };

    let mut ctx = PromptContext {
        run: slug.to_string(),
        station: station.clone(),
        phase: Some(action_tag(action).to_string()),
        ..Default::default()
    };

    // Resolve the station def for the roster / kills / artifact / checkpoint.
    if let Some(station) = station.as_deref() {
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
        if let Some(factory) = resolve_factory(&run.frontmatter.factory) {
            if let Some(def) = factory.station(station) {
                ctx.kills = Some(def.kills.clone());
                ctx.locked_artifact = Some(def.artifact.clone());
                ctx.kind = Some(def.checkpoint);
                ctx.workers = def.workers.clone();
                ctx.reviewers = def.reviewers.clone();
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
    }

    // Overlay the action-specific fields (these win over station-derived ones).
    match action {
        RunAction::Manufacture { worker, units, .. } => {
            ctx.worker = Some(worker.clone());
            // The action's wave-ready units are the ones the agent dispatches.
            ctx.units = units.clone();
        }
        RunAction::Checkpoint { kind, .. } => {
            ctx.kind = Some(*kind);
        }
        RunAction::FixFeedback { feedback_id, .. }
        | RunAction::FeedbackQuestion { feedback_id, .. } => {
            ctx.feedback_id = Some(feedback_id.clone());
        }
        RunAction::ResolveDrift { path, .. } => {
            ctx.path = Some(path.clone());
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
        RunAction::ExternalReviewRequested { target, .. } => {
            ctx.target = Some(target.clone());
            ctx.kind = Some(CheckpointKind::External);
        }
        RunAction::PendingSeal { kind, .. } => {
            ctx.seal = Some(kind.as_str().to_string());
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
pub fn run_tick(store: &StateStore, slug: &str) -> Result<TickResult> {
    // Sweep first: re-hash every locked artifact so a silent mutation surfaces
    // as a drift entry that Track C (inside derive_position) then preempts.
    crate::drift::sweep(store, slug)?;
    let position = derive_position(store, slug)?;

    let action = match &position.action {
        Some(a) => a.clone(),
        None => RunAction::Noop {
            run: slug.to_string(),
            message:
                "Mid-wave noop. Outstanding unit passes are still in flight — wait, then retick."
                    .to_string(),
        },
    };

    // Render the engine-driven instructions for this action BEFORE advancing
    // state, so the prompt reflects the action exactly as derived.
    let prompt = render_prompt(store, slug, &action)?;

    // Advance the phase write-cache based on the derived action.
    advance_state(store, slug, &action)?;

    Ok(TickResult {
        run: slug.to_string(),
        position,
        action,
        prompt,
    })
}

/// Stamp the station phase forward based on the action just emitted.
fn advance_state(store: &StateStore, slug: &str, action: &RunAction) -> Result<()> {
    let run = store.read_run(slug)?;
    let factory = resolve_factory(&run.frontmatter.factory)
        .ok_or_else(|| McpError::UnknownFactory(run.frontmatter.factory.clone()))?;
    let mut state = store.read_state(slug)?.unwrap_or_else(|| RunState {
        factory: run.frontmatter.factory.clone(),
        active_station: run.frontmatter.active_station.clone(),
        ..Default::default()
    });

    let now = Utc::now().to_rfc3339();

    match action {
        RunAction::Spec { station, .. } => {
            let st = ensure_station(&mut state, &factory, station)?;
            st.status = Status::InProgress;
            st.phase = StationPhase::Review;
            if st.started_at.is_none() {
                st.started_at = Some(now.clone());
            }
            state.active_station = station.clone();
        }
        RunAction::Review { station, .. } => {
            let st = ensure_station(&mut state, &factory, station)?;
            st.phase = StationPhase::Manufacture;
            state.active_station = station.clone();
        }
        RunAction::Manufacture { station, .. } => {
            // One wave per tick — stay in Manufacture until every unit locks.
            let st = ensure_station(&mut state, &factory, station)?;
            st.phase = StationPhase::Manufacture;
            state.active_station = station.clone();
        }
        RunAction::Audit { station, .. } => {
            // Audit absorbs what tests did — it verifies the output AND runs
            // the quality checks, then advances straight to Reflect.
            let st = ensure_station(&mut state, &factory, station)?;
            st.phase = StationPhase::Reflect;
            state.active_station = station.clone();
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
                // Snapshot the locked artifacts so the sweep can witness drift.
                crate::drift::record_station_witnesses(store, slug, station)?;
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
        // Validation / repair / feedback / drift / seal / noop actions are all
        // HOLDS — they don't advance the run phase machine on their own. The
        // next tick re-derives once the agent has cleared the condition.
        _ => {}
    }

    store.write_state(slug, &state)?;
    Ok(())
}

/// Ensure a `Station` entry exists in state, seeding it from the factory def.
fn ensure_station<'a>(
    state: &'a mut RunState,
    factory: &FactoryDef,
    station: &str,
) -> Result<&'a mut Station> {
    if !state.stations.contains_key(station) {
        let def = factory
            .station(station)
            .ok_or_else(|| McpError::UnknownStation(station.to_string()))?;
        state.stations.insert(
            station.to_string(),
            Station {
                station: station.to_string(),
                status: Status::Pending,
                phase: StationPhase::Spec,
                checkpoint: Some(Checkpoint {
                    kind: def.checkpoint,
                    entered_at: None,
                    outcome: None,
                }),
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

/// Start a fresh run: write `run.md`, right-size the station plan from `mode`,
/// seed `state.json` at the plan's first station in the `Spec` phase, and return
/// the run slug.
///
/// `mode` selects a [`resolve_template`]: `full`/unknown walks every factory
/// station with its own gates; `quick`/`bugfix`/`refactor` collapse to a station
/// subset with `auto` gates.
pub fn run_start(
    store: &StateStore,
    slug: &str,
    factory_name: &str,
    title: Option<String>,
    mode: &str,
) -> Result<Run> {
    let factory =
        resolve_factory(factory_name).ok_or_else(|| McpError::UnknownFactory(factory_name.into()))?;
    let factory_first = factory
        .first_station()
        .ok_or_else(|| McpError::UnknownFactory(factory_name.into()))?;

    // Right-size: the plan is the mode's station subset (empty = full factory).
    let (plan, auto_gates) = resolve_template(mode, &factory);
    let first_name = plan
        .first()
        .cloned()
        .unwrap_or_else(|| factory_first.name.clone());

    let now = Utc::now().to_rfc3339();
    let resolved_title = title.clone().unwrap_or_else(|| slug.to_string());
    let frontmatter = RunFrontmatter {
        title: title.clone(),
        factory: factory_name.to_string(),
        mode: mode.to_string(),
        active_station: first_name.clone(),
        status: Status::Active,
        started_at: Some(now.clone()),
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

    // Seed state at the plan's first station, Spec phase.
    let mut state = RunState {
        factory: factory_name.to_string(),
        active_station: first_name.clone(),
        plan,
        auto_gates,
        ..Default::default()
    };
    ensure_station(&mut state, &factory, &first_name)?;
    store.write_state(slug, &state)?;

    Ok(run)
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
    let factory = resolve_factory(&run.frontmatter.factory)
        .ok_or_else(|| McpError::UnknownFactory(run.frontmatter.factory.clone()))?;
    let mut state = store.read_state(slug)?.unwrap_or_default();
    let station = current_station(&factory, &state)
        .ok_or_else(|| McpError::NoActiveStation(slug.to_string()))?;

    let now = Utc::now().to_rfc3339();
    if approved {
        complete_station(&mut state, &factory, &station, &now)?;
        // Snapshot the locked artifacts so the sweep can witness drift.
        crate::drift::record_station_witnesses(store, slug, &station)?;
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
    fn run_start_seeds_state_at_first_station() {
        let (_d, store) = store();
        let run = run_start(&store, "my-run", "software", Some("Ship it".into()), "continuous")
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
        run_start(&store, "f", "software", None, "continuous").expect("start");
        let state = store.read_state("f").unwrap().unwrap();
        assert!(state.plan.is_empty(), "full mode walks the whole factory");
        assert!(!state.auto_gates);
        assert_eq!(state.active_station, "frame");
    }

    #[test]
    fn quick_mode_right_sizes_plan_and_auto_gates() {
        let (_d, store) = store();
        run_start(&store, "q", "software", Some("Small fix".into()), "quick").expect("start");
        let state = store.read_state("q").unwrap().unwrap();
        assert_eq!(state.plan, vec!["build".to_string(), "prove".to_string()]);
        assert!(state.auto_gates);
        // The run starts at the plan's first station, not the factory's.
        assert_eq!(state.active_station, "build");
        assert_eq!(state.stations["build"].phase, StationPhase::Spec);
    }

    #[test]
    fn quick_run_walks_only_planned_stations_to_sealed() {
        let (_d, store) = store();
        run_start(&store, "q", "software", None, "quick").expect("start");

        // Drive to sealed. auto_gates downgrades every checkpoint to auto, so no
        // operator decision is needed; we just decompose+complete each station.
        let mut guard = 0;
        loop {
            guard += 1;
            assert!(guard < 100, "quick run failed to converge");
            let t = run_tick(&store, "q").expect("tick");
            match &t.action {
                RunAction::Sealed { .. } => break,
                RunAction::Spec { station, .. } => {
                    let unit = Unit {
                        slug: format!("{station}-u"),
                        frontmatter: darkrun_core::domain::UnitFrontmatter {
                            status: Status::Pending,
                            station: Some(station.clone()),
                            ..Default::default()
                        },
                        title: "u".into(),
                        body: String::new(),
                    };
                    store.write_unit("q", &unit).expect("write unit");
                }
                RunAction::Manufacture { station, units, .. } => {
                    let _ = station;
                    for u in units {
                        let mut done = store.read_unit("q", u).unwrap();
                        done.frontmatter.status = Status::Completed;
                        store.write_unit("q", &done).unwrap();
                    }
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

    #[test]
    fn unknown_mode_falls_back_to_full_plan() {
        let (_d, store) = store();
        run_start(&store, "u", "software", None, "nonsense-mode").expect("start");
        let state = store.read_state("u").unwrap().unwrap();
        assert!(state.plan.is_empty());
        assert_eq!(state.active_station, "frame");
    }

    #[test]
    fn run_next_walks_first_station_through_its_phases() {
        let (_d, store) = store();
        run_start(&store, "r", "software", None, "continuous").expect("start");

        // Tick 1: Spec (frame). State advances to Review.
        let t1 = run_tick(&store, "r").expect("t1");
        assert!(matches!(
            t1.action,
            RunAction::Spec { ref station, .. } if station == "frame"
        ));
        assert_eq!(
            store.read_state("r").unwrap().unwrap().stations["frame"].phase,
            StationPhase::Review
        );

        // Tick 2: Review (frame). State advances to Manufacture.
        let t2 = run_tick(&store, "r").expect("t2");
        assert!(matches!(
            t2.action,
            RunAction::Review { ref station, .. } if station == "frame"
        ));
        assert_eq!(
            store.read_state("r").unwrap().unwrap().stations["frame"].phase,
            StationPhase::Manufacture
        );

        // Decompose a unit, then Manufacture dispatches its pass.
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

        let t3 = run_tick(&store, "r").expect("t3");
        assert!(
            matches!(t3.action, RunAction::Manufacture { ref units, .. } if units == &vec!["u1".to_string()]),
            "expected Manufacture, got {:?}",
            t3.action
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
        run_start(&store, "r", "software", None, "continuous").expect("start");

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
        run_start(&store, "r", "software", None, "continuous").expect("start");
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
        run_start(&store, "r", "software", None, "continuous").expect("start");
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
        run_start(&store, "r", "software", None, "continuous").expect("start");

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

    // ── Surface routing into the rendered prompt ─────────────────────────────

    /// A classified visual surface lights up the headless-browser route in the
    /// rendered Audit prompt.
    #[test]
    fn audit_prompt_routes_visual_surface_through_render() {
        let (_d, store) = store();
        run_start(&store, "r", "software", None, "continuous").expect("start");
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
        run_start(&store, "r", "software", None, "continuous").expect("start");
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
        run_start(&store, "r", "software", None, "continuous").expect("start");
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
}
