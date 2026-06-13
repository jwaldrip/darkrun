//! Comprehensive in-process route tests for darkrun-http — area "routes_core".
//!
//! Drives the public `build_router` surface via `tower::ServiceExt::oneshot`
//! (no socket bind) to exercise the core REST routes end to end:
//!   - `GET    /health`
//!   - `GET    /api/session/:id`            (present / absent / every variant)
//!   - `POST   /review/:id/decide`          (approve / request-changes / bad body)
//!   - `POST   /api/advance/:id`
//!
//! Asserts on status codes, content-types, JSON payload shapes, idempotency,
//! determinism, serde round-trips, CORS header presence/absence, and the
//! boundary conditions of the decision-canonicalization rule.

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use darkrun_api::{
    ApproveAction, ApproveActionKind, DirectionArchetype, DirectionSessionPayload, GateType,
    PickerKind, PickerOption, PickerSessionPayload, QuestionOption, QuestionSessionPayload,
    ReviewSessionPayload, SessionPayload, SessionStatus, ViewMode, ViewScope, ViewSessionPayload,
    ViewStatus,
};
use darkrun_core::StateStore;
use darkrun_http::{build_router, AppState, Limits};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

// ── Fixtures ────────────────────────────────────────────────────────────────

fn test_state() -> AppState {
    let tmp = tempfile::tempdir().expect("tmp");
    let store = StateStore::new(tmp.path());
    // Leak the tempdir guard so the dir survives the whole test process.
    std::mem::forget(tmp);
    AppState::new(store, Limits::default())
}

fn remote_state() -> AppState {
    let tmp = tempfile::tempdir().expect("tmp");
    let store = StateStore::new(tmp.path());
    std::mem::forget(tmp);
    let limits = Limits {
        remote: true,
        // Lift the per-IP ceiling so the in-process fallback-IP counter never
        // trips during these CORS / shape tests.
        rate_limit_per_min: 100_000,
        ..Limits::default()
    };
    AppState::new(store, limits)
}

fn review(session_id: &str) -> SessionPayload {
    SessionPayload::Review(ReviewSessionPayload {
        session_id: session_id.into(),
        status: SessionStatus::Pending,
        run_slug: Some("my-run".into()),
        gate_type: Some(GateType::Ask),
        station: Some("frame".into()),
        approve_action: Some(ApproveAction {
            label: "Complete Frame Station".into(),
            kind: ApproveActionKind::CompleteStation,
        }),
        await_active: Some(true),
        ..Default::default()
    })
}

fn question(session_id: &str) -> SessionPayload {
    SessionPayload::Question(QuestionSessionPayload {
        session_id: session_id.into(),
        status: SessionStatus::Pending,
        run_slug: None,
        title: Some("Pick a direction".into()),
        prompt: "Which?".into(),
        context: Some("Some context".into()),
        options: vec![
            QuestionOption {
                id: "A".into(),
                label: "A".into(),
                image_url: Some("/mock/a.png".into()),
                image_url_light: None,
                description: None,
            },
            QuestionOption {
                id: "B".into(),
                label: "B".into(),
                image_url: Some("/mock/b.png".into()),
                image_url_light: None,
                description: None,
            },
        ],
        multi_select: false,
        answer: None,
        image_urls: vec![],
    })
}

fn direction(session_id: &str) -> SessionPayload {
    SessionPayload::Direction(DirectionSessionPayload {
        session_id: session_id.into(),
        status: SessionStatus::Pending,
        title: Some("Design".into()),
        run_slug: Some("run-d".into()),
        prompt: "Pick a direction".into(),
        context: None,
        archetypes: vec![DirectionArchetype {
            id: "bold".into(),
            label: "Bold".into(),
            image_url: "/mock/bold.png".into(),
            image_url_light: None,
            description: "bold and loud".into(),
        }],
        chosen_archetype: None,
        annotations: None,
    })
}

fn picker(session_id: &str) -> SessionPayload {
    SessionPayload::Picker(PickerSessionPayload {
        session_id: session_id.into(),
        status: SessionStatus::Pending,
        run_slug: Some("run-p".into()),
        kind: PickerKind::Factory,
        title: "Pick a factory".into(),
        prompt: "Choose one".into(),
        options: vec![PickerOption {
            id: "f1".into(),
            label: "Factory One".into(),
            description: None,
            secondary: None,
        }],
        selection: None,
    })
}

fn view(session_id: &str) -> SessionPayload {
    SessionPayload::View(ViewSessionPayload {
        session_id: session_id.into(),
        status: ViewStatus::Open,
        run_slug: "run-v".into(),
        scope: ViewScope::Run,
        artifacts: vec![],
        factory: Some("web".into()),
        station: Some("frame".into()),
        artifact: None,
        mode: ViewMode::Viewer,
        boot_port: None,
        boot_command: None,
    })
}

// ── Request helpers ───────────────────────────────────────────────────────────

async fn send(app: axum::Router, req: Request<Body>) -> axum::response::Response {
    app.oneshot(req).await.unwrap()
}

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

fn post_json(uri: &str, v: &Value) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(v).unwrap()))
        .unwrap()
}

fn post_empty(uri: &str) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

async fn body_bytes(resp: axum::response::Response) -> Vec<u8> {
    resp.into_body().collect().await.unwrap().to_bytes().to_vec()
}

fn content_type(resp: &axum::response::Response) -> String {
    resp.headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string()
}

// ════════════════════════════════════════════════════════════════════════════
// GET /health
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn health_status_is_200() {
    let resp = send(build_router(test_state()), get("/health")).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn health_body_is_status_ok() {
    let resp = send(build_router(test_state()), get("/health")).await;
    let json = body_json(resp).await;
    assert_eq!(json["status"], "ok");
}

#[tokio::test]
async fn health_body_has_exactly_one_field() {
    let resp = send(build_router(test_state()), get("/health")).await;
    let json = body_json(resp).await;
    assert_eq!(json.as_object().unwrap().len(), 1);
}

#[tokio::test]
async fn health_content_type_is_json() {
    let resp = send(build_router(test_state()), get("/health")).await;
    assert!(content_type(&resp).starts_with("application/json"));
}

#[tokio::test]
async fn health_is_deterministic_across_calls() {
    let app = build_router(test_state());
    let a = body_bytes(send(app.clone(), get("/health")).await).await;
    let b = body_bytes(send(app, get("/health")).await).await;
    assert_eq!(a, b);
}

#[tokio::test]
async fn health_with_trailing_slash_is_404() {
    // axum does not treat `/health/` as `/health`.
    let resp = send(build_router(test_state()), get("/health/")).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn health_post_is_405() {
    let resp = send(build_router(test_state()), post_empty("/health")).await;
    assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn health_put_is_405() {
    let req = Request::builder()
        .method(Method::PUT)
        .uri("/health")
        .body(Body::empty())
        .unwrap();
    let resp = send(build_router(test_state()), req).await;
    assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn health_delete_is_405() {
    let req = Request::builder()
        .method(Method::DELETE)
        .uri("/health")
        .body(Body::empty())
        .unwrap();
    let resp = send(build_router(test_state()), req).await;
    assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn health_405_allow_header_lists_get() {
    let resp = send(build_router(test_state()), post_empty("/health")).await;
    let allow = resp
        .headers()
        .get(header::ALLOW)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(allow.contains("GET"), "allow header was {allow:?}");
}

#[tokio::test]
async fn health_ignores_request_body_on_get() {
    // A GET with a body should still be served (axum reads no body for health).
    let req = Request::builder()
        .uri("/health")
        .body(Body::from("ignored"))
        .unwrap();
    let resp = send(build_router(test_state()), req).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn health_repeated_100_times_all_ok_local() {
    let app = build_router(test_state());
    for _ in 0..100 {
        let resp = send(app.clone(), get("/health")).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }
}

// ════════════════════════════════════════════════════════════════════════════
// GET /api/session/:id — presence / absence
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn session_present_is_200() {
    let state = test_state();
    state.sessions.upsert(review("s-present"));
    let resp = send(build_router(state), get("/api/session/s-present")).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn session_present_content_type_is_json() {
    let state = test_state();
    state.sessions.upsert(review("s-ct"));
    let resp = send(build_router(state), get("/api/session/s-ct")).await;
    assert!(content_type(&resp).starts_with("application/json"));
}

#[tokio::test]
async fn session_absent_is_404() {
    let resp = send(build_router(test_state()), get("/api/session/nope")).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn session_absent_error_envelope() {
    let resp = send(build_router(test_state()), get("/api/session/ghost")).await;
    let json = body_json(resp).await;
    assert_eq!(json["error"], "session not found");
    assert_eq!(json["id"], "ghost");
}

#[tokio::test]
async fn session_absent_content_type_is_json() {
    let resp = send(build_router(test_state()), get("/api/session/x")).await;
    assert!(content_type(&resp).starts_with("application/json"));
}

#[tokio::test]
async fn session_echoes_requested_id_in_404() {
    for id in ["a", "b-c", "FB-99", "with.dot", "1234"] {
        let resp = send(build_router(test_state()), get(&format!("/api/session/{id}"))).await;
        let json = body_json(resp).await;
        assert_eq!(json["id"], id);
    }
}

#[tokio::test]
async fn session_review_payload_shape() {
    let state = test_state();
    state.sessions.upsert(review("r-1"));
    let resp = send(build_router(state), get("/api/session/r-1")).await;
    let json = body_json(resp).await;
    assert_eq!(json["session_type"], "review");
    assert_eq!(json["session_id"], "r-1");
    assert_eq!(json["status"], "pending");
    assert_eq!(json["gate_type"], "ask");
    assert_eq!(json["run_slug"], "my-run");
    assert_eq!(json["station"], "frame");
}

#[tokio::test]
async fn session_review_approve_action_nested_shape() {
    let state = test_state();
    state.sessions.upsert(review("r-aa"));
    let resp = send(build_router(state), get("/api/session/r-aa")).await;
    let json = body_json(resp).await;
    assert_eq!(json["approve_action"]["label"], "Complete Frame Station");
    assert_eq!(json["approve_action"]["kind"], "complete_station");
    assert_eq!(json["await_active"], true);
}

#[tokio::test]
async fn session_review_omits_empty_optionals() {
    let state = test_state();
    state.sessions.upsert(review("r-omit"));
    let resp = send(build_router(state), get("/api/session/r-omit")).await;
    let json = body_json(resp).await;
    // skip_serializing_if for None fields → keys absent entirely.
    assert!(json.get("decision").is_none());
    assert!(json.get("feedback").is_none());
    assert!(json.get("run_dir").is_none());
    // Vec/BTreeMap defaults skip too.
    assert!(json.get("units").is_none());
    assert!(json.get("station_states").is_none());
}

#[tokio::test]
async fn session_question_payload_shape() {
    let state = test_state();
    state.sessions.upsert(question("q-1"));
    let resp = send(build_router(state), get("/api/session/q-1")).await;
    let json = body_json(resp).await;
    assert_eq!(json["session_type"], "question");
    assert_eq!(json["session_id"], "q-1");
    assert_eq!(json["title"], "Pick a direction");
    assert_eq!(json["prompt"], "Which?");
    assert_eq!(json["options"][1]["id"], "B");
    assert_eq!(json["options"][1]["image_url"], "/mock/b.png");
}

#[tokio::test]
async fn session_direction_payload_shape() {
    let state = test_state();
    state.sessions.upsert(direction("d-1"));
    let resp = send(build_router(state), get("/api/session/d-1")).await;
    let json = body_json(resp).await;
    assert_eq!(json["session_type"], "direction");
    assert_eq!(json["session_id"], "d-1");
    assert_eq!(json["run_slug"], "run-d");
}

#[tokio::test]
async fn session_picker_payload_shape() {
    let state = test_state();
    state.sessions.upsert(picker("p-1"));
    let resp = send(build_router(state), get("/api/session/p-1")).await;
    let json = body_json(resp).await;
    assert_eq!(json["session_type"], "picker");
    assert_eq!(json["kind"], "factory");
    assert_eq!(json["title"], "Pick a factory");
    assert_eq!(json["prompt"], "Choose one");
    assert_eq!(json["options"][0]["id"], "f1");
}

#[tokio::test]
async fn session_view_payload_shape() {
    let state = test_state();
    state.sessions.upsert(view("v-1"));
    let resp = send(build_router(state), get("/api/session/v-1")).await;
    let json = body_json(resp).await;
    assert_eq!(json["session_type"], "view");
    assert_eq!(json["status"], "open");
    assert_eq!(json["mode"], "viewer");
    assert_eq!(json["run_slug"], "run-v");
    assert_eq!(json["factory"], "web");
}

#[tokio::test]
async fn session_payload_roundtrips_through_wire() {
    // The body the handler emits must deserialize back into a SessionPayload.
    let state = test_state();
    state.sessions.upsert(review("rt-1"));
    let resp = send(build_router(state), get("/api/session/rt-1")).await;
    let bytes = body_bytes(resp).await;
    let parsed: SessionPayload = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(parsed.session_id(), "rt-1");
    assert_eq!(parsed.session_type(), "review");
}

#[tokio::test]
async fn session_question_roundtrips_through_wire() {
    let state = test_state();
    state.sessions.upsert(question("rt-q"));
    let resp = send(build_router(state), get("/api/session/rt-q")).await;
    let bytes = body_bytes(resp).await;
    let parsed: SessionPayload = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(parsed.session_type(), "question");
}

#[tokio::test]
async fn session_view_roundtrips_through_wire() {
    let state = test_state();
    state.sessions.upsert(view("rt-v"));
    let resp = send(build_router(state), get("/api/session/rt-v")).await;
    let bytes = body_bytes(resp).await;
    let parsed: SessionPayload = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(parsed.session_type(), "view");
}

#[tokio::test]
async fn session_returns_latest_after_upsert_overwrite() {
    let state = test_state();
    state.sessions.upsert(review("ov"));
    // Overwrite with an approved status.
    let mut updated = review("ov");
    if let SessionPayload::Review(ref mut r) = updated {
        r.status = SessionStatus::Approved;
        r.decision = Some("approved".into());
    }
    state.sessions.upsert(updated);
    let resp = send(build_router(state), get("/api/session/ov")).await;
    let json = body_json(resp).await;
    assert_eq!(json["status"], "approved");
    assert_eq!(json["decision"], "approved");
}

#[tokio::test]
async fn session_distinct_ids_are_isolated() {
    let state = test_state();
    state.sessions.upsert(review("iso-a"));
    state.sessions.upsert(question("iso-b"));
    let app = build_router(state);

    let a = body_json(send(app.clone(), get("/api/session/iso-a")).await).await;
    let b = body_json(send(app, get("/api/session/iso-b")).await).await;
    assert_eq!(a["session_type"], "review");
    assert_eq!(b["session_type"], "question");
}

#[tokio::test]
async fn session_get_is_idempotent_and_nonmutating() {
    let state = test_state();
    state.sessions.upsert(review("idem"));
    let app = build_router(state.clone());
    for _ in 0..5 {
        let resp = send(app.clone(), get("/api/session/idem")).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }
    // Still pending — a GET never mutates.
    let SessionPayload::Review(r) = state.sessions.get("idem").unwrap() else {
        panic!("expected review");
    };
    assert_eq!(r.status, SessionStatus::Pending);
}

#[tokio::test]
async fn session_post_method_is_405() {
    let state = test_state();
    state.sessions.upsert(review("m1"));
    let resp = send(build_router(state), post_empty("/api/session/m1")).await;
    assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn session_id_with_url_unsafe_chars_404s_cleanly() {
    // A percent-encoded id that doesn't exist still maps to a clean 404.
    let resp = send(build_router(test_state()), get("/api/session/a%20b")).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn session_many_registered_each_resolvable() {
    let state = test_state();
    for i in 0..25 {
        state.sessions.upsert(review(&format!("multi-{i}")));
    }
    let app = build_router(state);
    for i in 0..25 {
        let resp = send(app.clone(), get(&format!("/api/session/multi-{i}"))).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["session_id"], format!("multi-{i}"));
    }
}

// ════════════════════════════════════════════════════════════════════════════
// POST /review/:id/decide — approve
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn decide_approve_status_200() {
    let state = test_state();
    state.sessions.upsert(review("ap-1"));
    let resp = send(
        build_router(state),
        post_json("/review/ap-1/decide", &json!({ "decision": "approved" })),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn decide_approve_response_shape() {
    let state = test_state();
    state.sessions.upsert(review("ap-2"));
    let resp = send(
        build_router(state),
        post_json(
            "/review/ap-2/decide",
            &json!({ "decision": "approved", "feedback": "ship it" }),
        ),
    )
    .await;
    let json = body_json(resp).await;
    assert_eq!(json["ok"], true);
    assert_eq!(json["decision"], "approved");
    assert_eq!(json["feedback"], "ship it");
}

#[tokio::test]
async fn decide_approve_mutates_session_status() {
    let state = test_state();
    state.sessions.upsert(review("ap-3"));
    let _ = send(
        build_router(state.clone()),
        post_json("/review/ap-3/decide", &json!({ "decision": "approved" })),
    )
    .await;
    let SessionPayload::Review(r) = state.sessions.get("ap-3").unwrap() else {
        panic!("expected review");
    };
    assert_eq!(r.status, SessionStatus::Approved);
    assert_eq!(r.decision.as_deref(), Some("approved"));
}

#[tokio::test]
async fn decide_approve_uppercase_canonicalizes() {
    let state = test_state();
    state.sessions.upsert(review("ap-up"));
    let resp = send(
        build_router(state),
        post_json("/review/ap-up/decide", &json!({ "decision": "APPROVED" })),
    )
    .await;
    let json = body_json(resp).await;
    assert_eq!(json["decision"], "approved");
}

#[tokio::test]
async fn decide_approve_mixed_case_canonicalizes() {
    for variant in ["Approved", "aPpRoVeD", "approveD"] {
        let state = test_state();
        state.sessions.upsert(review("mc"));
        let resp = send(
            build_router(state),
            post_json("/review/mc/decide", &json!({ "decision": variant })),
        )
        .await;
        let json = body_json(resp).await;
        assert_eq!(json["decision"], "approved", "variant {variant}");
    }
}

#[tokio::test]
async fn decide_approve_with_surrounding_whitespace_canonicalizes() {
    let state = test_state();
    state.sessions.upsert(review("ws"));
    let resp = send(
        build_router(state),
        post_json("/review/ws/decide", &json!({ "decision": "  approved  " })),
    )
    .await;
    let json = body_json(resp).await;
    assert_eq!(json["decision"], "approved");
}

#[tokio::test]
async fn decide_approve_content_type_is_json() {
    let state = test_state();
    state.sessions.upsert(review("ct"));
    let resp = send(
        build_router(state),
        post_json("/review/ct/decide", &json!({ "decision": "approved" })),
    )
    .await;
    assert!(content_type(&resp).starts_with("application/json"));
}

#[tokio::test]
async fn decide_response_roundtrips_into_typed() {
    use darkrun_api::{ReviewDecision, ReviewDecisionResponse};
    let state = test_state();
    state.sessions.upsert(review("typed"));
    let resp = send(
        build_router(state),
        post_json(
            "/review/typed/decide",
            &json!({ "decision": "approved", "feedback": "good" }),
        ),
    )
    .await;
    let bytes = body_bytes(resp).await;
    let parsed: ReviewDecisionResponse = serde_json::from_slice(&bytes).unwrap();
    assert!(parsed.ok);
    assert_eq!(parsed.decision, ReviewDecision::Approved);
    assert_eq!(parsed.feedback, "good");
}

#[tokio::test]
async fn decide_approve_feedback_persisted_on_session() {
    let state = test_state();
    state.sessions.upsert(review("fb-persist"));
    let _ = send(
        build_router(state.clone()),
        post_json(
            "/review/fb-persist/decide",
            &json!({ "decision": "approved", "feedback": "looks great" }),
        ),
    )
    .await;
    let SessionPayload::Review(r) = state.sessions.get("fb-persist").unwrap() else {
        panic!("expected review");
    };
    assert_eq!(r.feedback.as_deref(), Some("looks great"));
}

// ════════════════════════════════════════════════════════════════════════════
// POST /review/:id/decide — request changes
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn decide_changes_status_200() {
    let state = test_state();
    state.sessions.upsert(review("ch-1"));
    let resp = send(
        build_router(state),
        post_json("/review/ch-1/decide", &json!({ "decision": "changes_requested" })),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn decide_changes_response_shape() {
    let state = test_state();
    state.sessions.upsert(review("ch-2"));
    let resp = send(
        build_router(state),
        post_json(
            "/review/ch-2/decide",
            &json!({ "decision": "changes_requested", "feedback": "fix the spec" }),
        ),
    )
    .await;
    let json = body_json(resp).await;
    assert_eq!(json["ok"], true);
    assert_eq!(json["decision"], "changes_requested");
    assert_eq!(json["feedback"], "fix the spec");
}

#[tokio::test]
async fn decide_changes_mutates_session_status() {
    let state = test_state();
    state.sessions.upsert(review("ch-3"));
    let _ = send(
        build_router(state.clone()),
        post_json("/review/ch-3/decide", &json!({ "decision": "changes_requested" })),
    )
    .await;
    let SessionPayload::Review(r) = state.sessions.get("ch-3").unwrap() else {
        panic!("expected review");
    };
    assert_eq!(r.status, SessionStatus::ChangesRequested);
    assert_eq!(r.decision.as_deref(), Some("changes_requested"));
}

#[tokio::test]
async fn decide_arbitrary_string_coerced_to_changes() {
    // Anything that is not "approved" coerces to changes_requested.
    for raw in ["nope", "reject", "deny", "approve", "approvedd", "x", "0", "false"] {
        let state = test_state();
        state.sessions.upsert(review("coerce"));
        let resp = send(
            build_router(state),
            post_json("/review/coerce/decide", &json!({ "decision": raw })),
        )
        .await;
        let json = body_json(resp).await;
        assert_eq!(json["decision"], "changes_requested", "raw {raw:?}");
    }
}

#[tokio::test]
async fn decide_empty_decision_string_is_changes() {
    let state = test_state();
    state.sessions.upsert(review("empty-dec"));
    let resp = send(
        build_router(state),
        post_json("/review/empty-dec/decide", &json!({ "decision": "" })),
    )
    .await;
    let json = body_json(resp).await;
    assert_eq!(json["decision"], "changes_requested");
}

#[tokio::test]
async fn decide_no_feedback_echoes_empty_string() {
    let state = test_state();
    state.sessions.upsert(review("no-fb"));
    let resp = send(
        build_router(state),
        post_json("/review/no-fb/decide", &json!({ "decision": "nope" })),
    )
    .await;
    let json = body_json(resp).await;
    // feedback omitted in request → echoed as empty string in response.
    assert_eq!(json["feedback"], "");
}

#[tokio::test]
async fn decide_null_feedback_echoes_empty_string() {
    let state = test_state();
    state.sessions.upsert(review("null-fb"));
    let resp = send(
        build_router(state),
        post_json(
            "/review/null-fb/decide",
            &json!({ "decision": "nope", "feedback": null }),
        ),
    )
    .await;
    let json = body_json(resp).await;
    assert_eq!(json["feedback"], "");
}

#[tokio::test]
async fn decide_changes_with_annotations_persists() {
    let state = test_state();
    state.sessions.upsert(review("annot"));
    let body = json!({
        "decision": "changes_requested",
        "feedback": "see pins",
        "annotations": {
            "pins": [{ "x": 0.5, "y": 0.25, "text": "tighten here" }]
        }
    });
    let resp = send(build_router(state.clone()), post_json("/review/annot/decide", &body)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let SessionPayload::Review(r) = state.sessions.get("annot").unwrap() else {
        panic!("expected review");
    };
    let annotations = r.annotations.expect("annotations present");
    assert_eq!(annotations.pins.len(), 1);
    assert_eq!(annotations.pins[0].text, "tighten here");
}

// ════════════════════════════════════════════════════════════════════════════
// POST /review/:id/decide — errors / edges
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn decide_unknown_session_is_404() {
    let resp = send(
        build_router(test_state()),
        post_json("/review/ghost/decide", &json!({ "decision": "approved" })),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn decide_unknown_session_error_envelope() {
    let resp = send(
        build_router(test_state()),
        post_json("/review/ghost/decide", &json!({ "decision": "approved" })),
    )
    .await;
    let json = body_json(resp).await;
    assert_eq!(json["error"], "session not found");
    assert_eq!(json["id"], "ghost");
}

#[tokio::test]
async fn decide_malformed_json_is_400() {
    let state = test_state();
    state.sessions.upsert(review("bad-json"));
    let req = Request::builder()
        .method(Method::POST)
        .uri("/review/bad-json/decide")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{ not valid"))
        .unwrap();
    let resp = send(build_router(state), req).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn decide_missing_required_decision_field_is_422() {
    // `decision` is required; omitting it makes the Json extractor fail.
    let state = test_state();
    state.sessions.upsert(review("miss"));
    let resp = send(
        build_router(state),
        post_json("/review/miss/decide", &json!({ "feedback": "x" })),
    )
    .await;
    // axum surfaces a semantically-invalid-but-syntactically-valid body as 422.
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn decide_wrong_type_for_decision_is_422() {
    let state = test_state();
    state.sessions.upsert(review("wt"));
    let resp = send(
        build_router(state),
        post_json("/review/wt/decide", &json!({ "decision": 123 })),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn decide_empty_body_is_rejected() {
    let state = test_state();
    state.sessions.upsert(review("eb"));
    let req = Request::builder()
        .method(Method::POST)
        .uri("/review/eb/decide")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::empty())
        .unwrap();
    let resp = send(build_router(state), req).await;
    assert!(resp.status().is_client_error());
}

#[tokio::test]
async fn decide_missing_content_type_is_rejected() {
    // axum's Json extractor requires the application/json content-type.
    let state = test_state();
    state.sessions.upsert(review("noct"));
    let req = Request::builder()
        .method(Method::POST)
        .uri("/review/noct/decide")
        .body(Body::from(r#"{"decision":"approved"}"#))
        .unwrap();
    let resp = send(build_router(state), req).await;
    assert_eq!(resp.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
}

#[tokio::test]
async fn decide_get_method_is_405() {
    let state = test_state();
    state.sessions.upsert(review("gm"));
    let resp = send(build_router(state), get("/review/gm/decide")).await;
    assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn decide_on_non_review_session_is_409() {
    // A picker session is registered under the id, but decide expects a review.
    let state = test_state();
    state.sessions.upsert(picker("conf"));
    let resp = send(
        build_router(state),
        post_json("/review/conf/decide", &json!({ "decision": "approved" })),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let json = body_json(resp).await;
    assert_eq!(json["error"], "session is not a review session");
    assert_eq!(json["session_id"], "conf");
}

#[tokio::test]
async fn decide_on_question_session_is_409() {
    let state = test_state();
    state.sessions.upsert(question("qconf"));
    let resp = send(
        build_router(state),
        post_json("/review/qconf/decide", &json!({ "decision": "approved" })),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn decide_on_view_session_is_409() {
    let state = test_state();
    state.sessions.upsert(view("vconf"));
    let resp = send(
        build_router(state),
        post_json("/review/vconf/decide", &json!({ "decision": "approved" })),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn decide_unknown_takes_precedence_over_bad_body() {
    // 404 (session lookup) fires before the Json extractor runs? No — extractor
    // runs first in axum. A malformed body on an unknown session is still 400.
    let req = Request::builder()
        .method(Method::POST)
        .uri("/review/ghost/decide")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{bad"))
        .unwrap();
    let resp = send(build_router(test_state()), req).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn decide_is_repeatable_last_write_wins() {
    let state = test_state();
    state.sessions.upsert(review("repeat"));
    let app = build_router(state.clone());
    // First approve, then request changes — the latter wins.
    let _ = send(
        app.clone(),
        post_json("/review/repeat/decide", &json!({ "decision": "approved" })),
    )
    .await;
    let _ = send(
        app,
        post_json("/review/repeat/decide", &json!({ "decision": "nope" })),
    )
    .await;
    let SessionPayload::Review(r) = state.sessions.get("repeat").unwrap() else {
        panic!("expected review");
    };
    assert_eq!(r.status, SessionStatus::ChangesRequested);
}

#[tokio::test]
async fn decide_does_not_clobber_other_review_fields() {
    let state = test_state();
    let mut payload = review("keep");
    if let SessionPayload::Review(ref mut r) = payload {
        r.run_slug = Some("the-run".into());
        r.station = Some("frame".into());
    }
    state.sessions.upsert(payload);
    let _ = send(
        build_router(state.clone()),
        post_json("/review/keep/decide", &json!({ "decision": "approved" })),
    )
    .await;
    let SessionPayload::Review(r) = state.sessions.get("keep").unwrap() else {
        panic!("expected review");
    };
    assert_eq!(r.run_slug.as_deref(), Some("the-run"));
    assert_eq!(r.station.as_deref(), Some("frame"));
}

#[tokio::test]
async fn decide_long_feedback_is_accepted_under_body_limit() {
    let state = test_state();
    state.sessions.upsert(review("long"));
    let big = "x".repeat(50_000);
    let resp = send(
        build_router(state),
        post_json(
            "/review/long/decide",
            &json!({ "decision": "approved", "feedback": big }),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn decide_unicode_feedback_roundtrips() {
    let state = test_state();
    state.sessions.upsert(review("uni"));
    let resp = send(
        build_router(state),
        post_json(
            "/review/uni/decide",
            &json!({ "decision": "nope", "feedback": "café — 日本語 — 🚀" }),
        ),
    )
    .await;
    let json = body_json(resp).await;
    assert_eq!(json["feedback"], "café — 日本語 — 🚀");
}

#[tokio::test]
async fn decide_extra_unknown_fields_are_ignored() {
    // ReviewDecisionRequest does not deny unknown fields → extra keys ignored.
    let state = test_state();
    state.sessions.upsert(review("extra"));
    let resp = send(
        build_router(state),
        post_json(
            "/review/extra/decide",
            &json!({ "decision": "approved", "bogus": 1, "more": [1, 2, 3] }),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
}

// ════════════════════════════════════════════════════════════════════════════
// POST /api/advance/:id
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn advance_status_200() {
    let state = test_state();
    state.sessions.upsert(review("adv-1"));
    let resp = send(build_router(state), post_empty("/api/advance/adv-1")).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn advance_response_shape() {
    let state = test_state();
    state.sessions.upsert(review("adv-2"));
    let resp = send(build_router(state), post_empty("/api/advance/adv-2")).await;
    let json = body_json(resp).await;
    assert_eq!(json["ok"], true);
    assert_eq!(json["advanced"], true);
}

#[tokio::test]
async fn advance_content_type_is_json() {
    let state = test_state();
    state.sessions.upsert(review("adv-ct"));
    let resp = send(build_router(state), post_empty("/api/advance/adv-ct")).await;
    assert!(content_type(&resp).starts_with("application/json"));
}

#[tokio::test]
async fn advance_marks_review_decided() {
    let state = test_state();
    state.sessions.upsert(review("adv-3"));
    let _ = send(build_router(state.clone()), post_empty("/api/advance/adv-3")).await;
    let SessionPayload::Review(r) = state.sessions.get("adv-3").unwrap() else {
        panic!("expected review");
    };
    assert_eq!(r.status, SessionStatus::Decided);
}

#[tokio::test]
async fn advance_unknown_session_is_404() {
    let resp = send(build_router(test_state()), post_empty("/api/advance/ghost")).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn advance_unknown_session_error_envelope() {
    let resp = send(build_router(test_state()), post_empty("/api/advance/ghost")).await;
    let json = body_json(resp).await;
    assert_eq!(json["error"], "session not found");
    assert_eq!(json["id"], "ghost");
}

#[tokio::test]
async fn advance_non_review_session_still_200() {
    // advance is a no-op-ish wake; a non-review session existing returns 200
    // without mutating it (only the Review branch flips status).
    let state = test_state();
    state.sessions.upsert(picker("adv-pick"));
    let resp = send(build_router(state.clone()), post_empty("/api/advance/adv-pick")).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["advanced"], true);
    // Picker untouched.
    let SessionPayload::Picker(p) = state.sessions.get("adv-pick").unwrap() else {
        panic!("expected picker");
    };
    assert_eq!(p.status, SessionStatus::Pending);
}

#[tokio::test]
async fn advance_question_session_unmutated() {
    let state = test_state();
    state.sessions.upsert(question("adv-q"));
    let _ = send(build_router(state.clone()), post_empty("/api/advance/adv-q")).await;
    let SessionPayload::Question(q) = state.sessions.get("adv-q").unwrap() else {
        panic!("expected question");
    };
    assert_eq!(q.status, SessionStatus::Pending);
}

#[tokio::test]
async fn advance_is_idempotent() {
    let state = test_state();
    state.sessions.upsert(review("adv-idem"));
    let app = build_router(state.clone());
    for _ in 0..4 {
        let resp = send(app.clone(), post_empty("/api/advance/adv-idem")).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }
    let SessionPayload::Review(r) = state.sessions.get("adv-idem").unwrap() else {
        panic!("expected review");
    };
    assert_eq!(r.status, SessionStatus::Decided);
}

#[tokio::test]
async fn advance_ignores_request_body() {
    let state = test_state();
    state.sessions.upsert(review("adv-body"));
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/advance/adv-body")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"anything":true}"#))
        .unwrap();
    let resp = send(build_router(state), req).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn advance_get_method_is_405() {
    let state = test_state();
    state.sessions.upsert(review("adv-gm"));
    let resp = send(build_router(state), get("/api/advance/adv-gm")).await;
    assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn advance_after_decide_overwrites_to_decided() {
    let state = test_state();
    state.sessions.upsert(review("adv-after"));
    let app = build_router(state.clone());
    let _ = send(
        app.clone(),
        post_json("/review/adv-after/decide", &json!({ "decision": "approved" })),
    )
    .await;
    let _ = send(app, post_empty("/api/advance/adv-after")).await;
    let SessionPayload::Review(r) = state.sessions.get("adv-after").unwrap() else {
        panic!("expected review");
    };
    // advance unconditionally moves a review to Decided.
    assert_eq!(r.status, SessionStatus::Decided);
}

// ════════════════════════════════════════════════════════════════════════════
// Routing / method edges
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn unknown_route_is_404() {
    let resp = send(build_router(test_state()), get("/nope/nowhere")).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn root_path_is_404() {
    let resp = send(build_router(test_state()), get("/")).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn api_prefix_without_resource_is_404() {
    let resp = send(build_router(test_state()), get("/api")).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn session_without_id_segment_is_404() {
    // `/api/session` (no id) does not match the `:id` route.
    let resp = send(build_router(test_state()), get("/api/session")).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn review_decide_wrong_suffix_is_404() {
    let state = test_state();
    state.sessions.upsert(review("rs"));
    let resp = send(
        build_router(state),
        post_json("/review/rs/decline", &json!({ "decision": "approved" })),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn case_sensitive_paths_health_uppercase_404() {
    let resp = send(build_router(test_state()), get("/HEALTH")).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ════════════════════════════════════════════════════════════════════════════
// CORS — local (loopback) vs remote
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn remote_preflight_allows_any_origin() {
    let req = Request::builder()
        .method(Method::OPTIONS)
        .uri("/api/session/x")
        .header(header::ORIGIN, "https://example.com")
        .header("access-control-request-method", "GET")
        .body(Body::empty())
        .unwrap();
    let resp = send(build_router(remote_state()), req).await;
    let allow = resp
        .headers()
        .get("access-control-allow-origin")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert_eq!(allow, "*");
}

#[tokio::test]
async fn remote_get_response_carries_acao_star() {
    let state = remote_state();
    state.sessions.upsert(review("cors-get"));
    let req = Request::builder()
        .uri("/api/session/cors-get")
        .header(header::ORIGIN, "https://app.example.com")
        .body(Body::empty())
        .unwrap();
    let resp = send(build_router(state), req).await;
    let allow = resp
        .headers()
        .get("access-control-allow-origin")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert_eq!(allow, "*");
}

#[tokio::test]
async fn remote_health_carries_acao_star() {
    let req = Request::builder()
        .uri("/health")
        .header(header::ORIGIN, "https://x.test")
        .body(Body::empty())
        .unwrap();
    let resp = send(build_router(remote_state()), req).await;
    let allow = resp
        .headers()
        .get("access-control-allow-origin")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert_eq!(allow, "*");
}

#[tokio::test]
async fn local_get_does_not_echo_foreign_origin_as_star() {
    // Local mode pins ACAO to http://127.0.0.1; a foreign origin is not allowed.
    let state = test_state();
    state.sessions.upsert(review("cors-local"));
    let req = Request::builder()
        .uri("/api/session/cors-local")
        .header(header::ORIGIN, "https://evil.example.com")
        .body(Body::empty())
        .unwrap();
    let resp = send(build_router(state), req).await;
    let allow = resp
        .headers()
        .get("access-control-allow-origin")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert_ne!(allow, "*", "local mode must not advertise a wildcard origin");
}

#[tokio::test]
async fn local_loopback_origin_is_allowed() {
    let state = test_state();
    state.sessions.upsert(review("loop"));
    let req = Request::builder()
        .uri("/api/session/loop")
        .header(header::ORIGIN, "http://127.0.0.1")
        .body(Body::empty())
        .unwrap();
    let resp = send(build_router(state), req).await;
    let allow = resp
        .headers()
        .get("access-control-allow-origin")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert_eq!(allow, "http://127.0.0.1");
}

#[tokio::test]
async fn remote_preflight_advertises_post_method() {
    let req = Request::builder()
        .method(Method::OPTIONS)
        .uri("/review/x/decide")
        .header(header::ORIGIN, "https://example.com")
        .header("access-control-request-method", "POST")
        .body(Body::empty())
        .unwrap();
    let resp = send(build_router(remote_state()), req).await;
    let methods = resp
        .headers()
        .get("access-control-allow-methods")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(methods.contains("POST"), "methods were {methods:?}");
}

// ════════════════════════════════════════════════════════════════════════════
// Full review lifecycle through the routes
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn lifecycle_get_decide_get_reflects_decision() {
    let state = test_state();
    state.sessions.upsert(review("life"));
    let app = build_router(state);

    // Initially pending.
    let before = body_json(send(app.clone(), get("/api/session/life")).await).await;
    assert_eq!(before["status"], "pending");

    // Approve.
    let _ = send(
        app.clone(),
        post_json("/review/life/decide", &json!({ "decision": "approved", "feedback": "ok" })),
    )
    .await;

    // The subsequent GET reflects the resolved state.
    let after = body_json(send(app, get("/api/session/life")).await).await;
    assert_eq!(after["status"], "approved");
    assert_eq!(after["decision"], "approved");
    assert_eq!(after["feedback"], "ok");
}

#[tokio::test]
async fn lifecycle_changes_then_advance() {
    let state = test_state();
    state.sessions.upsert(review("life2"));
    let app = build_router(state);

    let _ = send(
        app.clone(),
        post_json("/review/life2/decide", &json!({ "decision": "nope" })),
    )
    .await;
    let mid = body_json(send(app.clone(), get("/api/session/life2")).await).await;
    assert_eq!(mid["status"], "changes_requested");

    let _ = send(app.clone(), post_empty("/api/advance/life2")).await;
    let end = body_json(send(app, get("/api/session/life2")).await).await;
    assert_eq!(end["status"], "decided");
}

#[tokio::test]
async fn lifecycle_distinct_sessions_decide_independently() {
    let state = test_state();
    state.sessions.upsert(review("ind-a"));
    state.sessions.upsert(review("ind-b"));
    let app = build_router(state);

    let _ = send(
        app.clone(),
        post_json("/review/ind-a/decide", &json!({ "decision": "approved" })),
    )
    .await;
    let _ = send(
        app.clone(),
        post_json("/review/ind-b/decide", &json!({ "decision": "nope" })),
    )
    .await;

    let a = body_json(send(app.clone(), get("/api/session/ind-a")).await).await;
    let b = body_json(send(app, get("/api/session/ind-b")).await).await;
    assert_eq!(a["status"], "approved");
    assert_eq!(b["status"], "changes_requested");
}

// ════════════════════════════════════════════════════════════════════════════
// Parametric sweep: every id charset survives 404 cleanly
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn parametric_404s_for_varied_absent_ids() {
    let app = build_router(test_state());
    let ids = [
        "alpha", "Beta", "123", "a-b-c", "x_y", "FB-01", "deeply.dotted", "UPPER", "mix3d",
        "z", "tail-", "lead",
    ];
    for id in ids {
        let resp = send(app.clone(), get(&format!("/api/session/{id}"))).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "id {id}");
    }
}

#[tokio::test]
async fn parametric_present_then_absent_after_no_remove() {
    // Registering many sessions, all resolvable; one never-registered id 404s.
    let state = test_state();
    for i in 0..10 {
        state.sessions.upsert(review(&format!("p-{i}")));
    }
    let app = build_router(state);
    for i in 0..10 {
        let resp = send(app.clone(), get(&format!("/api/session/p-{i}"))).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }
    let resp = send(app, get("/api/session/p-999")).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn unit_reset_route_sets_the_flag_then_404s_unknown() {
    use darkrun_core::domain::{Status, Unit, UnitFrontmatter};
    let state = test_state();
    let store = state.store.clone();
    // Seed a wedged unit on disk.
    let unit = Unit {
        slug: "u1".into(),
        frontmatter: UnitFrontmatter {
            status: Status::InProgress,
            station: Some("build".into()),
            ..Default::default()
        },
        title: "u1".into(),
        body: "# u1\nspec\n".into(),
    };
    store.write_unit("run", &unit).unwrap();
    assert!(!store.read_unit("run", "u1").unwrap().frontmatter.reset_requested);

    let app = build_router(state);
    // POST the reset request → 200 and the flag is set for the engine to consume.
    let resp = send(app.clone(), post_json("/api/unit/run/u1/reset", &json!({}))).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["ok"], json!(true));
    assert_eq!(body["reset_requested"], json!(true));
    assert!(
        store.read_unit("run", "u1").unwrap().frontmatter.reset_requested,
        "the flag is persisted on disk"
    );

    // Idempotent re-request still 200.
    let resp = send(app.clone(), post_json("/api/unit/run/u1/reset", &json!({}))).await;
    assert_eq!(resp.status(), StatusCode::OK);

    // Unknown unit → 404.
    let resp = send(app, post_json("/api/unit/run/nope/reset", &json!({}))).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
