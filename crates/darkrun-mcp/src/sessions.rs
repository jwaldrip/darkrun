//! Visual-interaction sessions — the engine side of the mid-run operator
//! prompts the agent can raise while a UI-facing run is in flight.
//!
//! Three interaction kinds live here, each building the matching
//! [`darkrun_api`] [`SessionPayload`] variant:
//!
//! - **question** — a VISUAL QUESTION: a prompt plus a list of options (each an
//!   optionally-image-backed design choice) the operator picks among
//!   (single- or multi-select), optionally over a set of reference images.
//! - **direction** — a DESIGN DIRECTION: a set of design archetypes (each an
//!   image-backed direction) the operator chooses + annotates (pins, a captured
//!   screenshot, comments).
//! - **picker** — a plain blocking selection among labelled options.
//!
//! ## How a session reaches the desktop app
//!
//! There is NO on-disk bridge. Interactive sessions are EPHEMERAL — tied to the
//! agent/MCP session, not to durable run state. The MCP server and the HTTP/WS
//! server run IN THE SAME PROCESS and share one in-memory
//! [`darkrun_http::SessionRegistry`]. Every visual session is upserted into that
//! shared registry; the in-process HTTP server reads the same registry to serve
//! `GET /api/session/:id` and pushes live updates over `GET /ws/session/:id`.
//! When the operator answers, the HTTP handler records the answer/selection back
//! onto the payload in the registry; the agent reads it back here via
//! [`question_result`] / [`direction_result`] / [`picker_result`] (or the
//! `darkrun_*_result` tools).
//!
//! The registry is keyed by `session_id` so a single run can hold several
//! concurrent visual sessions without clobbering each other.


use darkrun_api::common::GateType;
use darkrun_api::session::{
    ApproveAction, ApproveActionKind, DirectionArchetype, DirectionSessionPayload, PickerKind,
    PickerOption, PickerSessionPayload, QuestionOption, QuestionSessionPayload, ReviewSessionPayload,
    RunCurrentState, RunPhase, SessionPayload, StationStateInfo,
};
use darkrun_api::SessionStatus;
use darkrun_core::domain::{CheckpointKind, StationPhase};
use darkrun_core::StateStore;
use serde::Serialize;

use crate::error::{McpError, Result};

/// The focus channel the desktop home watches: when a Review payload appears
/// under this session id, the app navigates to the run it names.
pub const CURRENT_SESSION: &str = "current";

/// Raise a run's review surface for the **desktop app** — the only interactive
/// surface darkrun drives. Builds a [`ReviewSessionPayload`] from the run's real
/// state and upserts it twice: once under the run slug (the id the desktop
/// subscribes to for the live feed) and once under [`CURRENT_SESSION`] (the
/// focus pointer the home screen watches so it navigates to this run). Returns
/// the run slug — the session id the desktop renders.
pub fn create_show(registry: &SessionRegistry, store: &StateStore, slug: &str) -> Result<String> {
    create_show_with_focus(registry, store, slug, true)
}

/// [`create_show`] minus the focus side-effect when `focus` is false: upserts
/// the run-slug session WITHOUT repointing [`CURRENT_SESSION`]. The lazy HTTP
/// materializer uses this — a desktop *asking* for a session is navigation the
/// operator already made; it must not steal every other window's focus.
pub fn create_show_with_focus(
    registry: &SessionRegistry,
    store: &StateStore,
    slug: &str,
    focus: bool,
) -> Result<String> {
    let run = store.read_run(slug)?;
    let units = store.read_units(slug)?;
    let run_json = serde_json::to_value(&run).ok();
    let unit_jsons: Vec<serde_json::Value> =
        units.iter().filter_map(|u| serde_json::to_value(u).ok()).collect();

    // Derive the per-factory STATION strip + the live PHASE from the run's real
    // state. The factory declares the ordered station list; `RunState` carries
    // each station's derived status/phase. Absent state (a run with no
    // `state.json` yet) yields an empty strip and no phase — the payload stays
    // valid, the pipeline just renders nothing rather than stale data.
    let state = store.read_state(slug)?;
    let factory_stations: Vec<String> = crate::position::resolve_factory_for(store, &run.frontmatter.factory)
        .map(|f| f.station_names())
        .unwrap_or_default();

    let (station_states, current_state, gate) = match state {
        Some(state) => {
            // An ORDERED list (factory station order from station_status_summary),
            // not a map — the desktop renders the strip in this order, so it must
            // not be re-sorted alphabetically by a map key.
            let mut station_states: Vec<StationStateInfo> = Vec::new();
            for entry in state.station_status_summary(&factory_stations) {
                let recorded = state.stations.get(&entry.station);
                let checkpoint = recorded.and_then(|st| st.checkpoint.as_ref());
                station_states.push(StationStateInfo {
                    station: entry.station.clone(),
                    merged_into_main: matches!(
                        entry.status,
                        darkrun_core::domain::Status::Completed
                    ),
                    status: enum_token(&entry.status),
                    phase: enum_token(&entry.phase),
                    started_at: recorded.and_then(|st| st.started_at.clone()),
                    completed_at: recorded.and_then(|st| st.completed_at.clone()),
                    gate_entered_at: checkpoint.and_then(|c| c.entered_at.clone()),
                    gate_outcome: checkpoint
                        .and_then(|c| c.outcome.as_ref())
                        .and_then(enum_token),
                });
            }
            let current = RunCurrentState {
                factory: state.factory.clone(),
                station: state.active_station.clone(),
                phase: Some(run_phase(state.active_phase())),
                ..Default::default()
            };
            // The approval gate. TWO operator holds surface the Approve /
            // Request-changes bar to the desktop — both decided by
            // `darkrun_checkpoint_decide`, and (auto/dark never parks at either):
            //
            //   - the post-execution **Checkpoint** — the station was made,
            //     audited, and is asking to lock; and
            //   - the pre-execution **UserGate** — the spec is reviewed and the
            //     operator clears it BEFORE any Unit is manufactured.
            //
            // The gate kind is the run's global mode gate (no per-station
            // overrides). The trailing bool marks the pre-execution gate so the
            // Approve button reads "start execution" rather than "complete".
            let gate: Option<(GateType, String, bool)> = match state.active_phase() {
                StationPhase::Checkpoint => {
                    let decided = state
                        .stations
                        .get(&state.active_station)
                        .and_then(|s| s.checkpoint.as_ref())
                        .and_then(|c| c.outcome.as_ref())
                        .is_some();
                    match (decided, run.frontmatter.mode.gate()) {
                        (false, CheckpointKind::Ask) => {
                            Some((GateType::Ask, state.active_station.clone(), false))
                        }
                        (false, CheckpointKind::External) => {
                            Some((GateType::External, state.active_station.clone(), false))
                        }
                        _ => None,
                    }
                }
                // The pre-execution operator gate is OPEN while the phase holds at
                // UserGate (cleared by the decide that advances it to Manufacture).
                StationPhase::UserGate => match run.frontmatter.mode.gate() {
                    CheckpointKind::Ask => {
                        Some((GateType::Ask, state.active_station.clone(), true))
                    }
                    CheckpointKind::External => {
                        Some((GateType::External, state.active_station.clone(), true))
                    }
                    _ => None,
                },
                _ => None,
            };
            (station_states, Some(current), gate)
        }
        None => (Vec::new(), None, None),
    };

    // Expand the gate into the review payload's approval fields. The desktop
    // renders the checkpoint bar only when `await_active` is set.
    let (gate_type, gate_station, approve_action, await_active) = match gate {
        Some((gt, station, pre_exec)) => {
            let mut chars = station.chars();
            let cap = chars
                .next()
                .map(|f| f.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default();
            // The pre-execution gate releases the manufacture wave; the
            // post-execution gate locks the station. Distinct verbs + kinds so
            // the desktop's Approve button names the real consequence.
            let (label, kind) = if pre_exec {
                (format!("Start {cap} execution"), ApproveActionKind::StartExecution)
            } else {
                (format!("Complete {cap} station"), ApproveActionKind::CompleteStation)
            };
            (
                Some(gt),
                Some(station),
                Some(ApproveAction { label, kind }),
                Some(true),
            )
        }
        None => (None, None, None, None),
    };

    // Per-station narrative artifacts: the pre-execution briefs ("what I'm going
    // to do", surfaced before the review gate) and the closing outcomes ("what
    // the station produced", surfaced before the checkpoint) the agent persisted
    // via `darkrun_brief_record`. Split by phase into the two payload maps.
    let mut station_briefs: std::collections::BTreeMap<String, String> = Default::default();
    let mut station_outcomes: std::collections::BTreeMap<String, String> = Default::default();
    for b in crate::brief::list(store, slug).unwrap_or_default() {
        if b.station.is_empty() || b.body.is_empty() {
            continue;
        }
        match b.phase {
            crate::brief::BriefPhase::Pre => {
                station_briefs.insert(b.station, b.body);
            }
            crate::brief::BriefPhase::Post => {
                station_outcomes.insert(b.station, b.body);
            }
        }
    }

    let build = |session_id: &str| {
        SessionPayload::Review(ReviewSessionPayload {
            session_id: session_id.to_string(),
            status: SessionStatus::Pending,
            run_slug: Some(slug.to_string()),
            run: run_json.clone(),
            units: unit_jsons.clone(),
            station_states: station_states.clone(),
            current_state: current_state.clone(),
            station_briefs: station_briefs.clone(),
            station_outcomes: station_outcomes.clone(),
            gate_type,
            station: gate_station.clone(),
            approve_action: approve_action.clone(),
            await_active,
            ..Default::default()
        })
    };
    registry.upsert(build(slug));
    if focus {
        registry.upsert(build(CURRENT_SESSION));
    }

    // Re-attach any operator sessions persisted for this run (questions /
    // directions / pickers), so they survive an engine restart: load each
    // back into the registry under its canonical id (so the operator's answer
    // and `..._result` still resolve), and if one is still OPEN, surface it
    // onto the run channel — the desktop opens the run straight onto the
    // pending question instead of the review.
    hydrate_interactive(registry, store, slug);

    Ok(slug.to_string())
}

/// Load a run's persisted interactive sessions into the registry (only those
/// not already in memory, so a fresher live answer is never clobbered by a
/// staler disk copy), then mirror the most recent OPEN one onto the run
/// channel so the desktop surfaces it. Idempotent; safe to call every tick.
fn hydrate_interactive(registry: &SessionRegistry, store: &StateStore, slug: &str) {
    for payload in store.list_interactive_sessions(slug) {
        if !registry.contains(payload.session_id()) {
            registry.upsert(payload);
        }
    }
    if let Some(open) = store.latest_open_interactive(slug) {
        // Mirror under the run slug only — keep `current` pointing at the run
        // (a review focus) so the home still navigates correctly.
        registry.upsert_under(slug, open);
    }
}

/// Serialize a snake_case-tagged unit enum to its wire token (the `Option<String>`
/// display shims the review payload carries). Returns `None` only if the value
/// somehow fails to serialize as a JSON string.
fn enum_token<T: Serialize>(value: &T) -> Option<String> {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
}

/// Map the core [`StationPhase`] onto the API's [`RunPhase`] — the two share the
/// fixed universal taxonomy (`spec/review/manufacture/audit/reflect/checkpoint`),
/// kept as separate types to keep `darkrun-api` dependency-light.
fn run_phase(phase: StationPhase) -> RunPhase {
    match phase {
        StationPhase::Spec => RunPhase::Spec,
        // The pre-execution USER gate is the review stage's operator hold —
        // surfaced under `review` in the universal taxonomy.
        StationPhase::Review | StationPhase::UserGate => RunPhase::Review,
        StationPhase::Manufacture => RunPhase::Manufacture,
        StationPhase::Audit => RunPhase::Audit,
        StationPhase::Reflect => RunPhase::Reflect,
        StationPhase::Checkpoint => RunPhase::Checkpoint,
    }
}

/// The in-memory interactive-session registry shared between the MCP tool
/// handlers and the in-process HTTP/WS server. Re-exported from `darkrun-http`
/// so callers thread a single shared handle through both halves.
pub use darkrun_http::SessionRegistry;

/// The structured "awaiting answer" result returned by the create tools — the
/// minted session id, its kind, status, and the wire path the desktop app
/// reads it from.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AwaitingSession {
    /// The minted session id.
    pub session_id: String,
    /// The run the session belongs to.
    pub run_slug: String,
    /// The session kind (`question` / `direction` / `picker`).
    pub session_type: String,
    /// The lifecycle status — `pending` at creation time.
    pub status: SessionStatus,
    /// Whether the agent should block waiting on an operator decision.
    pub awaiting_answer: bool,
    /// The `GET /api/session/:id` path the desktop app fetches this from.
    pub session_path: String,
    /// The `GET /ws/session/:id` path for live updates.
    pub ws_path: String,
}

// ── question ────────────────────────────────────────────────────────────────

/// A validated option spec used to build a [`QuestionOption`].
#[derive(Debug, Clone)]
pub struct QuestionOptionSpec {
    /// Canonical option id (echoed back in the answer's `selected[]`).
    pub id: String,
    /// Display label.
    pub label: String,
    /// Optional generated-image URL (dark-theme variant when a light one is set).
    pub image_url: Option<String>,
    /// Optional light-theme variant of `image_url`.
    pub image_url_light: Option<String>,
    /// Optional longer description.
    pub description: Option<String>,
}

/// Build + register a VISUAL QUESTION session and return its awaiting handle.
///
/// Validation: the prompt must be non-empty; at least one option is required;
/// option ids must be non-empty and unique.
#[allow(clippy::too_many_arguments)]
pub fn create_question(
    registry: &SessionRegistry,
    run: &str,
    title: Option<String>,
    prompt: &str,
    context: Option<String>,
    options: Vec<QuestionOptionSpec>,
    multi_select: bool,
    image_urls: Vec<String>,
) -> Result<AwaitingSession> {
    if prompt.trim().is_empty() {
        return Err(McpError::InvalidInput("question prompt is required".into()));
    }
    if options.is_empty() {
        return Err(McpError::InvalidInput(
            "a question needs at least one option".into(),
        ));
    }
    let mut seen = std::collections::HashSet::new();
    let mut built = Vec::with_capacity(options.len());
    for opt in options {
        if opt.id.trim().is_empty() {
            return Err(McpError::InvalidInput(
                "every question option needs a non-empty id".into(),
            ));
        }
        if opt.label.trim().is_empty() {
            return Err(McpError::InvalidInput(format!(
                "question option '{}' needs a non-empty label",
                opt.id
            )));
        }
        if !seen.insert(opt.id.clone()) {
            return Err(McpError::InvalidInput(format!(
                "duplicate question option id: {}",
                opt.id
            )));
        }
        built.push(QuestionOption {
            id: opt.id,
            label: opt.label,
            image_url: opt.image_url.filter(|s| !s.trim().is_empty()),
            image_url_light: opt.image_url_light.filter(|s| !s.trim().is_empty()),
            description: opt.description.filter(|s| !s.trim().is_empty()),
        });
    }

    let session_id = registry.next_session_id("q");
    let payload = QuestionSessionPayload {
        session_id: session_id.clone(),
        status: SessionStatus::Pending,
        run_slug: Some(run.to_string()),
        title,
        prompt: prompt.trim().to_string(),
        context,
        options: built,
        multi_select,
        answer: None,
        image_urls: image_urls
            .into_iter()
            .filter(|u| !u.trim().is_empty())
            .collect(),
    };
    surface_interactive(registry, run, SessionPayload::Question(payload));

    Ok(awaiting(run, &session_id, "question"))
}

/// Raise an interactive operator session (question / direction / picker) so the
/// desktop shows it RIGHT NOW, wherever it is:
///   - the CANONICAL upsert (under the payload's own `q-NN`/`d-NN`/`p-NN` id)
///     is what `..._result` reads back and what the operator's answer targets —
///     and it fires the persist hook, writing the session to the run's
///     `interactive/` dir so it survives a restart;
///   - a MIRROR under the run slug pushes it onto the channel a desktop viewing
///     the run is already subscribed to, so the question appears without a
///     navigation; the mirror keeps the canonical `session_id`, so the answer
///     still routes to `q-NN`;
///   - the `current` focus pointer (a minimal review naming the run) makes the
///     home screen NAVIGATE to the run if the desktop isn't already on it.
fn surface_interactive(registry: &SessionRegistry, run: &str, payload: SessionPayload) {
    registry.upsert(payload.clone());
    registry.upsert_under(run, payload);
    registry.upsert_under(CURRENT_SESSION, focus_pointer(run));
}

/// A minimal Review payload under [`CURRENT_SESSION`] that names `run` — just
/// enough for the desktop home poller (which reads `run_slug` off a `review`
/// focus session) to navigate to the run.
fn focus_pointer(run: &str) -> SessionPayload {
    SessionPayload::Review(ReviewSessionPayload {
        session_id: CURRENT_SESSION.to_string(),
        status: SessionStatus::Pending,
        run_slug: Some(run.to_string()),
        ..Default::default()
    })
}

/// Read back the answer to a question session, if the operator has submitted
/// one. Returns the whole [`QuestionSessionPayload`] so the caller sees both
/// the answer and the current `status`.
pub fn question_result(
    registry: &SessionRegistry,
    run: &str,
    session_id: &str,
) -> Result<QuestionSessionPayload> {
    match registry.get(session_id) {
        Some(SessionPayload::Question(q)) => Ok(q),
        Some(_) => Err(McpError::InvalidInput(format!(
            "session '{session_id}' is not a question session"
        ))),
        None => Err(McpError::InvalidInput(format!(
            "no session '{session_id}' on run '{run}'"
        ))),
    }
}

// ── direction ───────────────────────────────────────────────────────────────

/// A validated archetype spec used to build a [`DirectionArchetype`].
#[derive(Debug, Clone)]
pub struct ArchetypeSpec {
    /// Canonical archetype id (echoed back as `chosen_archetype`).
    pub id: String,
    /// Display label.
    pub label: String,
    /// Generated preview-image URL (required for a direction archetype).
    pub image_url: String,
    /// Optional light-theme variant of `image_url`.
    pub image_url_light: Option<String>,
    /// Description of the design direction.
    pub description: String,
}

/// Build + register a DESIGN DIRECTION session and return its awaiting handle.
///
/// Validation: the prompt must be non-empty; at least one archetype is
/// required; every archetype needs a non-empty id, label, image_url, and
/// description; ids must be unique.
pub fn create_direction(
    registry: &SessionRegistry,
    run: &str,
    title: Option<String>,
    prompt: &str,
    context: Option<String>,
    archetypes: Vec<ArchetypeSpec>,
) -> Result<AwaitingSession> {
    if prompt.trim().is_empty() {
        return Err(McpError::InvalidInput(
            "direction prompt is required".into(),
        ));
    }
    if archetypes.is_empty() {
        return Err(McpError::InvalidInput(
            "a direction needs at least one archetype".into(),
        ));
    }
    let mut seen = std::collections::HashSet::new();
    let mut built = Vec::with_capacity(archetypes.len());
    for arch in archetypes {
        if arch.id.trim().is_empty() {
            return Err(McpError::InvalidInput(
                "every archetype needs a non-empty id".into(),
            ));
        }
        if arch.label.trim().is_empty() {
            return Err(McpError::InvalidInput(format!(
                "archetype '{}' needs a non-empty label",
                arch.id
            )));
        }
        if arch.image_url.trim().is_empty() {
            return Err(McpError::InvalidInput(format!(
                "archetype '{}' needs a non-empty image_url",
                arch.id
            )));
        }
        if arch.description.trim().is_empty() {
            return Err(McpError::InvalidInput(format!(
                "archetype '{}' needs a non-empty description",
                arch.id
            )));
        }
        if !seen.insert(arch.id.clone()) {
            return Err(McpError::InvalidInput(format!(
                "duplicate archetype id: {}",
                arch.id
            )));
        }
        built.push(DirectionArchetype {
            id: arch.id,
            label: arch.label,
            image_url: arch.image_url,
            image_url_light: arch.image_url_light.filter(|s| !s.trim().is_empty()),
            description: arch.description,
        });
    }

    let session_id = registry.next_session_id("d");
    let payload = DirectionSessionPayload {
        session_id: session_id.clone(),
        status: SessionStatus::Pending,
        title,
        run_slug: Some(run.to_string()),
        prompt: prompt.trim().to_string(),
        context,
        archetypes: built,
        chosen_archetype: None,
        annotations: None,
    };
    surface_interactive(registry, run, SessionPayload::Direction(payload));

    Ok(awaiting(run, &session_id, "direction"))
}

/// Read back the chosen archetype + annotations for a direction session.
pub fn direction_result(
    registry: &SessionRegistry,
    run: &str,
    session_id: &str,
) -> Result<DirectionSessionPayload> {
    match registry.get(session_id) {
        Some(SessionPayload::Direction(d)) => Ok(d),
        Some(_) => Err(McpError::InvalidInput(format!(
            "session '{session_id}' is not a direction session"
        ))),
        None => Err(McpError::InvalidInput(format!(
            "no session '{session_id}' on run '{run}'"
        ))),
    }
}

// ── picker ──────────────────────────────────────────────────────────────────

/// A validated option spec used to build a [`PickerOption`].
#[derive(Debug, Clone)]
pub struct PickerOptionSpec {
    /// Canonical option id (echoed back on selection).
    pub id: String,
    /// Display label.
    pub label: String,
    /// Optional description.
    pub description: Option<String>,
    /// Whether the option hides behind a "show all" expansion.
    pub secondary: Option<bool>,
}

/// Parse a picker-kind string into the typed [`PickerKind`].
pub fn parse_picker_kind(raw: &str) -> Option<PickerKind> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "factory" => Some(PickerKind::Factory),
        "mode" => Some(PickerKind::Mode),
        "station" => Some(PickerKind::Station),
        "confirm" => Some(PickerKind::Confirm),
        "url_input" | "urlinput" | "url" => Some(PickerKind::UrlInput),
        _ => None,
    }
}

/// Build + register a blocking PICKER session and return its awaiting handle.
///
/// Validation: the title + prompt must be non-empty; at least one option is
/// required; option ids must be non-empty and unique.
pub fn create_picker(
    registry: &SessionRegistry,
    run: &str,
    kind: PickerKind,
    title: &str,
    prompt: &str,
    options: Vec<PickerOptionSpec>,
) -> Result<AwaitingSession> {
    if title.trim().is_empty() {
        return Err(McpError::InvalidInput("picker title is required".into()));
    }
    if prompt.trim().is_empty() {
        return Err(McpError::InvalidInput("picker prompt is required".into()));
    }
    if options.is_empty() {
        return Err(McpError::InvalidInput(
            "a picker needs at least one option".into(),
        ));
    }
    let mut seen = std::collections::HashSet::new();
    let mut built = Vec::with_capacity(options.len());
    for opt in options {
        if opt.id.trim().is_empty() {
            return Err(McpError::InvalidInput(
                "every picker option needs a non-empty id".into(),
            ));
        }
        if opt.label.trim().is_empty() {
            return Err(McpError::InvalidInput(format!(
                "picker option '{}' needs a non-empty label",
                opt.id
            )));
        }
        if !seen.insert(opt.id.clone()) {
            return Err(McpError::InvalidInput(format!(
                "duplicate picker option id: {}",
                opt.id
            )));
        }
        built.push(PickerOption {
            id: opt.id,
            label: opt.label,
            description: opt.description.filter(|s| !s.trim().is_empty()),
            secondary: opt.secondary,
        });
    }

    let session_id = registry.next_session_id("p");
    let payload = PickerSessionPayload {
        session_id: session_id.clone(),
        status: SessionStatus::Pending,
        run_slug: Some(run.to_string()),
        kind,
        title: title.trim().to_string(),
        prompt: prompt.trim().to_string(),
        options: built,
        selection: None,
    };
    surface_interactive(registry, run, SessionPayload::Picker(payload));

    Ok(awaiting(run, &session_id, "picker"))
}

/// Read back the selection for a picker session.
pub fn picker_result(
    registry: &SessionRegistry,
    run: &str,
    session_id: &str,
) -> Result<PickerSessionPayload> {
    match registry.get(session_id) {
        Some(SessionPayload::Picker(p)) => Ok(p),
        Some(_) => Err(McpError::InvalidInput(format!(
            "session '{session_id}' is not a picker session"
        ))),
        None => Err(McpError::InvalidInput(format!(
            "no session '{session_id}' on run '{run}'"
        ))),
    }
}

/// Build the awaiting-session handle for a freshly-minted session. The session
/// paths mirror `darkrun_api::routes::paths`.
fn awaiting(run: &str, session_id: &str, session_type: &str) -> AwaitingSession {
    AwaitingSession {
        session_id: session_id.to_string(),
        run_slug: run.to_string(),
        session_type: session_type.to_string(),
        status: SessionStatus::Pending,
        awaiting_answer: true,
        session_path: format!("/api/session/{session_id}"),
        ws_path: format!("/ws/session/{session_id}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use darkrun_core::domain::Mode;
    use darkrun_api::session::{
        DirectionAnnotations, DirectionPin, PickerSelection, QuestionAnswer,
    };

    fn registry() -> SessionRegistry {
        SessionRegistry::new()
    }

    #[test]
    fn create_show_handles_a_run_with_no_state_yet() {
        use darkrun_core::domain::{Run, RunFrontmatter};
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new(dir.path());
        // A run document with NO state.json (never ticked) → the station strip is
        // empty and there's no current phase, but the payload still builds.
        store.write_run(&Run {
            slug: "r".into(), title: "R".into(), body: String::new(),
            frontmatter: RunFrontmatter { factory: "software".into(), active_station: "frame".into(), ..Default::default() },
        }).unwrap();
        let reg = registry();
        let id = create_show(&reg, &store, "r").expect("show builds without state");
        assert_eq!(id, "r");
        assert!(reg.get("r").is_some(), "the session is still registered");
    }

    #[test]
    fn create_show_raises_run_review_under_slug_and_current() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new(dir.path());
        crate::position::run_start(&store, "r", "software", None, Mode::Solo, "full").unwrap();
        let reg = registry();

        let id = create_show(&reg, &store, "r").expect("show");
        assert_eq!(id, "r");

        // The run is reachable both under its slug (the desktop's live feed) and
        // under `current` (the focus pointer the home navigates to).
        for sid in ["r", CURRENT_SESSION] {
            match reg.get(sid).expect("session present") {
                SessionPayload::Review(rev) => {
                    assert_eq!(rev.session_id, sid);
                    assert_eq!(rev.run_slug.as_deref(), Some("r"));
                    assert!(rev.run.is_some(), "run json populated");

                    // A freshly-started run sits at `frame`/Spec, and the strip
                    // carries every one of the factory's ordered stations.
                    let cur = rev.current_state.as_ref().expect("current_state set");
                    assert_eq!(cur.factory, "software");
                    assert_eq!(cur.station, "frame");
                    assert_eq!(cur.phase, Some(RunPhase::Spec));

                    let order: Vec<&str> =
                        rev.station_states.iter().map(|s| s.station.as_str()).collect();
                    // An ordered Vec in factory line order — exact, not alphabetical.
                    assert_eq!(
                        order,
                        ["frame", "specify", "shape", "build", "prove", "harden"],
                        "stations in factory order"
                    );
                }
                other => panic!("expected a review session, got {other:?}"),
            }
        }
    }

    #[test]
    fn create_show_payload_carries_ordered_stations_phase_and_statuses() {
        use darkrun_core::domain::{
            Checkpoint, CheckpointKind, CheckpointOutcome, Station, StationPhase, Status,
        };

        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new(dir.path());
        crate::position::run_start(&store, "r", "software", None, Mode::Solo, "full").unwrap();

        // Hand-build a mid-run state: `frame` locked (Completed, gate advanced),
        // `specify` the active station mid-Manufacture, everything downstream
        // untouched (Pending/Spec, no entry).
        let mut state = store.read_state("r").unwrap().unwrap();
        state.active_station = "specify".into();
        state.stations.insert(
            "frame".into(),
            Station {
                station: "frame".into(),
                status: Status::Completed,
                phase: StationPhase::Checkpoint,
            elaborated: false,
                checkpoint: Some(Checkpoint {
                    kind: CheckpointKind::Ask,
                    entered_at: Some("2026-05-31T00:00:00Z".into()),
                    outcome: Some(CheckpointOutcome::Advanced),
                }),
                branch: None,
                pr_ref: None,
                pr_status: None,
                pr_ready_at: None,
                pr_merged_at: None,
                verifier_nonce: None,
                started_at: Some("2026-05-31T00:00:00Z".into()),
                completed_at: Some("2026-05-31T01:00:00Z".into()),
            },
        );
        state.stations.insert(
            "specify".into(),
            Station {
                station: "specify".into(),
                status: Status::InProgress,
                phase: StationPhase::Manufacture,
            elaborated: false,
                checkpoint: None,
                branch: None,
                pr_ref: None,
                pr_status: None,
                pr_ready_at: None,
                pr_merged_at: None,
                verifier_nonce: None,
                started_at: Some("2026-05-31T01:00:00Z".into()),
                completed_at: None,
            },
        );
        store.write_state("r", &state).unwrap();

        let reg = registry();
        create_show(&reg, &store, "r").expect("show");

        let rev = match reg.get("r").expect("session present") {
            SessionPayload::Review(rev) => rev,
            other => panic!("expected a review session, got {other:?}"),
        };

        // The dead pipeline is fixed: the active phase is present and matches the
        // active station's recorded phase.
        let cur = rev.current_state.as_ref().expect("current_state set");
        assert_eq!(cur.station, "specify");
        assert_eq!(cur.phase, Some(RunPhase::Manufacture));
        assert_eq!(cur.factory, "software");

        // The strip carries every factory station, in factory order (not alphabetical).
        let order: Vec<&str> = rev.station_states.iter().map(|s| s.station.as_str()).collect();
        assert_eq!(order, ["frame", "specify", "shape", "build", "prove", "harden"]);
        assert_eq!(rev.station_states.len(), 6);

        // Completed station: merged, Completed status, Checkpoint phase, gate
        // metadata carried through.
        let frame = rev
            .station_states
            .iter()
            .find(|s| s.station == "frame")
            .expect("frame present");
        assert!(frame.merged_into_main);
        assert_eq!(frame.status.as_deref(), Some("completed"));
        assert_eq!(frame.phase.as_deref(), Some("checkpoint"));
        assert_eq!(frame.completed_at.as_deref(), Some("2026-05-31T01:00:00Z"));
        assert_eq!(frame.gate_entered_at.as_deref(), Some("2026-05-31T00:00:00Z"));
        assert_eq!(frame.gate_outcome.as_deref(), Some("advanced"));

        let by_name = |n: &str| {
            rev.station_states
                .iter()
                .find(|s| s.station == n)
                .unwrap_or_else(|| panic!("{n} present"))
        };

        // Active station: not merged, in-progress, Manufacture phase.
        let specify = by_name("specify");
        assert!(!specify.merged_into_main);
        assert_eq!(specify.status.as_deref(), Some("in_progress"));
        assert_eq!(specify.phase.as_deref(), Some("manufacture"));

        // A not-yet-reached station: pending, Spec phase, nothing started.
        let harden = by_name("harden");
        assert!(!harden.merged_into_main);
        assert_eq!(harden.status.as_deref(), Some("pending"));
        assert_eq!(harden.phase.as_deref(), Some("spec"));
        assert!(harden.started_at.is_none());
    }

    #[test]
    fn run_state_ordered_stations_prefers_plan_then_factory() {
        use darkrun_core::RunState;

        let factory = vec![
            "frame".to_string(),
            "specify".to_string(),
            "shape".to_string(),
        ];

        // Empty plan → the factory's full ordered list.
        let full = RunState::default();
        assert_eq!(full.ordered_stations(&factory), factory);

        // A recorded plan overrides the factory list.
        let sized = RunState {
            plan: vec!["build".to_string(), "prove".to_string()],
            ..Default::default()
        };
        assert_eq!(
            sized.ordered_stations(&factory),
            vec!["build".to_string(), "prove".to_string()]
        );
    }

    fn q_opt(id: &str, label: &str) -> QuestionOptionSpec {
        QuestionOptionSpec {
            id: id.into(),
            label: label.into(),
            image_url: None,
            image_url_light: None,
            description: None,
        }
    }

    #[test]
    fn create_show_surfaces_the_approval_gate_at_an_undecided_checkpoint() {
        use darkrun_core::domain::{Station, StationPhase, Status};

        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new(dir.path());
        crate::position::run_start(&store, "r", "software", None, Mode::Solo, "full").unwrap();

        // The active station sits at its Checkpoint phase, not yet decided.
        let mut state = store.read_state("r").unwrap().unwrap();
        state.active_station = "frame".into();
        state.stations.insert(
            "frame".into(),
            Station {
                station: "frame".into(),
                status: Status::InProgress,
                phase: StationPhase::Checkpoint,
                elaborated: false,
                checkpoint: None,
                branch: None,
                pr_ref: None,
                pr_status: None,
                pr_ready_at: None,
                pr_merged_at: None,
                verifier_nonce: None,
                started_at: Some("2026-06-01T00:00:00Z".into()),
                completed_at: None,
            },
        );
        store.write_state("r", &state).unwrap();

        let reg = registry();
        create_show(&reg, &store, "r").expect("show");
        let SessionPayload::Review(rev) = reg.get("r").expect("session present") else {
            panic!("expected a review session");
        };

        // Solo mode → ask gate; undecided checkpoint → the bar is active.
        assert_eq!(rev.await_active, Some(true));
        assert_eq!(rev.gate_type, Some(GateType::Ask));
        assert_eq!(rev.station.as_deref(), Some("frame"));
        assert!(rev.approve_action.is_some());

        // A dark-mode run at the same checkpoint advances on its own — no bar.
        crate::position::run_start(&store, "d", "software", None, Mode::Dark, "full").unwrap();
        let mut dstate = store.read_state("d").unwrap().unwrap();
        dstate.active_station = "frame".into();
        dstate.stations.insert(
            "frame".into(),
            Station {
                station: "frame".into(),
                status: Status::InProgress,
                phase: StationPhase::Checkpoint,
                elaborated: false,
                checkpoint: None,
                branch: None,
                pr_ref: None,
                pr_status: None,
                pr_ready_at: None,
                pr_merged_at: None,
                verifier_nonce: None,
                started_at: Some("2026-06-01T00:00:00Z".into()),
                completed_at: None,
            },
        );
        store.write_state("d", &dstate).unwrap();
        create_show(&reg, &store, "d").expect("show");
        let SessionPayload::Review(drev) = reg.get("d").expect("session present") else {
            panic!("expected a review session");
        };
        assert_eq!(drev.await_active, None);
        assert!(drev.approve_action.is_none());
    }

    #[test]
    fn create_show_surfaces_the_pre_execution_user_gate() {
        use darkrun_api::session::ApproveActionKind;
        use darkrun_core::domain::{Station, StationPhase, Status};

        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new(dir.path());
        crate::position::run_start(&store, "r", "software", None, Mode::Solo, "full").unwrap();

        // The active station sits at its PRE-execution operator gate (the spec is
        // reviewed; the operator clears it before any Unit is manufactured).
        let mut state = store.read_state("r").unwrap().unwrap();
        state.active_station = "frame".into();
        state.stations.insert(
            "frame".into(),
            Station {
                station: "frame".into(),
                status: Status::InProgress,
                phase: StationPhase::UserGate,
                elaborated: true,
                checkpoint: None,
                branch: None,
                pr_ref: None,
                pr_status: None,
                pr_ready_at: None,
                pr_merged_at: None,
                verifier_nonce: None,
                started_at: Some("2026-06-01T00:00:00Z".into()),
                completed_at: None,
            },
        );
        store.write_state("r", &state).unwrap();

        let reg = registry();
        create_show(&reg, &store, "r").expect("show");
        let SessionPayload::Review(rev) = reg.get("r").expect("session present") else {
            panic!("expected a review session");
        };

        // Solo mode → an ask gate is OPEN at the user gate, with a
        // start-execution approve action (it releases the wave, not "complete").
        assert_eq!(rev.await_active, Some(true), "the user gate surfaces the approval bar");
        assert_eq!(rev.gate_type, Some(GateType::Ask));
        assert_eq!(rev.station.as_deref(), Some("frame"));
        let approve = rev.approve_action.expect("an approve action");
        assert_eq!(approve.kind, ApproveActionKind::StartExecution);
        assert!(approve.label.contains("Start"), "label names execution: {}", approve.label);
        // The phase still reads as `review` in the universal taxonomy.
        assert_eq!(
            rev.current_state.and_then(|c| c.phase),
            Some(darkrun_api::session::RunPhase::Review),
        );
    }

    // ── question ─────────────────────────────────────────────────────────

    #[test]
    fn question_creates_pending_payload_in_registry() {
        let reg = registry();
        let res = create_question(
            &reg,
            "r",
            Some("Pick a hero".into()),
            "Which hero layout?",
            Some("context md".into()),
            vec![
                QuestionOptionSpec {
                    id: "a".into(),
                    label: "Option A".into(),
                    image_url: Some("https://img/a.png".into()),
                    image_url_light: None,
                    description: Some("bold".into()),
                },
                q_opt("b", "Option B"),
            ],
            false,
            vec!["https://ref/surface.png".into()],
        )
        .unwrap();

        assert_eq!(res.session_id, "q-01");
        assert_eq!(res.session_type, "question");
        assert!(res.awaiting_answer);
        assert_eq!(res.status, SessionStatus::Pending);
        assert_eq!(res.session_path, "/api/session/q-01");

        // Served straight out of the shared in-memory registry as a Question.
        let q = question_result(&reg, "r", "q-01").unwrap();
        assert_eq!(q.prompt, "Which hero layout?");
        assert_eq!(q.options.len(), 2);
        assert_eq!(q.options[0].image_url.as_deref(), Some("https://img/a.png"));
        assert_eq!(q.image_urls, vec!["https://ref/surface.png".to_string()]);
        assert!(!q.multi_select);
        assert!(q.answer.is_none());
        assert_eq!(q.status, SessionStatus::Pending);
    }

    #[test]
    fn question_requires_prompt_and_options() {
        let reg = registry();
        let err = create_question(&reg, "r", None, "  ", None, vec![q_opt("a", "A")], false, vec![])
            .unwrap_err();
        assert!(matches!(err, McpError::InvalidInput(_)));

        let err = create_question(&reg, "r", None, "prompt", None, vec![], false, vec![])
            .unwrap_err();
        assert!(matches!(err, McpError::InvalidInput(_)));
    }

    #[test]
    fn question_rejects_duplicate_and_empty_option_ids() {
        let reg = registry();
        let err = create_question(
            &reg,
            "r",
            None,
            "p",
            None,
            vec![q_opt("a", "A"), q_opt("a", "A2")],
            false,
            vec![],
        )
        .unwrap_err();
        assert!(matches!(err, McpError::InvalidInput(_)));

        let err = create_question(&reg, "r", None, "p", None, vec![q_opt("", "A")], false, vec![])
            .unwrap_err();
        assert!(matches!(err, McpError::InvalidInput(_)));
    }

    #[test]
    fn question_surfaces_submitted_answer() {
        let reg = registry();
        create_question(&reg, "r", None, "p", None, vec![q_opt("a", "A"), q_opt("b", "B")], true, vec![])
            .unwrap();

        // Simulate the HTTP handler writing an answer back onto the payload by
        // re-upserting the mutated session into the shared registry.
        if let Some(SessionPayload::Question(mut q)) = reg.get("q-01") {
            q.answer = Some(QuestionAnswer {
                selected: vec!["a".into(), "b".into()],
                text: Some("both work".into()),
            });
            q.status = SessionStatus::Answered;
            reg.upsert(SessionPayload::Question(q));
        }

        let q = question_result(&reg, "r", "q-01").unwrap();
        assert_eq!(q.status, SessionStatus::Answered);
        let ans = q.answer.unwrap();
        assert_eq!(ans.selected, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(ans.text.as_deref(), Some("both work"));
    }

    // ── direction ────────────────────────────────────────────────────────

    fn arch(id: &str) -> ArchetypeSpec {
        ArchetypeSpec {
            id: id.into(),
            label: format!("{id} label"),
            image_url: format!("https://img/{id}.png"),
            image_url_light: None,
            description: format!("{id} direction"),
        }
    }

    #[test]
    fn direction_creates_pending_payload_in_registry() {
        let reg = registry();
        let res = create_direction(
            &reg,
            "r",
            Some("Direction".into()),
            "Pick a direction",
            None,
            vec![arch("brutalist"), arch("editorial")],
        )
        .unwrap();
        assert_eq!(res.session_id, "d-01");
        assert_eq!(res.session_type, "direction");

        let d = direction_result(&reg, "r", "d-01").unwrap();
        assert_eq!(d.archetypes.len(), 2);
        assert_eq!(d.run_slug.as_deref(), Some("r"));
        assert!(d.chosen_archetype.is_none());
        assert_eq!(d.status, SessionStatus::Pending);
    }

    #[test]
    fn direction_requires_complete_archetypes() {
        let reg = registry();
        // empty prompt
        assert!(create_direction(&reg, "r", None, " ", None, vec![arch("a")]).is_err());
        // no archetypes
        assert!(create_direction(&reg, "r", None, "p", None, vec![]).is_err());
        // missing image_url
        let mut bad = arch("a");
        bad.image_url = "  ".into();
        assert!(create_direction(&reg, "r", None, "p", None, vec![bad]).is_err());
        // missing description
        let mut bad = arch("a");
        bad.description = String::new();
        assert!(create_direction(&reg, "r", None, "p", None, vec![bad]).is_err());
        // duplicate ids
        assert!(create_direction(&reg, "r", None, "p", None, vec![arch("a"), arch("a")]).is_err());
    }

    #[test]
    fn direction_surfaces_choice_and_annotations() {
        let reg = registry();
        create_direction(&reg, "r", None, "p", None, vec![arch("a"), arch("b")]).unwrap();

        if let Some(SessionPayload::Direction(mut d)) = reg.get("d-01") {
            d.chosen_archetype = Some("b".into());
            d.annotations = Some(DirectionAnnotations {
                pins: vec![DirectionPin {
                    x: 0.5,
                    y: 0.25,
                    note: "tighten header".into(),
                }],
                screenshot: Some("data:image/png;base64,AAAA".into()),
                comments: vec!["love it".into()],
            });
            d.status = SessionStatus::Decided;
            reg.upsert(SessionPayload::Direction(d));
        }

        let d = direction_result(&reg, "r", "d-01").unwrap();
        assert_eq!(d.chosen_archetype.as_deref(), Some("b"));
        assert_eq!(d.status, SessionStatus::Decided);
        let ann = d.annotations.unwrap();
        assert_eq!(ann.pins.len(), 1);
        assert_eq!(ann.pins[0].note, "tighten header");
        assert_eq!(ann.comments, vec!["love it".to_string()]);
    }

    // ── picker ───────────────────────────────────────────────────────────

    fn p_opt(id: &str) -> PickerOptionSpec {
        PickerOptionSpec {
            id: id.into(),
            label: format!("{id} label"),
            description: None,
            secondary: None,
        }
    }

    #[test]
    fn picker_creates_pending_payload_in_registry() {
        let reg = registry();
        let res = create_picker(
            &reg,
            "r",
            PickerKind::Factory,
            "Pick a factory",
            "which factory?",
            vec![p_opt("software"), p_opt("design")],
        )
        .unwrap();
        assert_eq!(res.session_id, "p-01");
        assert_eq!(res.session_type, "picker");

        let p = picker_result(&reg, "r", "p-01").unwrap();
        assert_eq!(p.kind, PickerKind::Factory);
        assert_eq!(p.options.len(), 2);
        assert!(p.selection.is_none());
    }

    #[test]
    fn picker_validates_title_prompt_and_options() {
        let reg = registry();
        assert!(create_picker(&reg, "r", PickerKind::Mode, " ", "p", vec![p_opt("a")]).is_err());
        assert!(create_picker(&reg, "r", PickerKind::Mode, "t", " ", vec![p_opt("a")]).is_err());
        assert!(create_picker(&reg, "r", PickerKind::Mode, "t", "p", vec![]).is_err());
        assert!(create_picker(
            &reg,
            "r",
            PickerKind::Mode,
            "t",
            "p",
            vec![p_opt("a"), p_opt("a")]
        )
        .is_err());
    }

    #[test]
    fn picker_surfaces_selection() {
        let reg = registry();
        create_picker(&reg, "r", PickerKind::Station, "t", "p", vec![p_opt("frame"), p_opt("shape")])
            .unwrap();

        if let Some(SessionPayload::Picker(mut p)) = reg.get("p-01") {
            p.selection = Some(PickerSelection { id: "shape".into() });
            p.status = SessionStatus::Decided;
            reg.upsert(SessionPayload::Picker(p));
        }

        let p = picker_result(&reg, "r", "p-01").unwrap();
        assert_eq!(p.selection.unwrap().id, "shape");
        assert_eq!(p.status, SessionStatus::Decided);
    }

    #[test]
    fn parse_picker_kind_covers_aliases() {
        assert_eq!(parse_picker_kind("factory"), Some(PickerKind::Factory));
        assert_eq!(parse_picker_kind("MODE"), Some(PickerKind::Mode));
        assert_eq!(parse_picker_kind("url"), Some(PickerKind::UrlInput));
        assert_eq!(parse_picker_kind("url_input"), Some(PickerKind::UrlInput));
        assert!(parse_picker_kind("telepathy").is_none());
    }

    // ── registry behaviour ───────────────────────────────────────────────

    #[test]
    fn ids_increment_across_kinds_and_calls() {
        let reg = registry();
        create_question(&reg, "r", None, "p", None, vec![q_opt("a", "A")], false, vec![]).unwrap();
        create_question(&reg, "r", None, "p", None, vec![q_opt("a", "A")], false, vec![]).unwrap();
        create_direction(&reg, "r", None, "p", None, vec![arch("a")]).unwrap();
        create_picker(&reg, "r", PickerKind::Confirm, "t", "p", vec![p_opt("a")]).unwrap();

        assert!(reg.get("q-01").is_some());
        assert!(reg.get("q-02").is_some());
        assert!(reg.get("d-01").is_some());
        assert!(reg.get("p-01").is_some());
    }

    /// Gaps #14/#15: persisted station briefs (pre) and outcomes (post) surface
    /// in the review payload — no longer dead, test-only wiring.
    #[test]
    fn create_show_surfaces_persisted_briefs_and_outcomes() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new(dir.path());
        crate::position::run_start(&store, "r", "software", None, Mode::Solo, "full").unwrap();
        crate::brief::record(&store, "r", "frame", crate::brief::BriefPhase::Pre, "framing the problem").unwrap();
        crate::brief::record(&store, "r", "frame", crate::brief::BriefPhase::Post, "frame.md locked").unwrap();

        let reg = registry();
        create_show(&reg, &store, "r").unwrap();
        let SessionPayload::Review(payload) = reg.get("r").unwrap() else {
            panic!("expected a review payload");
        };
        assert_eq!(payload.station_briefs.get("frame").map(String::as_str), Some("framing the problem"));
        assert_eq!(payload.station_outcomes.get("frame").map(String::as_str), Some("frame.md locked"));
    }

    #[test]
    fn result_readers_reject_wrong_kind_and_missing() {
        let reg = registry();
        let q = create_question(&reg, "r", None, "p", None, vec![q_opt("a", "A")], false, vec![]).unwrap();
        let d = create_direction(&reg, "r", None, "p", None, vec![arch("x")]).unwrap();
        let pk = create_picker(&reg, "r", PickerKind::Confirm, "t", "p", vec![p_opt("y")]).unwrap();

        // Each reader rejects a session of the WRONG kind…
        assert!(picker_result(&reg, "r", &q.session_id).is_err());
        assert!(question_result(&reg, "r", &d.session_id).is_err());
        assert!(direction_result(&reg, "r", &pk.session_id).is_err());
        // …and a MISSING id.
        assert!(question_result(&reg, "r", "q-99").is_err());
        assert!(direction_result(&reg, "r", "d-99").is_err());
        assert!(picker_result(&reg, "r", "pk-99").is_err());
    }

    #[test]
    fn builders_reject_empty_labels_and_ids_across_session_types() {
        let reg = registry();
        // Question option with an empty LABEL.
        let q = QuestionOptionSpec { id: "a".into(), label: "  ".into(), image_url: None, image_url_light: None, description: None };
        assert!(create_question(&reg, "r", None, "p", None, vec![q], false, vec![]).is_err());

        // Direction archetype with an empty id, then an empty label.
        let blank_id = ArchetypeSpec { id: " ".into(), label: "L".into(), image_url: "u".into(), image_url_light: None, description: "d".into() };
        assert!(create_direction(&reg, "r", None, "p", None, vec![blank_id]).is_err());
        let blank_label = ArchetypeSpec { id: "x".into(), label: " ".into(), image_url: "u".into(), image_url_light: None, description: "d".into() };
        assert!(create_direction(&reg, "r", None, "p", None, vec![blank_label]).is_err());

        // Picker option with an empty id, then an empty label.
        let p_blank_id = PickerOptionSpec { id: " ".into(), label: "L".into(), description: None, secondary: None };
        assert!(create_picker(&reg, "r", PickerKind::Mode, "t", "p", vec![p_blank_id]).is_err());
        let p_blank_label = PickerOptionSpec { id: "x".into(), label: " ".into(), description: None, secondary: None };
        assert!(create_picker(&reg, "r", PickerKind::Mode, "t", "p", vec![p_blank_label]).is_err());
    }

    #[test]
    fn run_phase_maps_audit_and_reflect() {
        assert_eq!(run_phase(StationPhase::Audit), RunPhase::Audit);
        assert_eq!(run_phase(StationPhase::Reflect), RunPhase::Reflect);
        assert_eq!(run_phase(StationPhase::UserGate), RunPhase::Review);
    }
}
