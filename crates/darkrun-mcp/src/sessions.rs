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

use darkrun_api::session::{
    DirectionArchetype, DirectionSessionPayload, PickerKind, PickerOption, PickerSessionPayload,
    QuestionOption, QuestionSessionPayload, SessionPayload,
};
use darkrun_api::SessionStatus;
use serde::Serialize;

use crate::error::{McpError, Result};

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
            description: opt.description.filter(|s| !s.trim().is_empty()),
        });
    }

    let session_id = registry.next_session_id("q");
    let payload = QuestionSessionPayload {
        session_id: session_id.clone(),
        status: SessionStatus::Pending,
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
    registry.upsert(SessionPayload::Question(payload));

    Ok(awaiting(run, &session_id, "question"))
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
    registry.upsert(SessionPayload::Direction(payload));

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
    registry.upsert(SessionPayload::Picker(payload));

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
    use darkrun_api::session::{
        DirectionAnnotations, DirectionPin, PickerSelection, QuestionAnswer,
    };

    fn registry() -> SessionRegistry {
        SessionRegistry::new()
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

    #[test]
    fn result_readers_reject_wrong_kind_and_missing() {
        let reg = registry();
        create_question(&reg, "r", None, "p", None, vec![q_opt("a", "A")], false, vec![]).unwrap();
        // q-01 is a question, asking for it as a picker fails.
        assert!(picker_result(&reg, "r", "q-01").is_err());
        // missing id fails.
        assert!(question_result(&reg, "r", "q-99").is_err());
    }
}
