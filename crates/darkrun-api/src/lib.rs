//! darkrun-api — the shared wire contract between the darkrun engine, the
//! HTTP/WS server, and the desktop review app.
//!
//! Defines `serde` + `schemars` types for the interactive session payloads as
//! a discriminated union tagged on `session_type`
//! (`review | question | direction | picker | view`), the review-decision,
//! question-answer, direction/picker-select, advance, and feedback-CRUD
//! request/response bodies, a route descriptor table, and an OpenAPI emitter —
//! all in the factory vocabulary.
//!
//! Dependency-light by design: only `serde`, `serde_json`, and `schemars`.
//! Opaque parser output (parsed run/unit/criteria structures) is carried as raw
//! [`serde_json::Value`]s rather than schematized here.

pub mod advance;
pub mod common;
pub mod direction;
pub mod feedback;
pub mod openapi;
pub mod question;
pub mod review;
pub mod review_current;
pub mod routes;
pub mod runs;
pub mod session;

pub use advance::AdvanceResponse;
pub use common::{
    AuthorType, FeedbackOrigin, FeedbackReply, FeedbackResolution, FeedbackSeverity,
    FeedbackStatus, GateType, InlineComment, Pin, QuestionAnnotations, QuestionPin,
    QuestionScreenshotAnnotation, ReviewAnnotations, SessionStatus, SessionType,
    ValidationError, ValidationIssue,
};
pub use direction::{
    DirectionAnnotations, DirectionSelectRequest, DirectionSelectResponse,
    DirectionUploadFile, PickerSelectRequest, PickerSelectResponse,
};
pub use feedback::{
    ClosureReply, FeedbackAnchor, FeedbackCreateRequest, FeedbackCreateResponse,
    FeedbackDeleteResponse, FeedbackInlineAnchor, FeedbackItem, FeedbackIteration,
    FeedbackListResponse, FeedbackReplyCreateRequest, FeedbackReplyCreateResponse,
    FeedbackScope, FeedbackUpdateRequest, FeedbackUpdateResponse, IterationResult,
};
pub use question::{QuestionAnswerItem, QuestionAnswerRequest, QuestionAnswerResponse};
pub use review::{ReviewDecision, ReviewDecisionRequest, ReviewDecisionResponse};
pub use review_current::{
    FeedbackSummary, ReviewCurrentPayload, ReviewCurrentStation, ReviewCurrentUnit,
};
pub use routes::{HttpMethod, RouteSpec, ROUTES};
pub use runs::{
    RunDetailPayload, RunDetailStation, RunDetailUnit, RunListPayload, RunSummary, StationProgress,
};
pub use session::{
    ApproveAction, ApproveActionKind, DirectionArchetype, DirectionSelection,
    DirectionSessionPayload, DiscoveredReviewUrl, DriftAction, DriftEntry, DriftKind,
    KnowledgeFile, MilestoneStatus, OutputArtifact, OutputArtifactType, PendingDecision,
    PickerKind, PickerOption, PickerSelection, PickerSessionPayload, PreviousReviewSnapshot,
    ProgressMilestone, QuestionAnswer, QuestionDef, QuestionSessionPayload,
    ReviewSessionPayload, RunCurrentState, RunPhase, SealStatus, SessionPayload,
    StationArtifact, StationStateInfo, UnitOutputPreview, UnitOutputType, ViewMode,
    ViewSessionPayload, ViewStatus,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::ReviewSessionPayload;

    #[test]
    fn review_payload_roundtrips_with_discriminator() {
        let payload = SessionPayload::Review(ReviewSessionPayload {
            session_id: "s-1".into(),
            status: SessionStatus::Pending,
            run_slug: Some("my-run".into()),
            gate_type: Some(GateType::Ask),
            station: Some("frame".into()),
            current_state: Some(RunCurrentState {
                factory: "software".into(),
                station: "frame".into(),
                phase: Some(RunPhase::Checkpoint),
                ..Default::default()
            }),
            approve_action: Some(ApproveAction {
                label: "Complete Frame Station".into(),
                kind: ApproveActionKind::CompleteStation,
            }),
            await_active: Some(true),
            ..Default::default()
        });

        let json = serde_json::to_value(&payload).expect("serialize");
        assert_eq!(json["session_type"], "review");
        assert_eq!(json["session_id"], "s-1");
        assert_eq!(json["gate_type"], "ask");

        let back: SessionPayload = serde_json::from_value(json).expect("deserialize");
        assert_eq!(back.session_type(), "review");
        assert_eq!(back.session_id(), "s-1");
    }

    /// Every session variant round-trips through serde and reports the right
    /// discriminator + session id.
    #[test]
    fn every_session_variant_roundtrips() {
        let cases: Vec<(SessionPayload, &str)> = vec![
            (
                SessionPayload::Review(ReviewSessionPayload {
                    session_id: "r".into(),
                    ..Default::default()
                }),
                "review",
            ),
            (
                SessionPayload::Question(QuestionSessionPayload {
                    session_id: "q".into(),
                    ..Default::default()
                }),
                "question",
            ),
            (
                SessionPayload::Direction(DirectionSessionPayload {
                    session_id: "d".into(),
                    ..Default::default()
                }),
                "direction",
            ),
            (
                SessionPayload::Picker(PickerSessionPayload {
                    session_id: "p".into(),
                    status: SessionStatus::Pending,
                    run_slug: None,
                    kind: PickerKind::Factory,
                    title: "pick a factory".into(),
                    prompt: "which one?".into(),
                    options: vec![PickerOption {
                        id: "software".into(),
                        label: "Software".into(),
                        description: None,
                        secondary: None,
                    }],
                    selection: None,
                }),
                "picker",
            ),
            (
                SessionPayload::View(ViewSessionPayload {
                    session_id: "v".into(),
                    status: ViewStatus::Open,
                    run_slug: "run".into(),
                    factory: None,
                    station: None,
                    artifact: None,
                    mode: ViewMode::Viewer,
                    boot_port: None,
                    boot_command: None,
                }),
                "view",
            ),
        ];

        for (payload, expected_type) in cases {
            let json = serde_json::to_value(&payload).expect("serialize");
            assert_eq!(json["session_type"], expected_type);
            let back: SessionPayload =
                serde_json::from_value(json).expect("deserialize");
            assert_eq!(back.session_type(), expected_type);
        }
    }

    /// The discriminated union refuses an unknown `session_type`.
    #[test]
    fn unknown_session_type_is_rejected() {
        let json = serde_json::json!({
            "session_type": "telepathy",
            "session_id": "x"
        });
        let parsed: Result<SessionPayload, _> = serde_json::from_value(json);
        assert!(parsed.is_err(), "unknown session_type must not parse");
    }

    /// The discriminated union refuses a payload with no `session_type` tag.
    #[test]
    fn missing_discriminator_is_rejected() {
        let json = serde_json::json!({ "session_id": "x", "status": "pending" });
        let parsed: Result<SessionPayload, _> = serde_json::from_value(json);
        assert!(parsed.is_err(), "missing session_type must not parse");
    }

    #[test]
    fn direction_select_request_is_discriminated_on_mode() {
        let select = DirectionSelectRequest::Select {
            archetype: "brutalist".into(),
            comments: Some("love it".into()),
            annotations: None,
        };
        let json = serde_json::to_value(&select).unwrap();
        assert_eq!(json["mode"], "select");
        assert_eq!(json["archetype"], "brutalist");

        let upload = serde_json::json!({
            "mode": "upload",
            "files": [{ "filename": "a.png", "data_url": "data:image/png;base64,AA" }]
        });
        let back: DirectionSelectRequest = serde_json::from_value(upload).unwrap();
        assert!(matches!(back, DirectionSelectRequest::Upload { .. }));
    }

    #[test]
    fn decision_canonicalizes() {
        assert_eq!(
            ReviewDecision::canonicalize("approved"),
            ReviewDecision::Approved
        );
        assert_eq!(
            ReviewDecision::canonicalize("APPROVED"),
            ReviewDecision::Approved
        );
        assert_eq!(
            ReviewDecision::canonicalize("nope"),
            ReviewDecision::ChangesRequested
        );
    }

    #[test]
    fn routes_lookup_resolves() {
        let r = routes::find(HttpMethod::Get, "/health").expect("health route");
        assert_eq!(r.operation_id, "getHealth");
        assert_eq!(routes::paths::session("abc"), "/api/session/abc");
        assert_eq!(routes::paths::ws_session("abc"), "/ws/session/abc");
        assert_eq!(
            routes::paths::feedback_item("run", "frame", "FB-01"),
            "/api/feedback/run/frame/FB-01"
        );
    }

    #[test]
    fn runs_routes_resolve() {
        let list = routes::find(HttpMethod::Get, "/api/runs").expect("runs list route");
        assert_eq!(list.operation_id, "listRuns");
        assert_eq!(list.tag, "runs");
        let detail = routes::find(HttpMethod::Get, "/api/runs/{slug}").expect("run detail route");
        assert_eq!(detail.operation_id, "getRun");
        assert_eq!(routes::paths::runs(), "/api/runs");
        assert_eq!(routes::paths::run_detail("alpha"), "/api/runs/alpha");
    }

    /// Every route in the table has a unique operation id.
    #[test]
    fn route_operation_ids_are_unique() {
        let mut ids: Vec<&str> = ROUTES.iter().map(|r| r.operation_id).collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "duplicate operationId in ROUTES");
    }

    /// JSON Schema generation succeeds for the union and carries a title.
    #[test]
    fn session_payload_schema_generates() {
        let schema = schemars::schema_for!(SessionPayload);
        let json = serde_json::to_value(&schema).expect("schema serializes");
        assert!(json.is_object());
        assert_eq!(json["title"], "SessionPayload");
    }

    /// The OpenAPI document round-trips through serde_json and is non-empty.
    #[test]
    fn openapi_document_emits() {
        let text = openapi::document_json();
        assert!(text.contains("\"openapi\""));
        assert!(text.contains("darkrun API"));
        let _: serde_json::Value = serde_json::from_str(&text).expect("valid json");
    }
}
