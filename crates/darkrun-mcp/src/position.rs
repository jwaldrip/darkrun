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
    Checkpoint, CheckpointKind, CheckpointOutcome, Run, RunFrontmatter, Station, StationPhase,
    Status, Unit,
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

/// Derive the station's current phase from its persisted state, defaulting to
/// `Spec` for a freshly-entered station.
fn station_phase(state: &RunState, station: &str) -> StationPhase {
    state
        .stations
        .get(station)
        .map(|s| s.phase)
        .unwrap_or(StationPhase::Spec)
}

/// Find the first station in the plan that is not yet `Completed`.
fn current_station<'a>(factory: &'a FactoryDef, state: &RunState) -> Option<&'a str> {
    factory
        .stations
        .iter()
        .find(|s| {
            state
                .stations
                .get(&s.name)
                .map(|st| !matches!(st.status, Status::Completed))
                .unwrap_or(true)
        })
        .map(|s| s.name.as_str())
}

/// Track B — feedback. Returns a `FixFeedback` action for the first open
/// feedback item, or `None` when no open feedback exists. Feedback is "open"
/// when its `status:` frontmatter line is not a terminal value.
fn walk_feedback(store: &StateStore, slug: &str, station: &str) -> Result<Option<RunAction>> {
    let raw = store.read_feedback_raw(slug)?;
    for (id, content) in raw {
        if feedback_open(&content) {
            return Ok(Some(RunAction::FixFeedback {
                run: slug.to_string(),
                station: station.to_string(),
                feedback_id: id,
            }));
        }
    }
    Ok(None)
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
        Some(s) => s.to_string(),
        None => {
            // Every station locked → sealed.
            return Ok(Position {
                track: Track::Run,
                action: Some(RunAction::Sealed {
                    run: slug.to_string(),
                }),
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
        StationPhase::Checkpoint => RunAction::Checkpoint {
            run: slug.to_string(),
            station: station.clone(),
            kind: def.checkpoint,
        },
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
fn action_tag(action: &RunAction) -> &'static str {
    match action {
        RunAction::Spec { .. } => "spec",
        RunAction::Review { .. } => "review",
        RunAction::Manufacture { .. } => "manufacture",
        RunAction::Audit { .. } => "audit",
        RunAction::Reflect { .. } => "reflect",
        RunAction::Checkpoint { .. } => "checkpoint",
        RunAction::FixFeedback { .. } => "fix_feedback",
        RunAction::ResolveDrift { .. } => "resolve_drift",
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
        | RunAction::ResolveDrift { station, .. } => Some(station.clone()),
        RunAction::Sealed { .. } | RunAction::Noop { .. } => None,
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
        RunAction::FixFeedback { feedback_id, .. } => {
            ctx.feedback_id = Some(feedback_id.clone());
        }
        RunAction::ResolveDrift { path, .. } => {
            ctx.path = Some(path.clone());
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
            }
        }
        // Feedback / drift / noop / sealed actions don't advance the run
        // phase machine on their own.
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
    if let Some(next) = factory.next_station(station) {
        let next_name = next.name.clone();
        let st = ensure_station(state, factory, &next_name)?;
        st.status = Status::Pending;
        st.phase = StationPhase::Spec;
        state.active_station = next_name;
    }
    Ok(())
}

/// Start a fresh run: write `run.md`, seed `state.json` at the factory's first
/// station in the `Spec` phase, and return the run slug.
///
/// The auto right-sizing pass is a future enhancement; this slice always seeds
/// the full station plan.
pub fn run_start(
    store: &StateStore,
    slug: &str,
    factory_name: &str,
    title: Option<String>,
    mode: &str,
) -> Result<Run> {
    let factory =
        resolve_factory(factory_name).ok_or_else(|| McpError::UnknownFactory(factory_name.into()))?;
    let first = factory
        .first_station()
        .ok_or_else(|| McpError::UnknownFactory(factory_name.into()))?;

    let now = Utc::now().to_rfc3339();
    let resolved_title = title.clone().unwrap_or_else(|| slug.to_string());
    let frontmatter = RunFrontmatter {
        title: title.clone(),
        factory: factory_name.to_string(),
        mode: mode.to_string(),
        active_station: first.name.clone(),
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

    // Seed state at the first station, Spec phase.
    let mut state = RunState {
        factory: factory_name.to_string(),
        active_station: first.name.clone(),
        ..Default::default()
    };
    ensure_station(&mut state, &factory, &first.name)?;
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
        .ok_or_else(|| McpError::NoActiveStation(slug.to_string()))?
        .to_string();

    let now = Utc::now().to_rfc3339();
    if approved {
        complete_station(&mut state, &factory, &station, &now)?;
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
}
