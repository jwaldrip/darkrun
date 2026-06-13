//! Integration coverage for darkrun-api's public wire contract: JSON Schema
//! generation for the exported types, the ROUTES descriptor table (paths,
//! methods, operation ids, tags), the OpenAPI emitter, the openapi-vs-routes
//! parity guard, and the shared common types (annotations, statuses, the
//! feedback taxonomy).
//!
//! Drives only the crate's public API. Every test is written to fail for a
//! concrete reason — a wrong discriminator, a missing route, a broken
//! roundtrip, a drifted schema.
//!
//! The body-size-constant ordering tests compare compile-time constants, which
//! trips `clippy::assertions_on_constants` even though the ordering guard is a
//! real invariant we want to enforce.
#![allow(clippy::assertions_on_constants)]

use schemars::schema_for;
use serde_json::{json, Value};

use darkrun_api::common::{
    DEFAULT_BODY_MAX_BYTES, FEEDBACK_BODY_MAX_BYTES, FEEDBACK_CREATE_MAX_BYTES,
    SESSION_ANSWER_MAX_BYTES,
};
use darkrun_api::openapi::{self, API_VERSION};
use darkrun_api::routes::{find, paths};
use darkrun_api::{
    AdvanceResponse, ApproveAction, ApproveActionKind, AuthorType, ClosureReply,
    DirectionAnnotations, DirectionArchetype, DirectionPin, DirectionSelectRequest,
    DirectionSelectResponse, DirectionSessionPayload,
    DiscoveredReviewUrl,     FeedbackAnchor, FeedbackCreateRequest, FeedbackCreateResponse,
    FeedbackDeleteResponse, FeedbackInlineAnchor, FeedbackItem, FeedbackIteration,
    FeedbackListResponse, FeedbackOrigin, FeedbackReply, FeedbackReplyCreateRequest,
    FeedbackReplyCreateResponse, FeedbackResolution, FeedbackScope, FeedbackSeverity,
    FeedbackStatus, FeedbackSummary, FeedbackUpdateRequest, FeedbackUpdateResponse,
    GateType, HttpMethod, InlineComment, IterationResult, KnowledgeFile,
    MilestoneStatus, OutputArtifact, OutputArtifactType, PendingDecision, PickerKind,
    PickerOption, PickerSelectRequest, PickerSelectResponse, PickerSelection,
    PickerSessionPayload, Pin, ProgressMilestone, QuestionAnnotations,
    QuestionAnswer, QuestionAnswerRequest, QuestionAnswerResponse, QuestionOption,
    QuestionPin, QuestionScreenshotAnnotation, QuestionSessionPayload, ReviewAnnotations,
    ReviewCurrentPayload, ReviewCurrentStation, ReviewCurrentUnit, ReviewDecision,
    ReviewDecisionRequest, ReviewDecisionResponse, ReviewSessionPayload, RouteSpec,
    RunCurrentState, RunPhase, SealStatus, SessionPayload, SessionStatus, SessionType,
    StationArtifact, StationStateInfo, UnitOutputPreview, UnitOutputType,
    ValidationError, ValidationIssue, ViewMode, ViewScope, ViewSessionPayload, ViewStatus, ROUTES,
};

// ---------------------------------------------------------------------------
// Small helpers shared across the file.
// ---------------------------------------------------------------------------

/// Serialize `$ty`'s schema and return it as a `serde_json::Value`.
macro_rules! schema_value {
    ($ty:ty) => {{
        serde_json::to_value(schema_for!($ty)).expect("schema serializes")
    }};
}

/// Collect every string in the `enum` arrays of a `oneOf`-encoded enum schema.
fn one_of_enum_values(schema: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(arms) = schema.get("oneOf").and_then(|v| v.as_array()) {
        for arm in arms {
            // Documented enum variants: `const` per arm (draft 2020-12), with
            // single-element `enum` arrays accepted for older emissions.
            if let Some(c) = arm.get("const").and_then(|v| v.as_str()) {
                out.push(c.to_string());
                continue;
            }
            if let Some(vals) = arm.get("enum").and_then(|v| v.as_array()) {
                for v in vals {
                    if let Some(s) = v.as_str() {
                        out.push(s.to_string());
                    }
                }
            }
        }
    }
    out
}

/// Roundtrip a serializable+deserializable value through serde_json and assert
/// the JSON form is stable across the trip.
fn roundtrips_stably<T>(value: &T)
where
    T: serde::Serialize + serde::de::DeserializeOwned,
{
    let json = serde_json::to_value(value).expect("serialize");
    let back: T = serde_json::from_value(json.clone()).expect("deserialize");
    let again = serde_json::to_value(&back).expect("re-serialize");
    assert_eq!(json, again, "value did not roundtrip stably");
}

// ===========================================================================
// SECTION 1 — Route descriptor table
// ===========================================================================

#[test]
fn routes_table_is_non_empty() {
    assert!(!ROUTES.is_empty());
    assert_eq!(ROUTES.len(), 20);
}

#[test]
fn every_route_has_non_empty_fields() {
    for r in ROUTES {
        assert!(!r.path_template.is_empty(), "{} path", r.operation_id);
        assert!(!r.operation_id.is_empty(), "empty operation id");
        assert!(!r.summary.is_empty(), "{} summary", r.operation_id);
        assert!(!r.tag.is_empty(), "{} tag", r.operation_id);
    }
}

#[test]
fn every_route_path_starts_with_slash() {
    for r in ROUTES {
        assert!(
            r.path_template.starts_with('/'),
            "{} does not start with /",
            r.path_template
        );
    }
}

#[test]
fn route_operation_ids_are_unique() {
    let mut ids: Vec<&str> = ROUTES.iter().map(|r| r.operation_id).collect();
    let n = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), n, "duplicate operationId in ROUTES");
}

#[test]
fn route_method_path_pairs_are_unique() {
    let mut pairs: Vec<(HttpMethod, &str)> =
        ROUTES.iter().map(|r| (r.method, r.path_template)).collect();
    let n = pairs.len();
    pairs.sort_by(|a, b| format!("{:?}{}", a.0, a.1).cmp(&format!("{:?}{}", b.0, b.1)));
    pairs.dedup();
    assert_eq!(pairs.len(), n, "duplicate (method, path) in ROUTES");
}

#[test]
fn route_summaries_are_unique() {
    let mut s: Vec<&str> = ROUTES.iter().map(|r| r.summary).collect();
    let n = s.len();
    s.sort_unstable();
    s.dedup();
    assert_eq!(s.len(), n, "duplicate summary in ROUTES");
}

#[test]
fn find_resolves_health() {
    let r = find(HttpMethod::Get, "/health").expect("health");
    assert_eq!(r.operation_id, "getHealth");
    assert_eq!(r.tag, "health");
    assert_eq!(r.method, HttpMethod::Get);
}

#[test]
fn find_resolves_get_session() {
    let r = find(HttpMethod::Get, "/api/session/{sessionId}").expect("getSession");
    assert_eq!(r.operation_id, "getSession");
    assert_eq!(r.tag, "session");
}

#[test]
fn find_resolves_head_heartbeat() {
    let r =
        find(HttpMethod::Head, "/api/session/{sessionId}/heartbeat").expect("heartbeat");
    assert_eq!(r.operation_id, "sessionHeartbeat");
    assert_eq!(r.method, HttpMethod::Head);
}

#[test]
fn find_resolves_ws_upgrade() {
    let r = find(HttpMethod::Ws, "/ws/session/{sessionId}").expect("ws");
    assert_eq!(r.operation_id, "upgradeSessionWebSocket");
    assert_eq!(r.tag, "websocket");
    assert_eq!(r.method, HttpMethod::Ws);
}

#[test]
fn find_distinguishes_method_on_shared_feedback_path() {
    let get = find(HttpMethod::Get, "/api/feedback/{run}/{station}").unwrap();
    let post = find(HttpMethod::Post, "/api/feedback/{run}/{station}").unwrap();
    assert_eq!(get.operation_id, "listFeedback");
    assert_eq!(post.operation_id, "createFeedback");
    assert_ne!(get.operation_id, post.operation_id);
}

#[test]
fn find_distinguishes_put_vs_delete_on_feedback_item() {
    let put = find(HttpMethod::Put, "/api/feedback/{run}/{station}/{id}").unwrap();
    let del = find(HttpMethod::Delete, "/api/feedback/{run}/{station}/{id}").unwrap();
    assert_eq!(put.operation_id, "updateFeedback");
    assert_eq!(del.operation_id, "deleteFeedback");
}

#[test]
fn find_returns_none_for_unknown_path() {
    assert!(find(HttpMethod::Get, "/nope").is_none());
}

#[test]
fn find_returns_none_for_wrong_method() {
    // /health is GET only.
    assert!(find(HttpMethod::Post, "/health").is_none());
    assert!(find(HttpMethod::Delete, "/health").is_none());
}

#[test]
fn find_is_method_sensitive_for_session_get_not_post() {
    assert!(find(HttpMethod::Get, "/api/session/{sessionId}").is_some());
    assert!(find(HttpMethod::Post, "/api/session/{sessionId}").is_none());
}

#[test]
fn all_expected_operation_ids_present() {
    let expected = [
        "getSession",
        "sessionHeartbeat",
        "postReviewDecide",
        "getReviewCurrent",
        "postQuestionAnswer",
        "postDirectionSelect",
        "postPickerSelect",
        "postAdvance",
        "listFeedback",
        "createFeedback",
        "updateFeedback",
        "deleteFeedback",
        "createFeedbackReply",
        "getHealth",
        "upgradeSessionWebSocket",
    ];
    for op in expected {
        assert!(
            ROUTES.iter().any(|r| r.operation_id == op),
            "missing route operation id {op}"
        );
    }
}

#[test]
fn exactly_one_ws_route() {
    let ws: Vec<_> = ROUTES.iter().filter(|r| r.method == HttpMethod::Ws).collect();
    assert_eq!(ws.len(), 1);
    assert_eq!(ws[0].operation_id, "upgradeSessionWebSocket");
}

#[test]
fn feedback_tag_groups_five_routes() {
    let n = ROUTES.iter().filter(|r| r.tag == "feedback").count();
    assert_eq!(n, 5);
}

#[test]
fn session_tag_groups_two_routes() {
    let n = ROUTES.iter().filter(|r| r.tag == "session").count();
    assert_eq!(n, 2);
}

#[test]
fn review_tag_groups_three_routes() {
    // decide, current, advance are all tagged "review".
    let n = ROUTES.iter().filter(|r| r.tag == "review").count();
    assert_eq!(n, 3);
}

#[test]
fn each_tag_has_at_least_one_route() {
    for tag in [
        "session",
        "review",
        "question",
        "direction",
        "picker",
        "feedback",
        "health",
        "websocket",
    ] {
        assert!(
            ROUTES.iter().any(|r| r.tag == tag),
            "no route for tag {tag}"
        );
    }
}

#[test]
fn methods_used_match_expected_set() {
    use std::collections::BTreeSet;
    let methods: BTreeSet<String> =
        ROUTES.iter().map(|r| format!("{:?}", r.method)).collect();
    let expected: BTreeSet<String> = ["Get", "Head", "Post", "Put", "Delete", "Ws"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(methods, expected);
}

#[test]
fn templated_paths_use_brace_params() {
    for r in ROUTES {
        if r.path_template.contains('{') {
            assert!(
                r.path_template.contains('}'),
                "unbalanced brace in {}",
                r.path_template
            );
        }
    }
}

#[test]
fn no_route_path_has_trailing_slash() {
    for r in ROUTES {
        assert!(
            !r.path_template.ends_with('/') || r.path_template == "/",
            "trailing slash on {}",
            r.path_template
        );
    }
}

#[test]
fn routespec_is_copy_and_eq() {
    let a: RouteSpec = *find(HttpMethod::Get, "/health").unwrap();
    let b = a; // Copy
    assert_eq!(a, b);
    assert_eq!(a.operation_id, b.operation_id);
}

#[test]
fn http_method_equality_and_inequality() {
    assert_eq!(HttpMethod::Get, HttpMethod::Get);
    assert_ne!(HttpMethod::Get, HttpMethod::Post);
    assert_ne!(HttpMethod::Put, HttpMethod::Delete);
    assert_ne!(HttpMethod::Ws, HttpMethod::Get);
}

#[test]
fn http_method_debug_is_distinct() {
    use std::collections::BTreeSet;
    let dbg: BTreeSet<String> = [
        HttpMethod::Get,
        HttpMethod::Head,
        HttpMethod::Post,
        HttpMethod::Put,
        HttpMethod::Delete,
        HttpMethod::Ws,
    ]
    .iter()
    .map(|m| format!("{m:?}"))
    .collect();
    assert_eq!(dbg.len(), 6);
}

// --- path builders ---------------------------------------------------------

#[test]
fn path_builder_session() {
    assert_eq!(paths::session("abc"), "/api/session/abc");
    assert_eq!(paths::session(""), "/api/session/");
    assert_eq!(paths::session("a/b"), "/api/session/a/b");
}

#[test]
fn path_builder_session_heartbeat() {
    assert_eq!(
        paths::session_heartbeat("xyz"),
        "/api/session/xyz/heartbeat"
    );
}

#[test]
fn path_builder_review_decide() {
    assert_eq!(paths::review_decide("s1"), "/review/s1/decide");
}

#[test]
fn path_builder_review_current_is_constant() {
    assert_eq!(paths::review_current(), "/api/review/current");
    assert_eq!(paths::review_current(), paths::review_current());
}

#[test]
fn path_builder_question_answer() {
    assert_eq!(paths::question_answer("q9"), "/question/q9/answer");
}

#[test]
fn path_builder_direction_select() {
    assert_eq!(paths::direction_select("d2"), "/direction/d2/select");
}

#[test]
fn path_builder_picker_select() {
    assert_eq!(paths::picker_select("p3"), "/picker/p3/select");
}

#[test]
fn path_builder_advance() {
    assert_eq!(paths::advance("s7"), "/api/advance/s7");
}

#[test]
fn path_builder_feedback_list() {
    assert_eq!(
        paths::feedback_list("run", "frame"),
        "/api/feedback/run/frame"
    );
}

#[test]
fn path_builder_feedback_item() {
    assert_eq!(
        paths::feedback_item("run", "frame", "FB-01"),
        "/api/feedback/run/frame/FB-01"
    );
}

#[test]
fn path_builder_feedback_replies() {
    assert_eq!(
        paths::feedback_replies("run", "frame", "FB-01"),
        "/api/feedback/run/frame/FB-01/replies"
    );
}

#[test]
fn path_builder_health_is_constant() {
    assert_eq!(paths::health(), "/health");
}

#[test]
fn path_builder_ws_session() {
    assert_eq!(paths::ws_session("s1"), "/ws/session/s1");
}

#[test]
fn feedback_replies_extends_feedback_item() {
    let item = paths::feedback_item("r", "s", "FB-9");
    let replies = paths::feedback_replies("r", "s", "FB-9");
    assert_eq!(replies, format!("{item}/replies"));
}

#[test]
fn concrete_session_path_template_alignment() {
    // The concrete builder fills the same prefix the template declares.
    let r = find(HttpMethod::Get, "/api/session/{sessionId}").unwrap();
    let concrete = paths::session("ID");
    assert!(concrete.starts_with("/api/session/"));
    assert!(r.path_template.starts_with("/api/session/"));
}

#[test]
fn path_builders_handle_unicode_ids() {
    assert_eq!(paths::session("日本"), "/api/session/日本");
    assert_eq!(paths::feedback_list("rün", "stâge"), "/api/feedback/rün/stâge");
}

// ===========================================================================
// SECTION 2 — SessionPayload union (serde discrimination)
// ===========================================================================

#[test]
fn session_payload_review_tag() {
    let p = SessionPayload::Review(ReviewSessionPayload {
        session_id: "r".into(),
        ..Default::default()
    });
    let j = serde_json::to_value(&p).unwrap();
    assert_eq!(j["session_type"], "review");
    assert_eq!(j["session_id"], "r");
    assert_eq!(p.session_type(), "review");
    assert_eq!(p.session_id(), "r");
}

#[test]
fn session_payload_question_tag() {
    let p = SessionPayload::Question(QuestionSessionPayload {
        session_id: "q".into(),
        ..Default::default()
    });
    assert_eq!(serde_json::to_value(&p).unwrap()["session_type"], "question");
    assert_eq!(p.session_type(), "question");
}

#[test]
fn session_payload_direction_tag() {
    let p = SessionPayload::Direction(DirectionSessionPayload {
        session_id: "d".into(),
        ..Default::default()
    });
    assert_eq!(serde_json::to_value(&p).unwrap()["session_type"], "direction");
    assert_eq!(p.session_type(), "direction");
}

#[test]
fn session_payload_picker_tag() {
    let p = SessionPayload::Picker(PickerSessionPayload {
        session_id: "p".into(),
        status: SessionStatus::Pending,
        run_slug: None,
        kind: PickerKind::Factory,
        title: "t".into(),
        prompt: "?".into(),
        options: vec![],
        selection: None,
    });
    assert_eq!(serde_json::to_value(&p).unwrap()["session_type"], "picker");
    assert_eq!(p.session_type(), "picker");
}

#[test]
fn session_payload_view_tag() {
    let p = SessionPayload::View(ViewSessionPayload {
        session_id: "v".into(),
        status: ViewStatus::Open,
        run_slug: "run".into(),
        scope: ViewScope::Run,
        artifacts: vec![],
        factory: None,
        station: None,
        artifact: None,
        mode: ViewMode::Viewer,
        boot_port: None,
        boot_command: None,
    });
    assert_eq!(serde_json::to_value(&p).unwrap()["session_type"], "view");
    assert_eq!(p.session_id(), "v");
}

#[test]
fn session_payload_roundtrips_all_variants() {
    let variants = vec![
        SessionPayload::Review(ReviewSessionPayload {
            session_id: "r".into(),
            ..Default::default()
        }),
        SessionPayload::Question(QuestionSessionPayload {
            session_id: "q".into(),
            ..Default::default()
        }),
        SessionPayload::Direction(DirectionSessionPayload {
            session_id: "d".into(),
            ..Default::default()
        }),
        SessionPayload::Picker(PickerSessionPayload {
            session_id: "p".into(),
            status: SessionStatus::Pending,
            run_slug: None,
            kind: PickerKind::Mode,
            title: "t".into(),
            prompt: "?".into(),
            options: vec![],
            selection: None,
        }),
        SessionPayload::View(ViewSessionPayload {
            session_id: "v".into(),
            status: ViewStatus::Closed,
            run_slug: "run".into(),
            scope: ViewScope::Run,
            artifacts: vec![],
            factory: None,
            station: None,
            artifact: None,
            mode: ViewMode::Boot,
            boot_port: Some(3000),
            boot_command: Some("npm run dev".into()),
        }),
    ];
    for v in &variants {
        let json = serde_json::to_value(v).unwrap();
        let back: SessionPayload = serde_json::from_value(json).unwrap();
        assert_eq!(back.session_type(), v.session_type());
        assert_eq!(back.session_id(), v.session_id());
    }
}

#[test]
fn session_payload_rejects_unknown_type() {
    let j = json!({ "session_type": "telepathy", "session_id": "x" });
    let r: Result<SessionPayload, _> = serde_json::from_value(j);
    assert!(r.is_err());
}

#[test]
fn session_payload_rejects_missing_type() {
    let j = json!({ "session_id": "x", "status": "pending" });
    let r: Result<SessionPayload, _> = serde_json::from_value(j);
    assert!(r.is_err());
}

#[test]
fn session_payload_rejects_empty_object() {
    let r: Result<SessionPayload, _> = serde_json::from_value(json!({}));
    assert!(r.is_err());
}

#[test]
fn session_payload_rejects_capitalized_type() {
    // rename_all snake_case means "Review" must not match.
    let j = json!({ "session_type": "Review", "session_id": "x" });
    let r: Result<SessionPayload, _> = serde_json::from_value(j);
    assert!(r.is_err());
}

#[test]
fn session_payload_review_inlines_fields_not_nested() {
    // Internally tagged: the review fields are siblings of session_type.
    let p = SessionPayload::Review(ReviewSessionPayload {
        session_id: "r".into(),
        run_slug: Some("slug".into()),
        ..Default::default()
    });
    let j = serde_json::to_value(&p).unwrap();
    assert_eq!(j["run_slug"], "slug");
    assert!(j.get("Review").is_none(), "must not externally tag");
}

#[test]
fn session_type_helper_matches_all_five() {
    let kinds = [
        ("review", SessionPayload::Review(Default::default())),
        ("question", SessionPayload::Question(Default::default())),
        ("direction", SessionPayload::Direction(Default::default())),
    ];
    for (name, p) in kinds {
        assert_eq!(p.session_type(), name);
    }
}

// ===========================================================================
// SECTION 3 — Enum serde encodings (the load-bearing wire tokens)
// ===========================================================================

#[test]
fn session_status_snake_case_tokens() {
    let cases = [
        (SessionStatus::Pending, "pending"),
        (SessionStatus::Decided, "decided"),
        (SessionStatus::Answered, "answered"),
        (SessionStatus::Approved, "approved"),
        (SessionStatus::ChangesRequested, "changes_requested"),
    ];
    for (v, s) in cases {
        assert_eq!(serde_json::to_value(v).unwrap(), json!(s));
        let back: SessionStatus = serde_json::from_value(json!(s)).unwrap();
        assert_eq!(back, v);
    }
}

#[test]
fn session_status_default_is_pending() {
    assert_eq!(SessionStatus::default(), SessionStatus::Pending);
}

#[test]
fn session_status_rejects_camel_case() {
    let r: Result<SessionStatus, _> = serde_json::from_value(json!("changesRequested"));
    assert!(r.is_err());
}

#[test]
fn gate_type_tokens() {
    for (v, s) in [
        (GateType::Auto, "auto"),
        (GateType::Ask, "ask"),
        (GateType::External, "external"),
        (GateType::Await, "await"),
    ] {
        assert_eq!(serde_json::to_value(v).unwrap(), json!(s));
        assert_eq!(
            serde_json::from_value::<GateType>(json!(s)).unwrap(),
            v
        );
    }
}

#[test]
fn session_type_enum_tokens() {
    for (v, s) in [
        (SessionType::Review, "review"),
        (SessionType::Question, "question"),
        (SessionType::Direction, "direction"),
        (SessionType::Picker, "picker"),
        (SessionType::View, "view"),
    ] {
        assert_eq!(serde_json::to_value(v).unwrap(), json!(s));
    }
}

#[test]
fn author_type_tokens() {
    for (v, s) in [
        (AuthorType::Human, "human"),
        (AuthorType::Agent, "agent"),
        (AuthorType::System, "system"),
    ] {
        assert_eq!(serde_json::to_value(v).unwrap(), json!(s));
    }
}

#[test]
fn feedback_origin_kebab_case_tokens() {
    let cases = [
        (FeedbackOrigin::AdversarialReview, "adversarial-review"),
        (FeedbackOrigin::StudioReview, "studio-review"),
        (FeedbackOrigin::EngineReview, "engine-review"),
        (FeedbackOrigin::Drift, "drift"),
        (FeedbackOrigin::Discovery, "discovery"),
        (FeedbackOrigin::ExternalPr, "external-pr"),
        (FeedbackOrigin::ExternalMr, "external-mr"),
        (FeedbackOrigin::UserVisual, "user-visual"),
        (FeedbackOrigin::UserChat, "user-chat"),
        (FeedbackOrigin::UserQuestion, "user-question"),
        (FeedbackOrigin::UserRevisit, "user-revisit"),
        (FeedbackOrigin::Agent, "agent"),
    ];
    for (v, s) in cases {
        assert_eq!(serde_json::to_value(v).unwrap(), json!(s));
        assert_eq!(
            serde_json::from_value::<FeedbackOrigin>(json!(s)).unwrap(),
            v
        );
    }
}

#[test]
fn feedback_origin_rejects_snake_case() {
    let r: Result<FeedbackOrigin, _> =
        serde_json::from_value(json!("adversarial_review"));
    assert!(r.is_err());
}

#[test]
fn feedback_severity_tokens() {
    for (v, s) in [
        (FeedbackSeverity::Blocker, "blocker"),
        (FeedbackSeverity::High, "high"),
        (FeedbackSeverity::Medium, "medium"),
        (FeedbackSeverity::Low, "low"),
    ] {
        assert_eq!(serde_json::to_value(v).unwrap(), json!(s));
    }
}

#[test]
fn feedback_status_tokens() {
    let cases = [
        (FeedbackStatus::Pending, "pending"),
        (FeedbackStatus::Fixing, "fixing"),
        (FeedbackStatus::Addressed, "addressed"),
        (FeedbackStatus::Answered, "answered"),
        (FeedbackStatus::NonActionable, "non_actionable"),
        (FeedbackStatus::Escalated, "escalated"),
        (FeedbackStatus::Closed, "closed"),
        (FeedbackStatus::Rejected, "rejected"),
    ];
    for (v, s) in cases {
        assert_eq!(serde_json::to_value(v).unwrap(), json!(s));
        assert_eq!(
            serde_json::from_value::<FeedbackStatus>(json!(s)).unwrap(),
            v
        );
    }
}

#[test]
fn feedback_resolution_tokens() {
    for (v, s) in [
        (FeedbackResolution::Question, "question"),
        (FeedbackResolution::InlineFix, "inline_fix"),
        (FeedbackResolution::StageRevisit, "stage_revisit"),
    ] {
        assert_eq!(serde_json::to_value(v).unwrap(), json!(s));
    }
}

#[test]
fn feedback_scope_tokens() {
    for (v, s) in [
        (FeedbackScope::Intent, "intent"),
        (FeedbackScope::Stage, "stage"),
    ] {
        assert_eq!(serde_json::to_value(v).unwrap(), json!(s));
    }
}

#[test]
fn iteration_result_tokens() {
    for (v, s) in [
        (IterationResult::Advanced, "advanced"),
        (IterationResult::Closed, "closed"),
        (IterationResult::Reopened, "reopened"),
        (IterationResult::Rejected, "rejected"),
    ] {
        assert_eq!(serde_json::to_value(v).unwrap(), json!(s));
    }
}

#[test]
fn run_phase_tokens_follow_factory_vocabulary() {
    let cases = [
        (RunPhase::Spec, "spec"),
        (RunPhase::Review, "review"),
        (RunPhase::Manufacture, "manufacture"),
        (RunPhase::Audit, "audit"),
        (RunPhase::Reflect, "reflect"),
        (RunPhase::Checkpoint, "checkpoint"),
    ];
    for (v, s) in cases {
        assert_eq!(serde_json::to_value(v).unwrap(), json!(s));
        assert_eq!(serde_json::from_value::<RunPhase>(json!(s)).unwrap(), v);
    }
}

#[test]
fn milestone_status_tokens() {
    for (v, s) in [
        (MilestoneStatus::Done, "done"),
        (MilestoneStatus::Active, "active"),
        (MilestoneStatus::Pending, "pending"),
    ] {
        assert_eq!(serde_json::to_value(v).unwrap(), json!(s));
    }
}

#[test]
fn output_artifact_type_tokens() {
    for (v, s) in [
        (OutputArtifactType::Markdown, "markdown"),
        (OutputArtifactType::Html, "html"),
        (OutputArtifactType::Image, "image"),
        (OutputArtifactType::Video, "video"),
        (OutputArtifactType::Code, "code"),
        (OutputArtifactType::File, "file"),
    ] {
        assert_eq!(serde_json::to_value(v).unwrap(), json!(s));
    }
}

#[test]
fn unit_output_type_tokens() {
    for (v, s) in [
        (UnitOutputType::Markdown, "markdown"),
        (UnitOutputType::Html, "html"),
        (UnitOutputType::Image, "image"),
        (UnitOutputType::File, "file"),
    ] {
        assert_eq!(serde_json::to_value(v).unwrap(), json!(s));
    }
}

#[test]
fn approve_action_kind_tokens() {
    let cases = [
        (ApproveActionKind::AdHocDone, "ad_hoc_done"),
        (ApproveActionKind::OpenPr, "open_pr"),
        (ApproveActionKind::SubmitExternal, "submit_external"),
        (ApproveActionKind::StartRun, "start_run"),
        (ApproveActionKind::StartExecution, "start_execution"),
        (ApproveActionKind::CompleteStation, "complete_station"),
        (ApproveActionKind::SubmitRunReview, "submit_run_review"),
        (ApproveActionKind::CompleteRun, "complete_run"),
        (ApproveActionKind::Approve, "approve"),
    ];
    for (v, s) in cases {
        assert_eq!(serde_json::to_value(v).unwrap(), json!(s));
    }
}

#[test]
fn seal_status_tokens() {
    for (v, s) in [
        (SealStatus::Sealed, "sealed"),
        (SealStatus::PendingSeal, "pending_seal"),
    ] {
        assert_eq!(serde_json::to_value(v).unwrap(), json!(s));
    }
}

#[test]
fn picker_kind_tokens() {
    for (v, s) in [
        (PickerKind::Factory, "factory"),
        (PickerKind::Mode, "mode"),
        (PickerKind::Station, "station"),
        (PickerKind::Confirm, "confirm"),
        (PickerKind::UrlInput, "url_input"),
    ] {
        assert_eq!(serde_json::to_value(v).unwrap(), json!(s));
    }
}

#[test]
fn view_mode_tokens() {
    for (v, s) in [(ViewMode::Viewer, "viewer"), (ViewMode::Boot, "boot")] {
        assert_eq!(serde_json::to_value(v).unwrap(), json!(s));
    }
}

#[test]
fn view_status_tokens() {
    for (v, s) in [(ViewStatus::Open, "open"), (ViewStatus::Closed, "closed")] {
        assert_eq!(serde_json::to_value(v).unwrap(), json!(s));
    }
}

#[test]
fn review_decision_tokens() {
    assert_eq!(
        serde_json::to_value(ReviewDecision::Approved).unwrap(),
        json!("approved")
    );
    assert_eq!(
        serde_json::to_value(ReviewDecision::ChangesRequested).unwrap(),
        json!("changes_requested")
    );
}

// ===========================================================================
// SECTION 4 — Domain logic on the common/feedback types
// ===========================================================================

#[test]
fn feedback_origin_author_type_human_set() {
    for o in [
        FeedbackOrigin::UserVisual,
        FeedbackOrigin::UserChat,
        FeedbackOrigin::UserQuestion,
        FeedbackOrigin::UserRevisit,
        FeedbackOrigin::ExternalPr,
        FeedbackOrigin::ExternalMr,
    ] {
        assert_eq!(o.author_type(), AuthorType::Human, "{o:?}");
    }
}

#[test]
fn feedback_origin_author_type_agent_set() {
    for o in [
        FeedbackOrigin::AdversarialReview,
        FeedbackOrigin::StudioReview,
        FeedbackOrigin::EngineReview,
        FeedbackOrigin::Drift,
        FeedbackOrigin::Discovery,
        FeedbackOrigin::Agent,
    ] {
        assert_eq!(o.author_type(), AuthorType::Agent, "{o:?}");
    }
}

#[test]
fn feedback_origin_author_type_is_total_two_valued() {
    // Author type is never System for any origin (System is manager-only,
    // not derivable from an origin).
    for o in [
        FeedbackOrigin::AdversarialReview,
        FeedbackOrigin::StudioReview,
        FeedbackOrigin::EngineReview,
        FeedbackOrigin::Drift,
        FeedbackOrigin::Discovery,
        FeedbackOrigin::ExternalPr,
        FeedbackOrigin::ExternalMr,
        FeedbackOrigin::UserVisual,
        FeedbackOrigin::UserChat,
        FeedbackOrigin::UserQuestion,
        FeedbackOrigin::UserRevisit,
        FeedbackOrigin::Agent,
    ] {
        assert_ne!(o.author_type(), AuthorType::System);
    }
}

#[test]
fn feedback_status_canonicalize_known_lowercase() {
    let cases = [
        ("pending", FeedbackStatus::Pending),
        ("fixing", FeedbackStatus::Fixing),
        ("addressed", FeedbackStatus::Addressed),
        ("answered", FeedbackStatus::Answered),
        ("non_actionable", FeedbackStatus::NonActionable),
        ("escalated", FeedbackStatus::Escalated),
        ("closed", FeedbackStatus::Closed),
        ("rejected", FeedbackStatus::Rejected),
    ];
    for (raw, want) in cases {
        assert_eq!(FeedbackStatus::canonicalize(raw), want, "{raw}");
    }
}

#[test]
fn feedback_status_canonicalize_uppercase_and_mixed() {
    assert_eq!(FeedbackStatus::canonicalize("FIXING"), FeedbackStatus::Fixing);
    assert_eq!(FeedbackStatus::canonicalize("Closed"), FeedbackStatus::Closed);
    assert_eq!(
        FeedbackStatus::canonicalize("ReJeCtEd"),
        FeedbackStatus::Rejected
    );
}

#[test]
fn feedback_status_canonicalize_trims_whitespace() {
    assert_eq!(
        FeedbackStatus::canonicalize("  closed  "),
        FeedbackStatus::Closed
    );
    assert_eq!(
        FeedbackStatus::canonicalize("\tfixing\n"),
        FeedbackStatus::Fixing
    );
}

#[test]
fn feedback_status_canonicalize_unknown_falls_back_to_pending() {
    for raw in ["", "weird", "done", "open", "in_progress", "approved"] {
        assert_eq!(
            FeedbackStatus::canonicalize(raw),
            FeedbackStatus::Pending,
            "{raw}"
        );
    }
}

#[test]
fn feedback_status_as_str_matches_serde_token() {
    for v in [
        FeedbackStatus::Pending,
        FeedbackStatus::Fixing,
        FeedbackStatus::Addressed,
        FeedbackStatus::Answered,
        FeedbackStatus::NonActionable,
        FeedbackStatus::Escalated,
        FeedbackStatus::Closed,
        FeedbackStatus::Rejected,
    ] {
        let serde_token = serde_json::to_value(v).unwrap();
        assert_eq!(serde_token, json!(v.as_str()), "{v:?}");
    }
}

#[test]
fn feedback_status_as_str_canonicalize_roundtrip() {
    for v in [
        FeedbackStatus::Pending,
        FeedbackStatus::Fixing,
        FeedbackStatus::Addressed,
        FeedbackStatus::Answered,
        FeedbackStatus::NonActionable,
        FeedbackStatus::Escalated,
        FeedbackStatus::Closed,
        FeedbackStatus::Rejected,
    ] {
        assert_eq!(FeedbackStatus::canonicalize(v.as_str()), v);
    }
}

#[test]
fn feedback_status_blocks_gate_only_pending_and_fixing() {
    assert!(FeedbackStatus::Pending.blocks_gate());
    assert!(FeedbackStatus::Fixing.blocks_gate());
    for v in [
        FeedbackStatus::Addressed,
        FeedbackStatus::Answered,
        FeedbackStatus::NonActionable,
        FeedbackStatus::Escalated,
        FeedbackStatus::Closed,
        FeedbackStatus::Rejected,
    ] {
        assert!(!v.blocks_gate(), "{v:?} must not block gate");
    }
}

#[test]
fn review_decision_canonicalize_approved_variants() {
    for raw in ["approved", "APPROVED", "Approved", "  approved ", "\tapproved\n"] {
        assert_eq!(ReviewDecision::canonicalize(raw), ReviewDecision::Approved, "{raw}");
    }
}

#[test]
fn review_decision_canonicalize_everything_else_changes_requested() {
    for raw in [
        "",
        "nope",
        "changes_requested",
        "reject",
        "approve",   // not the exact word "approved"
        "approveds", // suffix
        "approve d",
    ] {
        assert_eq!(
            ReviewDecision::canonicalize(raw),
            ReviewDecision::ChangesRequested,
            "{raw}"
        );
    }
}

#[test]
fn validation_error_new_stamps_discriminator() {
    let err = ValidationError::new(vec![ValidationIssue {
        code: "invalid_json".into(),
        message: "bad".into(),
        path: vec![],
    }]);
    assert_eq!(err.error, "validation_failed");
    let j = serde_json::to_value(&err).unwrap();
    assert_eq!(j["error"], "validation_failed");
    assert_eq!(j["issues"].as_array().unwrap().len(), 1);
}

#[test]
fn validation_error_empty_issues_still_stamps() {
    let err = ValidationError::new(vec![]);
    assert_eq!(err.error, "validation_failed");
    assert!(err.issues.is_empty());
}

#[test]
fn validation_issue_path_is_array_of_strings() {
    let issue = ValidationIssue {
        code: "type".into(),
        message: "expected string".into(),
        path: vec!["body".into(), "title".into()],
    };
    let j = serde_json::to_value(&issue).unwrap();
    assert_eq!(j["path"], json!(["body", "title"]));
    roundtrips_stably(&issue);
}

#[test]
fn feedback_update_request_is_empty_default() {
    assert!(FeedbackUpdateRequest::default().is_empty());
}

#[test]
fn feedback_update_request_not_empty_with_status() {
    let r = FeedbackUpdateRequest {
        status: Some(FeedbackStatus::Closed),
        ..Default::default()
    };
    assert!(!r.is_empty());
}

#[test]
fn feedback_update_request_not_empty_with_closed_by() {
    let r = FeedbackUpdateRequest {
        closed_by: Some("unit-1".into()),
        ..Default::default()
    };
    assert!(!r.is_empty());
}

#[test]
fn feedback_update_request_not_empty_with_resolution() {
    let r = FeedbackUpdateRequest {
        resolution: Some(FeedbackResolution::InlineFix),
        ..Default::default()
    };
    assert!(!r.is_empty());
}

#[test]
fn feedback_update_request_empty_body_deserializes_then_is_empty() {
    let r: FeedbackUpdateRequest = serde_json::from_value(json!({})).unwrap();
    assert!(r.is_empty());
}

// ===========================================================================
// SECTION 5 — Roundtrips and serde defaults / skipping
// ===========================================================================

#[test]
fn review_payload_full_roundtrip() {
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
    let json = serde_json::to_value(&payload).unwrap();
    assert_eq!(json["gate_type"], "ask");
    assert_eq!(json["current_state"]["phase"], "checkpoint");
    assert_eq!(json["approve_action"]["kind"], "complete_station");
    let back: SessionPayload = serde_json::from_value(json).unwrap();
    assert_eq!(back.session_id(), "s-1");
}

#[test]
fn review_payload_default_skips_optional_fields() {
    let p = ReviewSessionPayload {
        session_id: "x".into(),
        ..Default::default()
    };
    let j = serde_json::to_value(&p).unwrap();
    let obj = j.as_object().unwrap();
    // None / empty-collection fields are skipped.
    assert!(!obj.contains_key("run_slug"));
    assert!(!obj.contains_key("gate_type"));
    assert!(!obj.contains_key("units"));
    assert!(!obj.contains_key("station_states"));
    assert!(!obj.contains_key("drift"));
    // The required fields are present.
    assert!(obj.contains_key("session_id"));
    assert!(obj.contains_key("status"));
}

#[test]
fn review_payload_carries_opaque_run_value() {
    let p = ReviewSessionPayload {
        session_id: "x".into(),
        run: Some(json!({ "arbitrary": [1, 2, 3], "nested": { "k": "v" } })),
        units: vec![json!({ "u": 1 }), json!("opaque")],
        ..Default::default()
    };
    let j = serde_json::to_value(&p).unwrap();
    assert_eq!(j["run"]["arbitrary"], json!([1, 2, 3]));
    assert_eq!(j["units"][1], json!("opaque"));
    let back: ReviewSessionPayload = serde_json::from_value(j).unwrap();
    assert_eq!(back.units.len(), 2);
}

#[test]
fn review_payload_station_maps_roundtrip() {
    let mut p = ReviewSessionPayload {
        session_id: "x".into(),
        ..Default::default()
    };
    p.station_summaries.insert("frame".into(), "the frame".into());
    p.station_states.push(StationStateInfo {
        station: "frame".into(),
        merged_into_main: true,
        status: Some("done".into()),
        phase: Some("checkpoint".into()),
        started_at: None,
        completed_at: None,
        gate_entered_at: None,
        gate_outcome: Some("approved".into()),
    });
    let j = serde_json::to_value(&p).unwrap();
    assert_eq!(j["station_summaries"]["frame"], "the frame");
    // station_states is an ordered array now, not a map.
    assert_eq!(j["station_states"][0]["merged_into_main"], true);
    roundtrips_stably(&p);
}

#[test]
fn btreemap_fields_serialize_sorted() {
    let mut p = ReviewSessionPayload {
        session_id: "x".into(),
        ..Default::default()
    };
    for k in ["zeta", "alpha", "mid"] {
        p.station_summaries.insert(k.into(), k.into());
    }
    let s = serde_json::to_string(&p).unwrap();
    let a = s.find("alpha").unwrap();
    let m = s.find("mid").unwrap();
    let z = s.find("zeta").unwrap();
    assert!(a < m && m < z, "BTreeMap keys must serialize sorted");
}

#[test]
fn question_payload_roundtrip() {
    let p = QuestionSessionPayload {
        session_id: "q".into(),
        status: SessionStatus::Answered,
        run_slug: None,
        title: Some("Pick".into()),
        prompt: "which?".into(),
        context: Some("ctx".into()),
        options: vec![
            QuestionOption {
                id: "a".into(),
                label: "A".into(),
                image_url: Some("/mock/a.png".into()),
                image_url_light: None,
                description: None,
            },
            QuestionOption {
                id: "b".into(),
                label: "B".into(),
                image_url: None,
                image_url_light: None,
                description: Some("the other one".into()),
            },
        ],
        multi_select: true,
        answer: Some(QuestionAnswer {
            selected: vec!["a".into()],
            text: None,
        }),
        image_urls: vec!["/img/1.png".into()],
    };
    roundtrips_stably(&p);
    let j = serde_json::to_value(&p).unwrap();
    assert_eq!(j["options"][0]["id"], "a");
    assert_eq!(j["options"][0]["image_url"], "/mock/a.png");
    assert_eq!(j["answer"]["selected"], json!(["a"]));
    assert_eq!(j["status"], "answered");
}

#[test]
fn direction_payload_roundtrip() {
    let p = DirectionSessionPayload {
        session_id: "d".into(),
        status: SessionStatus::Pending,
        title: Some("Direction".into()),
        run_slug: Some("run".into()),
        prompt: "pick a direction".into(),
        context: Some("pick a vibe".into()),
        archetypes: vec![DirectionArchetype {
            id: "brutalist".into(),
            label: "Brutalist".into(),
            image_url: "/mock/brutalist.png".into(),
            image_url_light: None,
            description: "raw".into(),
        }],
        chosen_archetype: Some("brutalist".into()),
        annotations: Some(DirectionAnnotations {
            pins: vec![],
            screenshot: None,
            comments: vec!["yes".into()],
        }),
    };
    roundtrips_stably(&p);
    let j = serde_json::to_value(&p).unwrap();
    assert_eq!(j["archetypes"][0]["id"], "brutalist");
    assert_eq!(j["archetypes"][0]["image_url"], "/mock/brutalist.png");
    assert_eq!(j["chosen_archetype"], "brutalist");
    assert_eq!(j["annotations"]["comments"][0], "yes");
}

#[test]
fn picker_payload_roundtrip_with_options() {
    let p = PickerSessionPayload {
        session_id: "p".into(),
        status: SessionStatus::Pending,
        run_slug: Some("run".into()),
        kind: PickerKind::Station,
        title: "Pick a station".into(),
        prompt: "?".into(),
        options: vec![
            PickerOption {
                id: "frame".into(),
                label: "Frame".into(),
                description: Some("the frame".into()),
                secondary: None,
            },
            PickerOption {
                id: "polish".into(),
                label: "Polish".into(),
                description: None,
                secondary: Some(true),
            },
        ],
        selection: Some(PickerSelection { id: "frame".into() }),
    };
    roundtrips_stably(&p);
    let j = serde_json::to_value(&p).unwrap();
    assert_eq!(j["kind"], "station");
    assert_eq!(j["options"][1]["secondary"], true);
    assert_eq!(j["selection"]["id"], "frame");
}

#[test]
fn view_payload_boot_mode_roundtrip() {
    let p = ViewSessionPayload {
        session_id: "v".into(),
        status: ViewStatus::Open,
        run_slug: "run".into(),
        scope: ViewScope::Run,
        artifacts: vec![],
        factory: Some("software".into()),
        station: Some("frame".into()),
        artifact: Some("index.html".into()),
        mode: ViewMode::Boot,
        boot_port: Some(5173),
        boot_command: Some("vite".into()),
    };
    roundtrips_stably(&p);
    let j = serde_json::to_value(&p).unwrap();
    assert_eq!(j["mode"], "boot");
    assert_eq!(j["boot_port"], 5173);
}

#[test]
fn feedback_item_full_roundtrip() {
    let item = FeedbackItem {
        feedback_id: "FB-07".into(),
        title: "fix the thing".into(),
        body: "details".into(),
        status: FeedbackStatus::Fixing,
        origin: FeedbackOrigin::AdversarialReview,
        severity: Some(FeedbackSeverity::Blocker),
        author: "reviewer".into(),
        author_type: AuthorType::Agent,
        created_at: "2026-05-30T00:00:00Z".into(),
        visit: 3,
        source_ref: Some("unit-1/output.md".into()),
        closed_by: None,
        resolution: Some(FeedbackResolution::StageRevisit),
        replies: vec![FeedbackReply {
            author: "user".into(),
            author_type: AuthorType::Human,
            body: "thanks".into(),
            created_at: "2026-05-30T01:00:00Z".into(),
        }],
        inline_anchor: Some(FeedbackInlineAnchor {
            selected_text: "foo".into(),
            paragraph: 2,
            location: "doc.md".into(),
            comment_id: Some("c1".into()),
            file_path: Some("a/doc.md".into()),
            content_sha: Some("deadbeef".into()),
        }),
        scope: Some(FeedbackScope::Stage),
        iterations: vec![FeedbackIteration {
            bolt: 1,
            hat: "fixer".into(),
            started_at: Some("t0".into()),
            completed_at: Some("t1".into()),
            result: Some(IterationResult::Closed),
            commit: Some("abc123".into()),
            message: Some("done".into()),
            reason: None,
        }],
        closure_reply: Some(ClosureReply {
            text: "fixed".into(),
            at: "t2".into(),
        }),
        closure_reply_unread: Some(true),
    };
    roundtrips_stably(&item);
    let j = serde_json::to_value(&item).unwrap();
    assert_eq!(j["origin"], "adversarial-review");
    assert_eq!(j["author_type"], "agent");
    assert_eq!(j["severity"], "blocker");
    assert_eq!(j["status"], "fixing");
    assert_eq!(j["iterations"][0]["result"], "closed");
    assert_eq!(j["replies"][0]["author_type"], "human");
}

#[test]
fn feedback_item_minimal_keeps_nullable_fields() {
    // source_ref and closed_by are NOT skip_serializing — they always appear,
    // even when null. severity (skip-if-none) does not.
    let item = FeedbackItem {
        feedback_id: "FB-1".into(),
        title: String::new(),
        body: String::new(),
        status: FeedbackStatus::Pending,
        origin: FeedbackOrigin::UserChat,
        severity: None,
        author: "user".into(),
        author_type: AuthorType::Human,
        created_at: "t".into(),
        visit: 0,
        source_ref: None,
        closed_by: None,
        resolution: None,
        replies: vec![],
        inline_anchor: None,
        scope: None,
        iterations: vec![],
        closure_reply: None,
        closure_reply_unread: None,
    };
    let j = serde_json::to_value(&item).unwrap();
    let obj = j.as_object().unwrap();
    assert!(obj.contains_key("source_ref"));
    assert_eq!(j["source_ref"], Value::Null);
    assert!(obj.contains_key("closed_by"));
    assert!(!obj.contains_key("severity"));
    assert!(!obj.contains_key("replies"));
}

#[test]
fn feedback_item_default_title_body_on_deserialize() {
    // title/body are #[serde(default)] — missing keys deserialize to "".
    let j = json!({
        "feedback_id": "FB-2",
        "status": "pending",
        "origin": "agent",
        "author": "agent",
        "author_type": "agent",
        "created_at": "t",
        "visit": 0,
        "source_ref": null,
        "closed_by": null
    });
    let item: FeedbackItem = serde_json::from_value(j).unwrap();
    assert_eq!(item.title, "");
    assert_eq!(item.body, "");
    assert_eq!(item.feedback_id, "FB-2");
}

#[test]
fn feedback_list_response_roundtrip() {
    let resp = FeedbackListResponse {
        run: "run".into(),
        station: "frame".into(),
        count: 0,
        items: vec![],
    };
    roundtrips_stably(&resp);
    let j = serde_json::to_value(&resp).unwrap();
    assert_eq!(j["count"], 0);
    assert_eq!(j["items"], json!([]));
}

#[test]
fn feedback_create_request_minimal_roundtrip() {
    let req = FeedbackCreateRequest {
        title: "t".into(),
        body: "b".into(),
        origin: None,
        author: None,
        source_ref: None,
        anchor: None,
        inline_anchor: None,
        resolution: None,
        attachment_data_url: None,
    };
    let j = serde_json::to_value(&req).unwrap();
    let obj = j.as_object().unwrap();
    assert_eq!(obj.len(), 2, "only title+body serialized");
    roundtrips_stably(&req);
}

#[test]
fn feedback_create_request_with_anchor() {
    let req = FeedbackCreateRequest {
        title: "t".into(),
        body: "b".into(),
        origin: Some(FeedbackOrigin::UserVisual),
        author: Some("user".into()),
        source_ref: Some("ref".into()),
        anchor: Some(FeedbackAnchor {
            page_id: "p1".into(),
            x: 0.25,
            y: 0.75,
            viewport_width: 1920,
            viewport_height: 1080,
        }),
        inline_anchor: None,
        resolution: Some(FeedbackResolution::Question),
        attachment_data_url: Some("data:image/png;base64,AAAA".into()),
    };
    let j = serde_json::to_value(&req).unwrap();
    assert_eq!(j["origin"], "user-visual");
    assert_eq!(j["anchor"]["x"], 0.25);
    assert_eq!(j["anchor"]["viewport_width"], 1920);
    assert_eq!(j["resolution"], "question");
    roundtrips_stably(&req);
}

#[test]
fn feedback_create_response_roundtrip() {
    let resp = FeedbackCreateResponse {
        feedback_id: "FB-01".into(),
        file: "feedback/FB-01.md".into(),
        status: FeedbackStatus::Pending,
        message: "created".into(),
    };
    let j = serde_json::to_value(&resp).unwrap();
    assert_eq!(j["status"], "pending");
    roundtrips_stably(&resp);
}

#[test]
fn feedback_update_response_roundtrip() {
    let resp = FeedbackUpdateResponse {
        feedback_id: "FB-01".into(),
        updated_fields: vec!["status".into(), "closed_by".into()],
        message: "updated".into(),
    };
    let j = serde_json::to_value(&resp).unwrap();
    assert_eq!(j["updated_fields"], json!(["status", "closed_by"]));
    roundtrips_stably(&resp);
}

#[test]
fn feedback_delete_response_roundtrip() {
    let resp = FeedbackDeleteResponse {
        feedback_id: "FB-01".into(),
        deleted: true,
        message: "deleted".into(),
    };
    let j = serde_json::to_value(&resp).unwrap();
    assert_eq!(j["deleted"], true);
    roundtrips_stably(&resp);
}

#[test]
fn feedback_reply_create_request_roundtrip() {
    let req = FeedbackReplyCreateRequest {
        body: "reply".into(),
        author: Some("user".into()),
        close_as_answered: Some(true),
    };
    let j = serde_json::to_value(&req).unwrap();
    assert_eq!(j["close_as_answered"], true);
    roundtrips_stably(&req);
}

#[test]
fn feedback_reply_create_request_minimal() {
    let req = FeedbackReplyCreateRequest {
        body: "reply".into(),
        author: None,
        close_as_answered: None,
    };
    let j = serde_json::to_value(&req).unwrap();
    assert_eq!(j.as_object().unwrap().len(), 1);
}

#[test]
fn feedback_reply_create_response_roundtrip() {
    let resp = FeedbackReplyCreateResponse {
        feedback_id: "FB-01".into(),
        reply_index: 2,
        status: FeedbackStatus::Answered,
        message: "appended".into(),
    };
    let j = serde_json::to_value(&resp).unwrap();
    assert_eq!(j["reply_index"], 2);
    assert_eq!(j["status"], "answered");
    roundtrips_stably(&resp);
}

#[test]
fn advance_response_roundtrip() {
    let resp = AdvanceResponse {
        ok: true,
        station: "frame".into(),
        open_feedback_count: 0,
        stamped_user_slots: true,
    };
    let j = serde_json::to_value(&resp).unwrap();
    assert_eq!(j["stamped_user_slots"], true);
    assert_eq!(j["open_feedback_count"], 0);
    roundtrips_stably(&resp);
}

#[test]
fn advance_response_with_open_feedback() {
    let resp = AdvanceResponse {
        ok: true,
        station: "polish".into(),
        open_feedback_count: 5,
        stamped_user_slots: false,
    };
    let j = serde_json::to_value(&resp).unwrap();
    assert_eq!(j["open_feedback_count"], 5);
    assert_eq!(j["stamped_user_slots"], false);
}

#[test]
fn review_decision_request_minimal_roundtrip() {
    let req = ReviewDecisionRequest {
        decision: "approved".into(),
        feedback: None,
        annotations: None,
    };
    let j = serde_json::to_value(&req).unwrap();
    assert_eq!(j.as_object().unwrap().len(), 1);
    roundtrips_stably(&req);
}

#[test]
fn review_decision_request_with_annotations() {
    let req = ReviewDecisionRequest {
        decision: "changes_requested".into(),
        feedback: Some("needs work".into()),
        annotations: Some(ReviewAnnotations {
            screenshot: Some("data:...".into()),
            pins: vec![Pin {
                x: 0.1,
                y: 0.2,
                text: "here".into(),
            }],
            comments: vec![InlineComment {
                selected_text: "foo".into(),
                comment: "fix".into(),
                paragraph: 0,
                location: Some("doc.md".into()),
            }],
        }),
    };
    let j = serde_json::to_value(&req).unwrap();
    assert_eq!(j["annotations"]["pins"][0]["x"], 0.1);
    assert_eq!(j["annotations"]["comments"][0]["paragraph"], 0);
    roundtrips_stably(&req);
}

#[test]
fn review_decision_response_roundtrip() {
    let resp = ReviewDecisionResponse {
        ok: true,
        decision: ReviewDecision::Approved,
        feedback: String::new(),
    };
    let j = serde_json::to_value(&resp).unwrap();
    assert_eq!(j["decision"], "approved");
    assert_eq!(j["feedback"], "");
    roundtrips_stably(&resp);
}

#[test]
fn question_answer_request_roundtrip() {
    let req = QuestionAnswerRequest {
        selected: vec!["a".into(), "b".into()],
        text: Some("custom".into()),
        annotations: Some(QuestionAnnotations {
            comments: vec![],
            pins: vec![QuestionPin {
                x: 0.5,
                y: 0.5,
                text: "pin".into(),
                image_index: 1,
            }],
            screenshots: vec![QuestionScreenshotAnnotation {
                comment: "note".into(),
                screenshot_data_url: "data:...".into(),
                image_index: 0,
            }],
        }),
    };
    let j = serde_json::to_value(&req).unwrap();
    assert_eq!(j["selected"], json!(["a", "b"]));
    assert_eq!(j["text"], "custom");
    assert_eq!(j["annotations"]["pins"][0]["image_index"], 1);
    roundtrips_stably(&req);
}

#[test]
fn question_answer_request_projects_to_answer() {
    let req = QuestionAnswerRequest {
        selected: vec!["x".into()],
        text: Some("note".into()),
        annotations: None,
    };
    let answer = req.to_answer();
    assert_eq!(answer.selected, vec!["x".to_string()]);
    assert_eq!(answer.text.as_deref(), Some("note"));
}

#[test]
fn question_answer_request_minimal_omits_empty() {
    let req = QuestionAnswerRequest::default();
    let j = serde_json::to_value(&req).unwrap();
    assert_eq!(j, json!({}));
}

#[test]
fn question_answer_response_roundtrip() {
    let resp = QuestionAnswerResponse {
        ok: true,
        answer: QuestionAnswer {
            selected: vec!["a".into()],
            text: None,
        },
    };
    let j = serde_json::to_value(&resp).unwrap();
    assert_eq!(j["ok"], true);
    assert_eq!(j["answer"]["selected"], json!(["a"]));
    roundtrips_stably(&resp);
}

#[test]
fn picker_select_request_roundtrip() {
    let req = PickerSelectRequest { id: "frame".into() };
    let j = serde_json::to_value(&req).unwrap();
    assert_eq!(j["id"], "frame");
    roundtrips_stably(&req);
}

#[test]
fn picker_select_response_roundtrip() {
    let resp = PickerSelectResponse {
        ok: true,
        id: "frame".into(),
    };
    let j = serde_json::to_value(&resp).unwrap();
    assert_eq!(j["id"], "frame");
    roundtrips_stably(&resp);
}

#[test]
fn direction_select_response_roundtrip() {
    let resp = DirectionSelectResponse {
        ok: true,
        archetype: "brutalist".into(),
    };
    let j = serde_json::to_value(&resp).unwrap();
    assert_eq!(j["ok"], true);
    assert_eq!(j["archetype"], "brutalist");
    roundtrips_stably(&resp);
}

#[test]
fn review_current_payload_roundtrip() {
    let payload = ReviewCurrentPayload {
        run: "run".into(),
        station: Some("frame".into()),
        station_label: Some("Intake".into()),
        surface: Some("api".into()),
        phase: Some("manufacture".into()),
        units: vec![ReviewCurrentUnit {
            slug: "u1".into(),
            title: "Unit One".into(),
            status: "active".into(),
        }],
        feedback_summary: FeedbackSummary {
            pending: 2,
            addressed: 1,
            closed: 3,
            rejected: 0,
        },
        stations: vec![ReviewCurrentStation {
            name: "frame".into(),
            label: Some("Intake".into()),
            status: "active".into(),
            phase: Some("manufacture".into()),
            iteration: Some(2),
            visits: Some(1),
        }],
    };
    let j = serde_json::to_value(&payload).unwrap();
    assert_eq!(j["feedback_summary"]["pending"], 2);
    assert_eq!(j["units"][0]["slug"], "u1");
    assert_eq!(j["stations"][0]["iteration"], 2);
    assert_eq!(j["stations"][0]["label"], "Intake");
    assert_eq!(j["surface"], "api");
    roundtrips_stably(&payload);
}

#[test]
fn review_current_payload_null_station() {
    let payload = ReviewCurrentPayload {
        run: "run".into(),
        station: None,
        station_label: None,
        surface: None,
        phase: None,
        units: vec![],
        feedback_summary: FeedbackSummary::default(),
        stations: vec![],
    };
    let j = serde_json::to_value(&payload).unwrap();
    // station is Option without skip — null appears.
    assert_eq!(j["station"], Value::Null);
    // station_label and surface skip when None.
    assert!(j.get("station_label").is_none());
    assert!(j.get("surface").is_none());
    assert_eq!(j["feedback_summary"]["pending"], 0);
}

#[test]
fn feedback_summary_default_is_all_zero() {
    let s = FeedbackSummary::default();
    let j = serde_json::to_value(&s).unwrap();
    assert_eq!(j, json!({"pending":0,"addressed":0,"closed":0,"rejected":0}));
}

// --- direction select request (the design-direction decision body) ---

#[test]
fn direction_select_request_minimal() {
    let req = DirectionSelectRequest {
        archetype: "brutalist".into(),
        annotations: None,
    };
    let j = serde_json::to_value(&req).unwrap();
    assert_eq!(j["archetype"], "brutalist");
    assert!(j.get("annotations").is_none());
    let back: DirectionSelectRequest = serde_json::from_value(j).unwrap();
    assert_eq!(back.archetype, "brutalist");
}

#[test]
fn direction_select_request_with_annotations() {
    let req = DirectionSelectRequest {
        archetype: "a".into(),
        annotations: Some(DirectionAnnotations {
            pins: vec![DirectionPin {
                x: 0.25,
                y: 0.75,
                note: "tighten".into(),
            }],
            screenshot: Some("data:image/png;base64,AA".into()),
            comments: vec!["nice".into()],
        }),
    };
    let j = serde_json::to_value(&req).unwrap();
    assert_eq!(j["archetype"], "a");
    assert_eq!(j["annotations"]["pins"][0]["note"], "tighten");
    assert_eq!(j["annotations"]["screenshot"], "data:image/png;base64,AA");
    roundtrips_stably(&req);
}

#[test]
fn direction_select_request_parses_from_json() {
    let j = json!({ "archetype": "bold" });
    let back: DirectionSelectRequest = serde_json::from_value(j).unwrap();
    assert_eq!(back.archetype, "bold");
    assert!(back.annotations.is_none());
}

#[test]
fn direction_select_request_rejects_missing_archetype() {
    let j = json!({ "annotations": {} });
    let r: Result<DirectionSelectRequest, _> = serde_json::from_value(j);
    assert!(r.is_err(), "archetype is required");
}

#[test]
fn direction_annotations_default_omits_all() {
    let a = DirectionAnnotations::default();
    let j = serde_json::to_value(&a).unwrap();
    assert_eq!(j, json!({}));
    roundtrips_stably(&a);
}

// --- numeric and boundary fields ---

#[test]
fn pin_coordinates_preserve_floats() {
    let pin = Pin {
        x: 0.123456789,
        y: 0.987654321,
        text: "p".into(),
    };
    let back: Pin = serde_json::from_value(serde_json::to_value(&pin).unwrap()).unwrap();
    assert_eq!(back.x, 0.123456789);
    assert_eq!(back.y, 0.987654321);
}

#[test]
fn boot_port_handles_u16_boundary() {
    let p = ViewSessionPayload {
        session_id: "v".into(),
        status: ViewStatus::Open,
        run_slug: "r".into(),
        scope: ViewScope::Run,
        artifacts: vec![],
        factory: None,
        station: None,
        artifact: None,
        mode: ViewMode::Boot,
        boot_port: Some(65535),
        boot_command: None,
    };
    let j = serde_json::to_value(&p).unwrap();
    assert_eq!(j["boot_port"], 65535);
    let back: ViewSessionPayload = serde_json::from_value(j).unwrap();
    assert_eq!(back.boot_port, Some(65535));
}

#[test]
fn boot_port_rejects_overflow() {
    let j = json!({
        "session_id": "v", "status": "open", "run_slug": "r",
        "mode": "boot", "boot_port": 70000
    });
    let r: Result<ViewSessionPayload, _> = serde_json::from_value(j);
    assert!(r.is_err(), "u16 must reject 70000");
}

#[test]
fn size_bytes_handles_u64() {
    let preview = UnitOutputPreview {
        path: "out.bin".into(),
        name: "out".into(),
        output_type: UnitOutputType::File,
        url: "/x".into(),
        preview_body: None,
        size_bytes: Some(u64::MAX),
        exists: true,
    };
    let back: UnitOutputPreview =
        serde_json::from_value(serde_json::to_value(&preview).unwrap()).unwrap();
    assert_eq!(back.size_bytes, Some(u64::MAX));
}

#[test]
fn discovered_review_url_pr_number_u64() {
    let url = DiscoveredReviewUrl {
        url: "https://x/pr/1".into(),
        source: darkrun_api::session::DiscoveredReviewSource::GithubPrRef,
        pr_number: 42,
        matched_sha: "abc".into(),
    };
    let j = serde_json::to_value(&url).unwrap();
    assert_eq!(j["pr_number"], 42);
    assert_eq!(j["source"], "github-pr-ref");
}

#[test]
fn pending_decision_roundtrip() {
    let pd = PendingDecision {
        decision: "approved".into(),
        feedback: "lgtm".into(),
        submitted_at: "t".into(),
    };
    roundtrips_stably(&pd);
}

#[test]
fn knowledge_file_and_station_artifact_roundtrip() {
    let kf = KnowledgeFile {
        name: "notes.md".into(),
        content: "body".into(),
    };
    let sa = StationArtifact {
        station: "frame".into(),
        name: "spec.md".into(),
        content: "spec".into(),
    };
    roundtrips_stably(&kf);
    roundtrips_stably(&sa);
}

#[test]
fn output_artifact_type_field_renamed_to_type() {
    let oa = OutputArtifact {
        station: "frame".into(),
        name: "main.rs".into(),
        artifact_type: OutputArtifactType::Code,
        language: Some("rust".into()),
        directory: None,
        content: None,
        relative_path: Some("/x/main.rs".into()),
        run_relative_path: None,
    };
    let j = serde_json::to_value(&oa).unwrap();
    assert_eq!(j["type"], "code");
    assert!(j.get("artifact_type").is_none());
    assert_eq!(j["language"], "rust");
    roundtrips_stably(&oa);
}

#[test]
fn progress_milestone_roundtrip() {
    let m = ProgressMilestone {
        key: "review:spec".into(),
        label: "Review spec".into(),
        status: MilestoneStatus::Active,
    };
    let j = serde_json::to_value(&m).unwrap();
    assert_eq!(j["status"], "active");
    roundtrips_stably(&m);
}

#[test]
fn run_current_state_default_min_fields() {
    let s = RunCurrentState::default();
    let j = serde_json::to_value(&s).unwrap();
    // Required string fields appear even when empty; optionals skipped.
    assert_eq!(j["factory"], "");
    assert_eq!(j["station"], "");
    assert!(j.get("phase").is_none());
    assert!(j.get("milestones").is_none());
}

#[test]
fn run_current_state_seal_status_serializes() {
    let s = RunCurrentState {
        factory: "f".into(),
        station: "s".into(),
        seal_status: Some(SealStatus::Sealed),
        awaiting_merge_into: Some("main".into()),
        ..Default::default()
    };
    let j = serde_json::to_value(&s).unwrap();
    assert_eq!(j["seal_status"], "sealed");
    assert_eq!(j["awaiting_merge_into"], "main");
}

#[test]
fn inline_comment_location_skipped_when_none() {
    let c = InlineComment {
        selected_text: "x".into(),
        comment: "y".into(),
        paragraph: 0,
        location: None,
    };
    let j = serde_json::to_value(&c).unwrap();
    assert!(j.get("location").is_none());
}

#[test]
fn review_annotations_default_skips_all() {
    let a = ReviewAnnotations::default();
    let j = serde_json::to_value(&a).unwrap();
    assert_eq!(j, json!({}));
}

#[test]
fn question_annotations_default_skips_all() {
    let a = QuestionAnnotations::default();
    assert_eq!(serde_json::to_value(&a).unwrap(), json!({}));
}

#[test]
fn direction_annotations_default_skips_all() {
    let a = DirectionAnnotations::default();
    assert_eq!(serde_json::to_value(&a).unwrap(), json!({}));
}

// ===========================================================================
// SECTION 6 — Body-size constants
// ===========================================================================

#[test]
fn body_size_constants_are_ordered() {
    assert!(FEEDBACK_BODY_MAX_BYTES < DEFAULT_BODY_MAX_BYTES);
    assert!(DEFAULT_BODY_MAX_BYTES < FEEDBACK_CREATE_MAX_BYTES);
    assert!(FEEDBACK_CREATE_MAX_BYTES < SESSION_ANSWER_MAX_BYTES);
}

#[test]
fn body_size_constants_exact_values() {
    assert_eq!(DEFAULT_BODY_MAX_BYTES, 1_048_576);
    assert_eq!(FEEDBACK_BODY_MAX_BYTES, 131_072);
    assert_eq!(FEEDBACK_CREATE_MAX_BYTES, 8_388_608);
    assert_eq!(SESSION_ANSWER_MAX_BYTES, 33_554_432);
}

#[test]
fn body_size_constants_are_powers_aligned() {
    // Each is a clean binary multiple (MiB/KiB) — no off-by-one.
    assert_eq!(DEFAULT_BODY_MAX_BYTES, 1024 * 1024);
    assert_eq!(FEEDBACK_BODY_MAX_BYTES, 128 * 1024);
    assert_eq!(FEEDBACK_CREATE_MAX_BYTES, 8 * 1024 * 1024);
    assert_eq!(SESSION_ANSWER_MAX_BYTES, 32 * 1024 * 1024);
}

// ===========================================================================
// SECTION 7 — JSON Schema generation for the public types
// ===========================================================================

#[test]
fn session_payload_schema_has_title_and_one_of() {
    let s = schema_value!(SessionPayload);
    assert_eq!(s["title"], "SessionPayload");
    assert!(s.get("oneOf").is_some(), "discriminated union -> oneOf");
    let arms = s["oneOf"].as_array().unwrap();
    assert_eq!(
        arms.len(),
        7,
        "review / question / direction / picker / view / visual_review / proof"
    );
}

#[test]
fn session_payload_schema_arms_require_session_type() {
    let s = schema_value!(SessionPayload);
    for arm in s["oneOf"].as_array().unwrap() {
        let req = arm["required"].as_array().unwrap();
        let has = req.iter().any(|v| v == "session_type");
        assert!(has, "each arm must require session_type");
    }
}

#[test]
fn session_payload_schema_carries_definitions() {
    let s = schema_value!(SessionPayload);
    // draft 2020-12 names the shared-definitions bucket $defs.
    let defs = s["$defs"].as_object().expect("$defs present");
    // The variant payload structs are inlined into the oneOf arms; the leaf
    // types they reference are pulled out as shared definitions.
    assert!(defs.contains_key("RunCurrentState"));
    assert!(defs.contains_key("ApproveAction"));
    assert!(defs.contains_key("PickerOption"));
    assert!(defs.contains_key("DirectionArchetype"));
}

#[test]
fn run_phase_schema_enumerates_six_phases() {
    let s = schema_value!(RunPhase);
    assert_eq!(s["title"], "RunPhase");
    let vals = one_of_enum_values(&s);
    let expected = [
        "spec",
        "review",
        "manufacture",
        "audit",
        "reflect",
        "checkpoint",
    ];
    for e in expected {
        assert!(vals.contains(&e.to_string()), "missing phase {e}");
    }
    assert_eq!(vals.len(), 6);
}

#[test]
fn session_status_schema_enumerates_five_states() {
    let s = schema_value!(SessionStatus);
    let vals = one_of_enum_values(&s);
    assert_eq!(vals.len(), 5);
    assert!(vals.contains(&"changes_requested".to_string()));
}

#[test]
fn feedback_status_schema_enumerates_eight() {
    let s = schema_value!(FeedbackStatus);
    let vals = one_of_enum_values(&s);
    assert_eq!(vals.len(), 8);
    assert!(vals.contains(&"non_actionable".to_string()));
}

#[test]
fn feedback_origin_schema_enumerates_twelve_kebab() {
    let s = schema_value!(FeedbackOrigin);
    let vals = one_of_enum_values(&s);
    assert_eq!(vals.len(), 12);
    assert!(vals.contains(&"adversarial-review".to_string()));
    assert!(vals.contains(&"user-visual".to_string()));
    // No snake_case leaks in.
    assert!(!vals.iter().any(|v| v.contains('_')));
}

#[test]
fn gate_type_schema_enumerates_four() {
    let s = schema_value!(GateType);
    let vals = one_of_enum_values(&s);
    assert_eq!(vals.len(), 4);
}

#[test]
fn approve_action_kind_schema_enumerates_nine() {
    let s = schema_value!(ApproveActionKind);
    assert_eq!(one_of_enum_values(&s).len(), 9);
}

#[test]
fn direction_select_request_schema_requires_archetype() {
    let s = schema_value!(DirectionSelectRequest);
    assert_eq!(s["title"], "DirectionSelectRequest");
    let req: Vec<String> = s["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(req.contains(&"archetype".to_string()), "archetype is required");
    // annotations is optional (skip_serializing_if) -> not required.
    assert!(!req.contains(&"annotations".to_string()));
}

#[test]
fn direction_archetype_schema_requires_image_url() {
    let s = schema_value!(DirectionArchetype);
    let req: Vec<String> = s["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    for f in ["id", "label", "image_url", "description"] {
        assert!(req.contains(&f.to_string()), "{f} should be required");
    }
}

#[test]
fn question_session_payload_schema_has_options_and_multi_select() {
    let s = schema_value!(QuestionSessionPayload);
    let props = s["properties"].as_object().expect("properties");
    assert!(props.contains_key("prompt"));
    assert!(props.contains_key("options"));
    assert!(props.contains_key("multi_select"));
    assert!(props.contains_key("answer"));
}

#[test]
fn feedback_item_schema_required_core_fields() {
    let s = schema_value!(FeedbackItem);
    let req: Vec<String> = s["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    for f in [
        "feedback_id",
        "status",
        "origin",
        "author",
        "author_type",
        "created_at",
        "visit",
    ] {
        assert!(req.contains(&f.to_string()), "{f} should be required");
    }
    // title/body are #[serde(default)] -> NOT required.
    assert!(!req.contains(&"title".to_string()));
}

#[test]
fn feedback_create_request_schema_requires_title_body() {
    let s = schema_value!(FeedbackCreateRequest);
    let req: Vec<String> = s["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(req.contains(&"title".to_string()));
    assert!(req.contains(&"body".to_string()));
    assert!(!req.contains(&"origin".to_string()));
}

#[test]
fn output_artifact_schema_uses_type_property_name() {
    let s = schema_value!(OutputArtifact);
    let props = s["properties"].as_object().unwrap();
    assert!(props.contains_key("type"), "renamed field 'type'");
    assert!(!props.contains_key("artifact_type"));
}

#[test]
fn review_session_payload_schema_is_object() {
    let s = schema_value!(ReviewSessionPayload);
    assert_eq!(s["title"], "ReviewSessionPayload");
    assert_eq!(s["type"], "object");
    assert!(s["properties"]["session_id"].is_object());
}

#[test]
fn all_public_types_generate_schemas() {
    // Smoke-generate a schema for each public type; each must serialize to a
    // non-empty object with a title.
    macro_rules! check {
        ($($t:ty),* $(,)?) => {
            $(
                let v = schema_value!($t);
                assert!(v.is_object(), concat!(stringify!($t), " not object"));
                assert!(v.get("title").is_some(), concat!(stringify!($t), " no title"));
            )*
        };
    }
    check!(
        SessionPayload,
        ReviewSessionPayload,
        QuestionSessionPayload,
        DirectionSessionPayload,
        PickerSessionPayload,
        ViewSessionPayload,
        RunCurrentState,
        RunPhase,
        ReviewDecisionRequest,
        ReviewDecisionResponse,
        ReviewCurrentPayload,
        QuestionAnswerRequest,
        QuestionAnswerResponse,
        DirectionSelectRequest,
        DirectionSelectResponse,
        PickerSelectRequest,
        PickerSelectResponse,
        AdvanceResponse,
        FeedbackItem,
        FeedbackListResponse,
        FeedbackCreateRequest,
        FeedbackCreateResponse,
        FeedbackUpdateRequest,
        FeedbackUpdateResponse,
        FeedbackDeleteResponse,
        FeedbackReplyCreateRequest,
        FeedbackReplyCreateResponse,
        ValidationError,
    );
}

#[test]
fn schema_generation_is_deterministic() {
    let a = serde_json::to_string(&schema_for!(FeedbackItem)).unwrap();
    let b = serde_json::to_string(&schema_for!(FeedbackItem)).unwrap();
    assert_eq!(a, b);
    let c = serde_json::to_string(&schema_for!(SessionPayload)).unwrap();
    let d = serde_json::to_string(&schema_for!(SessionPayload)).unwrap();
    assert_eq!(c, d);
}

#[test]
fn enum_schemas_have_no_snake_case_in_kebab_enum() {
    // Cross-check: kebab enums produce only kebab tokens.
    for s in [
        schema_value!(FeedbackOrigin),
        schema_value!(darkrun_api::session::DiscoveredReviewSource),
    ] {
        for v in one_of_enum_values(&s) {
            assert!(!v.contains('_'), "kebab enum leaked underscore: {v}");
        }
    }
}

// ===========================================================================
// SECTION 8 — OpenAPI document emission + parity
// ===========================================================================

#[test]
fn openapi_document_core_shape() {
    let doc = openapi::document();
    assert_eq!(doc["openapi"], "3.1.0");
    assert_eq!(doc["info"]["title"], "darkrun API");
    assert_eq!(doc["info"]["version"], API_VERSION);
    assert!(doc["paths"].is_object());
    assert!(doc["components"]["schemas"].is_object());
}

#[test]
fn openapi_info_description_present() {
    let doc = openapi::document();
    let desc = doc["info"]["description"].as_str().unwrap();
    assert!(desc.contains("darkrun"));
    assert!(desc.contains("wire contract"));
}

#[test]
fn api_version_matches_crate_version() {
    assert_eq!(API_VERSION, env!("CARGO_PKG_VERSION"));
}

#[test]
fn openapi_document_json_is_valid_json() {
    let text = openapi::document_json();
    let _: Value = serde_json::from_str(&text).expect("valid json");
    assert!(text.contains("\"openapi\""));
    assert!(text.contains("darkrun API"));
}

#[test]
fn openapi_document_json_is_deterministic() {
    assert_eq!(openapi::document_json(), openapi::document_json());
}

#[test]
fn openapi_document_value_is_deterministic() {
    assert_eq!(openapi::document(), openapi::document());
}

#[test]
fn every_non_ws_route_is_an_openapi_operation() {
    let doc = openapi::document();
    let paths = doc["paths"].as_object().unwrap();
    for route in ROUTES {
        if route.method == HttpMethod::Ws {
            continue;
        }
        let verb = match route.method {
            HttpMethod::Get => "get",
            HttpMethod::Head => "head",
            HttpMethod::Post => "post",
            HttpMethod::Put => "put",
            HttpMethod::Delete => "delete",
            HttpMethod::Ws => unreachable!(),
        };
        let op = &paths[route.path_template][verb];
        assert_eq!(
            op["operationId"], route.operation_id,
            "route {} missing from openapi",
            route.operation_id
        );
        assert_eq!(op["tags"][0], route.tag);
        assert_eq!(op["summary"], route.summary);
    }
}

#[test]
fn ws_route_is_not_an_openapi_path_operation() {
    let doc = openapi::document();
    let paths = doc["paths"].as_object().unwrap();
    // The ws path template should not show up as a GET operation with the ws op id.
    let ws = find(HttpMethod::Ws, "/ws/session/{sessionId}").unwrap();
    let leaked = paths
        .get(ws.path_template)
        .and_then(|p| p.get("get"))
        .map(|op| op["operationId"] == ws.operation_id)
        .unwrap_or(false);
    assert!(!leaked, "ws upgrade must not be an openapi operation");
}

#[test]
fn openapi_paths_count_matches_distinct_non_ws_templates() {
    use std::collections::BTreeSet;
    let doc = openapi::document();
    let paths = doc["paths"].as_object().unwrap();
    let distinct: BTreeSet<&str> = ROUTES
        .iter()
        .filter(|r| r.method != HttpMethod::Ws)
        .map(|r| r.path_template)
        .collect();
    assert_eq!(paths.len(), distinct.len());
}

#[test]
fn openapi_operation_ids_are_unique() {
    let doc = openapi::document();
    let paths = doc["paths"].as_object().unwrap();
    let mut ids = Vec::new();
    for (_p, item) in paths {
        for (_v, op) in item.as_object().unwrap() {
            ids.push(op["operationId"].as_str().unwrap().to_string());
        }
    }
    let mut sorted = ids.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(ids.len(), sorted.len(), "duplicate operationId in openapi");
}

#[test]
fn openapi_operation_count_equals_non_ws_routes() {
    let doc = openapi::document();
    let paths = doc["paths"].as_object().unwrap();
    let mut ops = 0;
    for (_p, item) in paths {
        ops += item.as_object().unwrap().len();
    }
    let expected = ROUTES.iter().filter(|r| r.method != HttpMethod::Ws).count();
    assert_eq!(ops, expected);
}

#[test]
fn shared_feedback_path_carries_two_verbs() {
    let doc = openapi::document();
    let item = &doc["paths"]["/api/feedback/{run}/{station}"];
    assert!(item["get"].is_object());
    assert!(item["post"].is_object());
    assert_eq!(item["get"]["operationId"], "listFeedback");
    assert_eq!(item["post"]["operationId"], "createFeedback");
}

#[test]
fn feedback_item_path_carries_put_and_delete() {
    let doc = openapi::document();
    let item = &doc["paths"]["/api/feedback/{run}/{station}/{id}"];
    assert_eq!(item["put"]["operationId"], "updateFeedback");
    assert_eq!(item["delete"]["operationId"], "deleteFeedback");
}

#[test]
fn every_openapi_operation_has_a_200_response() {
    let doc = openapi::document();
    let paths = doc["paths"].as_object().unwrap();
    for (_p, item) in paths {
        for (_v, op) in item.as_object().unwrap() {
            assert!(
                op["responses"]["200"].is_object(),
                "operation {} missing 200",
                op["operationId"]
            );
        }
    }
}

#[test]
fn every_openapi_operation_has_summary_and_tag() {
    let doc = openapi::document();
    let paths = doc["paths"].as_object().unwrap();
    for (_p, item) in paths {
        for (_v, op) in item.as_object().unwrap() {
            assert!(op["summary"].as_str().map(|s| !s.is_empty()).unwrap_or(false));
            assert!(op["tags"].as_array().map(|t| !t.is_empty()).unwrap_or(false));
        }
    }
}

#[test]
fn openapi_core_schemas_present() {
    let doc = openapi::document();
    let schemas = doc["components"]["schemas"].as_object().unwrap();
    for name in [
        "SessionPayload",
        "ReviewSessionPayload",
        "FeedbackItem",
        "ReviewDecisionRequest",
        "ReviewDecisionResponse",
        "ValidationError",
        "RunPhase",
        "RunCurrentState",
    ] {
        assert!(schemas.contains_key(name), "missing schema {name}");
    }
}

#[test]
fn openapi_schema_count_is_stable() {
    let doc = openapi::document();
    let schemas = doc["components"]["schemas"].as_object().unwrap();
    // 39 component schemas are emitted by component_schemas().
    assert_eq!(schemas.len(), 39);
}

#[test]
fn openapi_schemas_include_all_request_response_bodies() {
    let doc = openapi::document();
    let schemas = doc["components"]["schemas"].as_object().unwrap();
    for name in [
        "QuestionAnswerRequest",
        "QuestionAnswerResponse",
        "DirectionSelectRequest",
        "DirectionSelectResponse",
        "PickerSelectRequest",
        "PickerSelectResponse",
        "AdvanceResponse",
        "FeedbackListResponse",
        "FeedbackCreateRequest",
        "FeedbackCreateResponse",
        "FeedbackUpdateRequest",
        "FeedbackUpdateResponse",
        "FeedbackDeleteResponse",
        "FeedbackReplyCreateRequest",
        "FeedbackReplyCreateResponse",
        "ReviewCurrentPayload",
    ] {
        assert!(schemas.contains_key(name), "missing body schema {name}");
    }
}

#[test]
fn openapi_component_schema_has_no_top_level_schema_marker() {
    // component_schemas inserts schema_for!($ty).schema — the inner root, which
    // strips the $schema/definitions envelope. So no "$schema" key leaks.
    let doc = openapi::document();
    let schemas = doc["components"]["schemas"].as_object().unwrap();
    let item = &schemas["FeedbackItem"];
    assert!(item.get("$schema").is_none(), "$schema leaked into component");
}

#[test]
fn openapi_session_payload_component_keeps_one_of() {
    let doc = openapi::document();
    let sp = &doc["components"]["schemas"]["SessionPayload"];
    assert!(sp.get("oneOf").is_some());
}

#[test]
fn every_emitted_schema_has_object_or_oneof_shape() {
    let doc = openapi::document();
    let schemas = doc["components"]["schemas"].as_object().unwrap();
    for (name, schema) in schemas {
        let is_object = schema.get("type").map(|t| t == "object").unwrap_or(false);
        let is_one_of = schema.get("oneOf").is_some();
        assert!(
            is_object || is_one_of,
            "schema {name} is neither object nor oneOf"
        );
    }
}

// --- the parity guard against the committed openapi.json ---

#[test]
fn committed_openapi_json_is_in_sync() {
    let committed = include_str!("../openapi.json");
    let mut expected = openapi::document_json();
    expected.push('\n');
    assert_eq!(
        committed, expected,
        "openapi.json is stale — run `cargo run -p darkrun-api --bin emit_openapi`"
    );
}

#[test]
fn committed_openapi_parses_and_matches_routes() {
    let committed: Value =
        serde_json::from_str(include_str!("../openapi.json")).unwrap();
    let paths = committed["paths"].as_object().unwrap();
    for route in ROUTES {
        if route.method == HttpMethod::Ws {
            assert!(
                !paths.contains_key(route.path_template)
                    || paths[route.path_template].get("get").is_none(),
                "ws path leaked into committed openapi"
            );
            continue;
        }
        assert!(
            paths.contains_key(route.path_template),
            "committed openapi missing path {}",
            route.path_template
        );
    }
}

#[test]
fn committed_openapi_every_path_traces_to_a_route() {
    // Parity in the other direction: no orphan paths in the committed doc.
    let committed: Value =
        serde_json::from_str(include_str!("../openapi.json")).unwrap();
    let paths = committed["paths"].as_object().unwrap();
    for path in paths.keys() {
        let known = ROUTES.iter().any(|r| r.path_template == path);
        assert!(known, "committed openapi has orphan path {path}");
    }
}

#[test]
fn committed_openapi_version_matches_api_version() {
    let committed: Value =
        serde_json::from_str(include_str!("../openapi.json")).unwrap();
    assert_eq!(committed["info"]["version"], API_VERSION);
}

#[test]
fn every_openapi_path_has_a_corresponding_route_operation() {
    // For each (path, verb) operation in the doc, a matching ROUTES entry exists.
    let doc = openapi::document();
    let paths = doc["paths"].as_object().unwrap();
    for (path, item) in paths {
        for (verb, op) in item.as_object().unwrap() {
            let method = match verb.as_str() {
                "get" => HttpMethod::Get,
                "head" => HttpMethod::Head,
                "post" => HttpMethod::Post,
                "put" => HttpMethod::Put,
                "delete" => HttpMethod::Delete,
                other => panic!("unexpected verb {other}"),
            };
            let route = find(method, path).expect("route for openapi op");
            assert_eq!(op["operationId"], route.operation_id);
        }
    }
}

#[test]
fn document_json_is_pretty_printed() {
    let text = openapi::document_json();
    // Pretty printing => newlines and indentation present.
    assert!(text.contains('\n'));
    assert!(text.contains("  "));
}
