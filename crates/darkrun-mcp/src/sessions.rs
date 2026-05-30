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
//! The MCP server (this crate) and the HTTP/WS server are decoupled: they meet
//! on disk. Every visual session is persisted into the run's
//! `.darkrun/<run>/session.json` registry — a `session_id -> SessionPayload`
//! map — via [`StateStore::write_session`]. The HTTP server reads the same file
//! to serve `GET /api/session/:id` and pushes live updates over
//! `GET /ws/session/:id`. When the operator answers, the HTTP handler records
//! the answer/selection back onto the payload; the agent reads it back here via
//! [`question_result`] / [`direction_result`] / [`picker_result`] (or the
//! `darkrun_*_result` tools).
//!
//! The registry is keyed by `session_id` so a single run can hold several
//! concurrent visual sessions without clobbering each other.

use std::collections::BTreeMap;

use chrono::Utc;
use darkrun_api::session::{
    DirectionArchetype, DirectionSessionPayload, PickerKind, PickerOption, PickerSessionPayload,
    QuestionOption, QuestionSessionPayload, SessionPayload,
};
use darkrun_api::SessionStatus;
use darkrun_core::StateStore;
use serde::Serialize;

use crate::error::{McpError, Result};

/// The on-disk session registry for one run: a `session_id -> SessionPayload`
/// map serialized into `.darkrun/<run>/session.json`.
///
/// Persisting a keyed map (rather than a single payload) lets a run carry
/// several concurrent visual sessions, and lets the result-reader tools find a
/// specific session by id.
#[derive(Debug, Default)]
pub struct SessionRegistry {
    /// The registered sessions, keyed by `session_id`.
    pub sessions: BTreeMap<String, SessionPayload>,
}

impl SessionRegistry {
    /// Load the registry from a run's `session.json`, returning an empty
    /// registry when the file is absent or holds an empty object.
    ///
    /// Tolerant of a legacy single-payload `session.json`: a bare object that
    /// parses as a [`SessionPayload`] is adopted under its own session id.
    pub fn load(store: &StateStore, run: &str) -> Result<Self> {
        let Some(value) = store.read_session(run)? else {
            return Ok(Self::default());
        };
        if value.is_null() {
            return Ok(Self::default());
        }
        // Preferred shape: a map of session_id -> SessionPayload.
        if let Ok(map) = serde_json::from_value::<BTreeMap<String, SessionPayload>>(value.clone()) {
            return Ok(Self { sessions: map });
        }
        // Legacy/tolerant shape: a single bare SessionPayload.
        if let Ok(single) = serde_json::from_value::<SessionPayload>(value) {
            let mut sessions = BTreeMap::new();
            sessions.insert(single.session_id().to_string(), single);
            return Ok(Self { sessions });
        }
        // An object we don't recognize: treat as empty rather than failing the
        // whole tool, so a stray file can't wedge the agent.
        Ok(Self::default())
    }

    /// Persist the registry back to the run's `session.json`.
    pub fn save(&self, store: &StateStore, run: &str) -> Result<()> {
        let value = serde_json::to_value(&self.sessions)?;
        store.write_session(run, &value)?;
        Ok(())
    }

    /// Insert or replace a session by its id.
    pub fn upsert(&mut self, payload: SessionPayload) {
        self.sessions
            .insert(payload.session_id().to_string(), payload);
    }

    /// Fetch a session payload by id.
    pub fn get(&self, session_id: &str) -> Option<&SessionPayload> {
        self.sessions.get(session_id)
    }
}

/// Mint the next session id for a run + kind prefix (`q`/`d`/`p`), scanning the
/// existing registry so ids stay unique and stable within a run.
fn next_session_id(registry: &SessionRegistry, prefix: &str) -> String {
    let want = format!("{prefix}-");
    let max = registry
        .sessions
        .keys()
        .filter_map(|k| k.strip_prefix(&want))
        .filter_map(|n| n.parse::<u32>().ok())
        .max()
        .unwrap_or(0);
    format!("{prefix}-{:02}", max + 1)
}

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
    /// Optional generated-image URL.
    pub image_url: Option<String>,
    /// Optional longer description.
    pub description: Option<String>,
}

/// Build + register a VISUAL QUESTION session and return its awaiting handle.
///
/// Validation: the prompt must be non-empty; at least one option is required;
/// option ids must be non-empty and unique.
#[allow(clippy::too_many_arguments)]
pub fn create_question(
    store: &StateStore,
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
            description: opt.description.filter(|s| !s.trim().is_empty()),
        });
    }

    let mut registry = SessionRegistry::load(store, run)?;
    let session_id = next_session_id(&registry, "q");
    let payload = QuestionSessionPayload {
        session_id: session_id.clone(),
        status: SessionStatus::Pending,
        title,
        prompt: prompt.trim().to_string(),
        context,
        options: built,
        multi_select,
        answer: None,
        image_urls: image_urls.into_iter().filter(|u| !u.trim().is_empty()).collect(),
    };
    registry.upsert(SessionPayload::Question(payload));
    registry.save(store, run)?;

    Ok(awaiting(run, &session_id, "question"))
}

/// Read back the answer to a question session, if the operator has submitted
/// one. Returns the whole [`QuestionSessionPayload`] so the caller sees both
/// the answer and the current `status`.
pub fn question_result(
    store: &StateStore,
    run: &str,
    session_id: &str,
) -> Result<QuestionSessionPayload> {
    let registry = SessionRegistry::load(store, run)?;
    match registry.get(session_id) {
        Some(SessionPayload::Question(q)) => Ok(q.clone()),
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
    /// Description of the design direction.
    pub description: String,
}

/// Build + register a DESIGN DIRECTION session and return its awaiting handle.
///
/// Validation: the prompt must be non-empty; at least one archetype is
/// required; every archetype needs a non-empty id, label, image_url, and
/// description; ids must be unique.
pub fn create_direction(
    store: &StateStore,
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
            description: arch.description,
        });
    }

    let mut registry = SessionRegistry::load(store, run)?;
    let session_id = next_session_id(&registry, "d");
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
    registry.upsert(SessionPayload::Direction(payload));
    registry.save(store, run)?;

    Ok(awaiting(run, &session_id, "direction"))
}

/// Read back the chosen archetype + annotations for a direction session.
pub fn direction_result(
    store: &StateStore,
    run: &str,
    session_id: &str,
) -> Result<DirectionSessionPayload> {
    let registry = SessionRegistry::load(store, run)?;
    match registry.get(session_id) {
        Some(SessionPayload::Direction(d)) => Ok(d.clone()),
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
    store: &StateStore,
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

    let mut registry = SessionRegistry::load(store, run)?;
    let session_id = next_session_id(&registry, "p");
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
    registry.upsert(SessionPayload::Picker(payload));
    registry.save(store, run)?;

    Ok(awaiting(run, &session_id, "picker"))
}

/// Read back the selection for a picker session.
pub fn picker_result(
    store: &StateStore,
    run: &str,
    session_id: &str,
) -> Result<PickerSessionPayload> {
    let registry = SessionRegistry::load(store, run)?;
    match registry.get(session_id) {
        Some(SessionPayload::Picker(p)) => Ok(p.clone()),
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

/// Mark a session's submitted-at clock — used only by tests that simulate the
/// HTTP handler writing an answer back. Kept here so the simulation path lives
/// next to the readers it feeds.
#[doc(hidden)]
pub fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;
    use darkrun_api::session::{
        DirectionAnnotations, DirectionPin, PickerSelection, QuestionAnswer,
    };
    use tempfile::tempdir;

    fn store() -> (tempfile::TempDir, StateStore) {
        let dir = tempdir().expect("tmp");
        let store = StateStore::new(dir.path());
        // Ensure the run dir exists so write_session can create session.json.
        std::fs::create_dir_all(store.run_dir("r")).unwrap();
        (dir, store)
    }

    fn q_opt(id: &str, label: &str) -> QuestionOptionSpec {
        QuestionOptionSpec {
            id: id.into(),
            label: label.into(),
            image_url: None,
            description: None,
        }
    }

    // ── question ─────────────────────────────────────────────────────────

    #[test]
    fn question_creates_pending_payload_on_disk() {
        let (_d, store) = store();
        let res = create_question(
            &store,
            "r",
            Some("Pick a hero".into()),
            "Which hero layout?",
            Some("context md".into()),
            vec![
                QuestionOptionSpec {
                    id: "a".into(),
                    label: "Option A".into(),
                    image_url: Some("https://img/a.png".into()),
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

        // Round-trips off disk as a Question payload with both options.
        let q = question_result(&store, "r", "q-01").unwrap();
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
        let (_d, store) = store();
        let err = create_question(&store, "r", None, "  ", None, vec![q_opt("a", "A")], false, vec![])
            .unwrap_err();
        assert!(matches!(err, McpError::InvalidInput(_)));

        let err = create_question(&store, "r", None, "prompt", None, vec![], false, vec![])
            .unwrap_err();
        assert!(matches!(err, McpError::InvalidInput(_)));
    }

    #[test]
    fn question_rejects_duplicate_and_empty_option_ids() {
        let (_d, store) = store();
        let err = create_question(
            &store,
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

        let err = create_question(&store, "r", None, "p", None, vec![q_opt("", "A")], false, vec![])
            .unwrap_err();
        assert!(matches!(err, McpError::InvalidInput(_)));
    }

    #[test]
    fn question_surfaces_submitted_answer() {
        let (_d, store) = store();
        create_question(&store, "r", None, "p", None, vec![q_opt("a", "A"), q_opt("b", "B")], true, vec![])
            .unwrap();

        // Simulate the HTTP handler writing an answer back onto the payload.
        let mut reg = SessionRegistry::load(&store, "r").unwrap();
        if let Some(SessionPayload::Question(q)) = reg.sessions.get_mut("q-01") {
            q.answer = Some(QuestionAnswer {
                selected: vec!["a".into(), "b".into()],
                text: Some("both work".into()),
            });
            q.status = SessionStatus::Answered;
        }
        reg.save(&store, "r").unwrap();

        let q = question_result(&store, "r", "q-01").unwrap();
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
            description: format!("{id} direction"),
        }
    }

    #[test]
    fn direction_creates_pending_payload_on_disk() {
        let (_d, store) = store();
        let res = create_direction(
            &store,
            "r",
            Some("Direction".into()),
            "Pick a direction",
            None,
            vec![arch("brutalist"), arch("editorial")],
        )
        .unwrap();
        assert_eq!(res.session_id, "d-01");
        assert_eq!(res.session_type, "direction");

        let d = direction_result(&store, "r", "d-01").unwrap();
        assert_eq!(d.archetypes.len(), 2);
        assert_eq!(d.run_slug.as_deref(), Some("r"));
        assert!(d.chosen_archetype.is_none());
        assert_eq!(d.status, SessionStatus::Pending);
    }

    #[test]
    fn direction_requires_complete_archetypes() {
        let (_d, store) = store();
        // empty prompt
        assert!(create_direction(&store, "r", None, " ", None, vec![arch("a")]).is_err());
        // no archetypes
        assert!(create_direction(&store, "r", None, "p", None, vec![]).is_err());
        // missing image_url
        let mut bad = arch("a");
        bad.image_url = "  ".into();
        assert!(create_direction(&store, "r", None, "p", None, vec![bad]).is_err());
        // missing description
        let mut bad = arch("a");
        bad.description = String::new();
        assert!(create_direction(&store, "r", None, "p", None, vec![bad]).is_err());
        // duplicate ids
        assert!(create_direction(&store, "r", None, "p", None, vec![arch("a"), arch("a")]).is_err());
    }

    #[test]
    fn direction_surfaces_choice_and_annotations() {
        let (_d, store) = store();
        create_direction(&store, "r", None, "p", None, vec![arch("a"), arch("b")]).unwrap();

        let mut reg = SessionRegistry::load(&store, "r").unwrap();
        if let Some(SessionPayload::Direction(d)) = reg.sessions.get_mut("d-01") {
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
        }
        reg.save(&store, "r").unwrap();

        let d = direction_result(&store, "r", "d-01").unwrap();
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
    fn picker_creates_pending_payload_on_disk() {
        let (_d, store) = store();
        let res = create_picker(
            &store,
            "r",
            PickerKind::Factory,
            "Pick a factory",
            "which factory?",
            vec![p_opt("software"), p_opt("design")],
        )
        .unwrap();
        assert_eq!(res.session_id, "p-01");
        assert_eq!(res.session_type, "picker");

        let p = picker_result(&store, "r", "p-01").unwrap();
        assert_eq!(p.kind, PickerKind::Factory);
        assert_eq!(p.options.len(), 2);
        assert!(p.selection.is_none());
    }

    #[test]
    fn picker_validates_title_prompt_and_options() {
        let (_d, store) = store();
        assert!(create_picker(&store, "r", PickerKind::Mode, " ", "p", vec![p_opt("a")]).is_err());
        assert!(create_picker(&store, "r", PickerKind::Mode, "t", " ", vec![p_opt("a")]).is_err());
        assert!(create_picker(&store, "r", PickerKind::Mode, "t", "p", vec![]).is_err());
        assert!(create_picker(
            &store,
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
        let (_d, store) = store();
        create_picker(&store, "r", PickerKind::Station, "t", "p", vec![p_opt("frame"), p_opt("shape")])
            .unwrap();

        let mut reg = SessionRegistry::load(&store, "r").unwrap();
        if let Some(SessionPayload::Picker(p)) = reg.sessions.get_mut("p-01") {
            p.selection = Some(PickerSelection { id: "shape".into() });
            p.status = SessionStatus::Decided;
        }
        reg.save(&store, "r").unwrap();

        let p = picker_result(&store, "r", "p-01").unwrap();
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
        let (_d, store) = store();
        create_question(&store, "r", None, "p", None, vec![q_opt("a", "A")], false, vec![]).unwrap();
        create_question(&store, "r", None, "p", None, vec![q_opt("a", "A")], false, vec![]).unwrap();
        create_direction(&store, "r", None, "p", None, vec![arch("a")]).unwrap();
        create_picker(&store, "r", PickerKind::Confirm, "t", "p", vec![p_opt("a")]).unwrap();

        let reg = SessionRegistry::load(&store, "r").unwrap();
        assert!(reg.get("q-01").is_some());
        assert!(reg.get("q-02").is_some());
        assert!(reg.get("d-01").is_some());
        assert!(reg.get("p-01").is_some());
        assert_eq!(reg.sessions.len(), 4);
    }

    #[test]
    fn result_readers_reject_wrong_kind_and_missing() {
        let (_d, store) = store();
        create_question(&store, "r", None, "p", None, vec![q_opt("a", "A")], false, vec![]).unwrap();
        // q-01 is a question, asking for it as a picker fails.
        assert!(picker_result(&store, "r", "q-01").is_err());
        // missing id fails.
        assert!(question_result(&store, "r", "q-99").is_err());
    }

    #[test]
    fn load_tolerates_legacy_single_payload() {
        let (_d, store) = store();
        // Write a bare single SessionPayload (legacy shape).
        let single = SessionPayload::Picker(PickerSessionPayload {
            session_id: "p-07".into(),
            status: SessionStatus::Pending,
            run_slug: Some("r".into()),
            kind: PickerKind::Confirm,
            title: "t".into(),
            prompt: "p".into(),
            options: vec![PickerOption {
                id: "yes".into(),
                label: "Yes".into(),
                description: None,
                secondary: None,
            }],
            selection: None,
        });
        store
            .write_session("r", &serde_json::to_value(&single).unwrap())
            .unwrap();

        let reg = SessionRegistry::load(&store, "r").unwrap();
        assert!(reg.get("p-07").is_some());
    }

    #[test]
    fn load_tolerates_empty_and_unrecognized() {
        let (_d, store) = store();
        // Empty object → empty registry.
        store.write_session("r", &serde_json::json!({})).unwrap();
        assert_eq!(SessionRegistry::load(&store, "r").unwrap().sessions.len(), 0);
        // Null → empty registry.
        store.write_session("r", &serde_json::Value::Null).unwrap();
        assert_eq!(SessionRegistry::load(&store, "r").unwrap().sessions.len(), 0);
        // Unrecognized scalar → empty registry (doesn't wedge the tool).
        store.write_session("r", &serde_json::json!("garbage")).unwrap();
        assert_eq!(SessionRegistry::load(&store, "r").unwrap().sessions.len(), 0);
    }

    #[test]
    fn now_rfc3339_parses() {
        let s = now_rfc3339();
        assert!(chrono::DateTime::parse_from_rfc3339(&s).is_ok());
    }
}
