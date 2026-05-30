//! End-to-end HTTP tests — area "http_session".
//!
//! Drives the real `darkrun_http` axum app (built via `build_router`) end to
//! end against a genuine `darkrun_core::StateStore` that was *seeded by the
//! manager* — every fixture starts a real run through `darkrun_mcp::run_start`,
//! then registers the interactive `ReviewSessionPayload`s into the same
//! `SessionRegistry` the production manager populates. No socket is bound: the
//! router is exercised through `tower::ServiceExt::oneshot`, the exact in-process
//! path the desktop review app talks to.
//!
//! This is a cross-crate flow test: `darkrun-mcp` (run lifecycle) +
//! `darkrun-core` (on-disk `.darkrun/` state + feedback sidecars) +
//! `darkrun-http` (the REST surface) + `darkrun-api` (the wire types every
//! payload is asserted to deserialize into) all participate as one system.
//!
//! Coverage:
//!   - `GET    /health`
//!   - `GET    /api/session/:id`        — across a run's station progression
//!   - `HEAD   /api/session/:id/heartbeat`
//!   - `POST   /review/:id/decide`      — approve + request-changes + canonicalize
//!   - `POST   /api/advance/:id`
//!   - feedback create -> list -> update -> reply -> delete (full CRUD)
//!   - error/status paths: 404 unknown session, 400 bad body, 409 conflicts,
//!     413 oversize, and the remote-mode per-IP rate limit (429).

#![allow(clippy::bool_assert_comparison)]

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use darkrun_api::{
    ApproveAction, ApproveActionKind, FeedbackCreateResponse, FeedbackDeleteResponse, FeedbackItem,
    FeedbackListResponse, FeedbackOrigin, FeedbackReplyCreateResponse, FeedbackStatus,
    FeedbackUpdateResponse, GateType, ReviewDecision, ReviewDecisionResponse, ReviewSessionPayload,
    SessionPayload, SessionStatus, AuthorType,
};
use darkrun_core::StateStore;
use darkrun_http::{build_router, AppState, Limits, SessionRegistry};
use darkrun_mcp::position::run_start;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

// ════════════════════════════════════════════════════════════════════════════
// Harness — a manager-seeded run behind the real HTTP app
// ════════════════════════════════════════════════════════════════════════════

/// The software factory's six stations, in walk order.
const STATIONS: [&str; 6] = ["frame", "specify", "shape", "build", "prove", "harden"];

/// A live HTTP fixture: a temp `.darkrun/` rooted store seeded by `run_start`,
/// the `AppState` over it (sharing the session registry), and the run slug.
struct Http {
    _dir: tempfile::TempDir,
    state: AppState,
    sessions: SessionRegistry,
    slug: String,
}

impl Http {
    /// Seed a fresh `software` run and stand the app up over its store.
    fn start(slug: &str) -> Self {
        Self::start_mode(slug, Limits::default())
    }

    /// Like [`Http::start`] but with explicit limits (used for the remote-mode
    /// rate-limit + CORS paths).
    fn start_mode(slug: &str, limits: Limits) -> Self {
        let dir = tempfile::tempdir().expect("tmpdir");
        let store = StateStore::new(dir.path());
        run_start(&store, slug, "software", Some("Ship it".into()), "continuous")
            .expect("run_start");
        let state = AppState::new(store, limits);
        let sessions = state.sessions.clone();
        Http {
            _dir: dir,
            state,
            sessions,
            slug: slug.to_string(),
        }
    }

    /// Build a fresh router over the shared state. A new router per request is
    /// fine — they all project the same `AppState`/registry.
    fn router(&self) -> axum::Router {
        build_router(self.state.clone())
    }

    /// Register a checkpoint review session for `station`, modeling what the
    /// manager does when a Checkpoint gate opens. Returns the session id.
    fn open_review(&self, station: &str) -> String {
        let id = format!("rev-{}-{station}", self.slug);
        self.sessions
            .upsert(SessionPayload::Review(self.review_payload(&id, station)));
        id
    }

    /// Build a representative review payload for a station's checkpoint.
    fn review_payload(&self, id: &str, station: &str) -> ReviewSessionPayload {
        ReviewSessionPayload {
            session_id: id.to_string(),
            status: SessionStatus::Pending,
            run_slug: Some(self.slug.clone()),
            gate_type: Some(GateType::Ask),
            station: Some(station.to_string()),
            approve_action: Some(ApproveAction {
                label: format!("Complete {station} Station"),
                kind: ApproveActionKind::CompleteStation,
            }),
            await_active: Some(true),
            ..Default::default()
        }
    }

    /// File a feedback sidecar directly on disk (Track-B style), the way the
    /// engine writes adversarial-review findings.
    fn file_feedback(&self, id: &str, station: &str, status: &str, origin: &str, body: &str) {
        let doc = format!(
            "---\nid: {id}\nstation: {station}\nstatus: {status}\norigin: {origin}\ntitle: {id} title\nauthor: agent\ncreated_at: 2026-05-30T00:00:00Z\nvisit: 0\n---\n{body}\n"
        );
        self.state
            .store
            .write_feedback_raw(&self.slug, id, &doc)
            .expect("write feedback");
    }

    async fn send(&self, req: Request<Body>) -> axum::response::Response {
        self.router().oneshot(req).await.unwrap()
    }
}

// ── Request builders ─────────────────────────────────────────────────────────

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

fn head(uri: &str) -> Request<Body> {
    Request::builder()
        .method(Method::HEAD)
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

fn post_json(uri: &str, v: &Value) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(v).unwrap()))
        .unwrap()
}

fn post_raw(uri: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn post_empty(uri: &str) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

fn put_json(uri: &str, v: &Value) -> Request<Body> {
    Request::builder()
        .method(Method::PUT)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(v).unwrap()))
        .unwrap()
}

fn delete(uri: &str) -> Request<Body> {
    Request::builder()
        .method(Method::DELETE)
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

// ── Response readers ─────────────────────────────────────────────────────────

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

/// Deserialize a session-fetch response into the strongly-typed wire union.
async fn read_session(resp: axum::response::Response) -> SessionPayload {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).expect("SessionPayload deserializes")
}

/// Convenience: create + register a feedback item over HTTP, return the id.
async fn create_feedback(h: &Http, station: &str, title: &str, body: &str) -> String {
    let resp = h
        .send(post_json(
            &format!("/api/feedback/{}/{station}", h.slug),
            &json!({ "title": title, "body": body }),
        ))
        .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let parsed: FeedbackCreateResponse = serde_json::from_slice(&body_bytes(resp).await).unwrap();
    parsed.feedback_id
}

// ════════════════════════════════════════════════════════════════════════════
// GET /health
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn health_returns_200() {
    let h = Http::start("r");
    let resp = h.send(get("/health")).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn health_body_status_ok() {
    let h = Http::start("r");
    let json = body_json(h.send(get("/health")).await).await;
    assert_eq!(json["status"], "ok");
}

#[tokio::test]
async fn health_is_json() {
    let h = Http::start("r");
    let resp = h.send(get("/health")).await;
    assert!(content_type(&resp).starts_with("application/json"));
}

#[tokio::test]
async fn health_has_single_field() {
    let h = Http::start("r");
    let json = body_json(h.send(get("/health")).await).await;
    assert_eq!(json.as_object().unwrap().len(), 1);
}

#[tokio::test]
async fn health_independent_of_seeded_run() {
    // Health never touches the store; a run with no sessions still answers.
    let h = Http::start("empty-run");
    let resp = h.send(get("/health")).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn health_idempotent_across_calls() {
    let h = Http::start("r");
    for _ in 0..5 {
        let resp = h.send(get("/health")).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }
}

// ════════════════════════════════════════════════════════════════════════════
// GET /api/session/:id — presence / absence
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn session_unknown_is_404() {
    let h = Http::start("r");
    let resp = h.send(get("/api/session/nope")).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn session_unknown_404_envelope() {
    let h = Http::start("r");
    let json = body_json(h.send(get("/api/session/ghost")).await).await;
    assert_eq!(json["error"], "session not found");
    assert_eq!(json["id"], "ghost");
}

#[tokio::test]
async fn session_present_is_200() {
    let h = Http::start("r");
    let id = h.open_review("frame");
    let resp = h.send(get(&format!("/api/session/{id}"))).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn session_present_is_json() {
    let h = Http::start("r");
    let id = h.open_review("frame");
    let resp = h.send(get(&format!("/api/session/{id}"))).await;
    assert!(content_type(&resp).starts_with("application/json"));
}

#[tokio::test]
async fn session_deserializes_into_api_type() {
    let h = Http::start("r");
    let id = h.open_review("frame");
    let resp = h.send(get(&format!("/api/session/{id}"))).await;
    let payload = read_session(resp).await;
    match payload {
        SessionPayload::Review(r) => {
            assert_eq!(r.session_id, id);
            assert_eq!(r.station.as_deref(), Some("frame"));
        }
        other => panic!("expected review, got {}", other.session_type()),
    }
}

#[tokio::test]
async fn session_carries_run_slug() {
    let h = Http::start("my-run");
    let id = h.open_review("frame");
    let payload = read_session(h.send(get(&format!("/api/session/{id}"))).await).await;
    let SessionPayload::Review(r) = payload else {
        panic!("review");
    };
    assert_eq!(r.run_slug.as_deref(), Some("my-run"));
}

#[tokio::test]
async fn session_discriminator_is_review() {
    let h = Http::start("r");
    let id = h.open_review("frame");
    let json = body_json(h.send(get(&format!("/api/session/{id}"))).await).await;
    assert_eq!(json["session_type"], "review");
}

#[tokio::test]
async fn session_gate_type_present() {
    let h = Http::start("r");
    let id = h.open_review("frame");
    let json = body_json(h.send(get(&format!("/api/session/{id}"))).await).await;
    assert_eq!(json["gate_type"], "ask");
}

#[tokio::test]
async fn session_status_initially_pending() {
    let h = Http::start("r");
    let id = h.open_review("frame");
    let payload = read_session(h.send(get(&format!("/api/session/{id}"))).await).await;
    let SessionPayload::Review(r) = payload else {
        panic!("review");
    };
    assert_eq!(r.status, SessionStatus::Pending);
}

#[tokio::test]
async fn session_approve_action_round_trips() {
    let h = Http::start("r");
    let id = h.open_review("build");
    let payload = read_session(h.send(get(&format!("/api/session/{id}"))).await).await;
    let SessionPayload::Review(r) = payload else {
        panic!("review");
    };
    let action = r.approve_action.expect("approve action");
    assert_eq!(action.kind, ApproveActionKind::CompleteStation);
    assert!(action.label.contains("build"));
}

#[tokio::test]
async fn session_await_active_true() {
    let h = Http::start("r");
    let id = h.open_review("frame");
    let payload = read_session(h.send(get(&format!("/api/session/{id}"))).await).await;
    let SessionPayload::Review(r) = payload else {
        panic!("review");
    };
    assert_eq!(r.await_active, Some(true));
}

// One presence + one deserialize test per station — the run's station roster.
macro_rules! station_session_tests {
    ($($name:ident => $station:literal),* $(,)?) => {
        $(
            #[tokio::test]
            async fn $name() {
                let h = Http::start("r");
                let id = h.open_review($station);
                let resp = h.send(get(&format!("/api/session/{id}"))).await;
                assert_eq!(resp.status(), StatusCode::OK);
                let payload = read_session(resp).await;
                let SessionPayload::Review(r) = payload else { panic!("review") };
                assert_eq!(r.station.as_deref(), Some($station));
            }
        )*
    };
}

station_session_tests! {
    session_station_frame => "frame",
    session_station_specify => "specify",
    session_station_shape => "shape",
    session_station_build => "build",
    session_station_prove => "prove",
    session_station_harden => "harden",
}

#[tokio::test]
async fn session_progression_across_all_stations() {
    // Model a run walking through every station: open each checkpoint review in
    // turn, fetch it, and assert the wire payload reflects the right station.
    let h = Http::start("walk");
    for station in STATIONS {
        let id = h.open_review(station);
        let payload = read_session(h.send(get(&format!("/api/session/{id}"))).await).await;
        let SessionPayload::Review(r) = payload else {
            panic!("review");
        };
        assert_eq!(r.station.as_deref(), Some(station));
        assert_eq!(r.status, SessionStatus::Pending);
    }
}

#[tokio::test]
async fn session_progression_keeps_prior_sessions_addressable() {
    // After advancing, an earlier station's session is still fetchable (the
    // registry never evicts on its own).
    let h = Http::start("walk");
    let first = h.open_review("frame");
    let _second = h.open_review("specify");
    let resp = h.send(get(&format!("/api/session/{first}"))).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn session_overwrite_reflects_latest_payload() {
    // Re-registering the same id with a new status (manager re-upsert) is the
    // version the GET returns.
    let h = Http::start("r");
    let id = h.open_review("frame");
    let mut p = h.review_payload(&id, "frame");
    p.status = SessionStatus::Approved;
    p.decision = Some("approved".into());
    h.sessions.upsert(SessionPayload::Review(p));
    let payload = read_session(h.send(get(&format!("/api/session/{id}"))).await).await;
    let SessionPayload::Review(r) = payload else {
        panic!("review");
    };
    assert_eq!(r.status, SessionStatus::Approved);
    assert_eq!(r.decision.as_deref(), Some("approved"));
}

// ════════════════════════════════════════════════════════════════════════════
// HEAD /api/session/:id/heartbeat
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn heartbeat_known_session_200() {
    let h = Http::start("r");
    let id = h.open_review("frame");
    let resp = h.send(head(&format!("/api/session/{id}/heartbeat"))).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn heartbeat_unknown_session_404() {
    let h = Http::start("r");
    let resp = h.send(head("/api/session/ghost/heartbeat")).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn heartbeat_has_no_body() {
    let h = Http::start("r");
    let id = h.open_review("frame");
    let resp = h.send(head(&format!("/api/session/{id}/heartbeat"))).await;
    let bytes = body_bytes(resp).await;
    assert!(bytes.is_empty());
}

#[tokio::test]
async fn heartbeat_tracks_progression() {
    let h = Http::start("r");
    for station in STATIONS {
        let id = h.open_review(station);
        let resp = h.send(head(&format!("/api/session/{id}/heartbeat"))).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }
}

// ════════════════════════════════════════════════════════════════════════════
// POST /review/:id/decide — approve
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn decide_approve_is_200() {
    let h = Http::start("r");
    let id = h.open_review("frame");
    let resp = h
        .send(post_json(
            &format!("/review/{id}/decide"),
            &json!({ "decision": "approved" }),
        ))
        .await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn decide_approve_response_type() {
    let h = Http::start("r");
    let id = h.open_review("frame");
    let resp = h
        .send(post_json(
            &format!("/review/{id}/decide"),
            &json!({ "decision": "approved" }),
        ))
        .await;
    let parsed: ReviewDecisionResponse = serde_json::from_slice(&body_bytes(resp).await).unwrap();
    assert!(parsed.ok);
    assert_eq!(parsed.decision, ReviewDecision::Approved);
    assert_eq!(parsed.feedback, "");
}

#[tokio::test]
async fn decide_approve_echoes_feedback() {
    let h = Http::start("r");
    let id = h.open_review("frame");
    let resp = h
        .send(post_json(
            &format!("/review/{id}/decide"),
            &json!({ "decision": "approved", "feedback": "looks great" }),
        ))
        .await;
    let parsed: ReviewDecisionResponse = serde_json::from_slice(&body_bytes(resp).await).unwrap();
    assert_eq!(parsed.feedback, "looks great");
}

#[tokio::test]
async fn decide_approve_updates_session_status() {
    let h = Http::start("r");
    let id = h.open_review("frame");
    h.send(post_json(
        &format!("/review/{id}/decide"),
        &json!({ "decision": "approved" }),
    ))
    .await;
    // The session payload is mutated in place; re-fetch reflects Approved.
    let payload = read_session(h.send(get(&format!("/api/session/{id}"))).await).await;
    let SessionPayload::Review(r) = payload else {
        panic!("review");
    };
    assert_eq!(r.status, SessionStatus::Approved);
    assert_eq!(r.decision.as_deref(), Some("approved"));
}

#[tokio::test]
async fn decide_approve_case_insensitive() {
    for raw in ["APPROVED", "Approved", "  approved  ", "aPpRoVeD"] {
        let h = Http::start("r");
        let id = h.open_review("frame");
        let resp = h
            .send(post_json(
                &format!("/review/{id}/decide"),
                &json!({ "decision": raw }),
            ))
            .await;
        let parsed: ReviewDecisionResponse =
            serde_json::from_slice(&body_bytes(resp).await).unwrap();
        assert_eq!(
            parsed.decision,
            ReviewDecision::Approved,
            "raw {raw:?} should canonicalize to approved"
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// POST /review/:id/decide — request changes
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn decide_request_changes_is_200() {
    let h = Http::start("r");
    let id = h.open_review("frame");
    let resp = h
        .send(post_json(
            &format!("/review/{id}/decide"),
            &json!({ "decision": "changes_requested" }),
        ))
        .await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn decide_request_changes_canonicalizes() {
    let h = Http::start("r");
    let id = h.open_review("frame");
    let resp = h
        .send(post_json(
            &format!("/review/{id}/decide"),
            &json!({ "decision": "changes_requested", "feedback": "tighten the spec" }),
        ))
        .await;
    let parsed: ReviewDecisionResponse = serde_json::from_slice(&body_bytes(resp).await).unwrap();
    assert_eq!(parsed.decision, ReviewDecision::ChangesRequested);
    assert_eq!(parsed.feedback, "tighten the spec");
}

#[tokio::test]
async fn decide_request_changes_updates_status() {
    let h = Http::start("r");
    let id = h.open_review("frame");
    h.send(post_json(
        &format!("/review/{id}/decide"),
        &json!({ "decision": "changes_requested", "feedback": "redo" }),
    ))
    .await;
    let payload = read_session(h.send(get(&format!("/api/session/{id}"))).await).await;
    let SessionPayload::Review(r) = payload else {
        panic!("review");
    };
    assert_eq!(r.status, SessionStatus::ChangesRequested);
    assert_eq!(r.decision.as_deref(), Some("changes_requested"));
    assert_eq!(r.feedback.as_deref(), Some("redo"));
}

#[tokio::test]
async fn decide_unrecognized_decision_is_changes_requested() {
    // Anything other than "approved" canonicalizes to changes-requested.
    for raw in ["reject", "nope", "deny", "", "approve", "approvedd"] {
        let h = Http::start("r");
        let id = h.open_review("frame");
        let resp = h
            .send(post_json(
                &format!("/review/{id}/decide"),
                &json!({ "decision": raw }),
            ))
            .await;
        let parsed: ReviewDecisionResponse =
            serde_json::from_slice(&body_bytes(resp).await).unwrap();
        assert_eq!(
            parsed.decision,
            ReviewDecision::ChangesRequested,
            "raw {raw:?} should canonicalize to changes_requested"
        );
    }
}

#[tokio::test]
async fn decide_unknown_session_404() {
    let h = Http::start("r");
    let resp = h
        .send(post_json(
            "/review/ghost/decide",
            &json!({ "decision": "approved" }),
        ))
        .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn decide_missing_decision_field_is_422() {
    // `decision` is required; axum's Json extractor rejects the missing field.
    let h = Http::start("r");
    let id = h.open_review("frame");
    let resp = h
        .send(post_json(&format!("/review/{id}/decide"), &json!({})))
        .await;
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn decide_malformed_json_is_400() {
    let h = Http::start("r");
    let id = h.open_review("frame");
    let resp = h
        .send(post_raw(&format!("/review/{id}/decide"), "{ not json"))
        .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn decide_approve_then_request_changes_overwrites() {
    let h = Http::start("r");
    let id = h.open_review("frame");
    h.send(post_json(
        &format!("/review/{id}/decide"),
        &json!({ "decision": "approved" }),
    ))
    .await;
    h.send(post_json(
        &format!("/review/{id}/decide"),
        &json!({ "decision": "changes_requested", "feedback": "wait" }),
    ))
    .await;
    let payload = read_session(h.send(get(&format!("/api/session/{id}"))).await).await;
    let SessionPayload::Review(r) = payload else {
        panic!("review");
    };
    assert_eq!(r.status, SessionStatus::ChangesRequested);
}

#[tokio::test]
async fn decide_each_station_independently() {
    let h = Http::start("r");
    for (i, station) in STATIONS.iter().enumerate() {
        let id = h.open_review(station);
        let approve = i % 2 == 0;
        let decision = if approve { "approved" } else { "changes_requested" };
        let resp = h
            .send(post_json(
                &format!("/review/{id}/decide"),
                &json!({ "decision": decision }),
            ))
            .await;
        let parsed: ReviewDecisionResponse =
            serde_json::from_slice(&body_bytes(resp).await).unwrap();
        let expected = if approve {
            ReviewDecision::Approved
        } else {
            ReviewDecision::ChangesRequested
        };
        assert_eq!(parsed.decision, expected);
    }
}

// ════════════════════════════════════════════════════════════════════════════
// POST /api/advance/:id
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn advance_known_session_200() {
    let h = Http::start("r");
    let id = h.open_review("frame");
    let resp = h.send(post_empty(&format!("/api/advance/{id}"))).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn advance_body_shape() {
    let h = Http::start("r");
    let id = h.open_review("frame");
    let json = body_json(h.send(post_empty(&format!("/api/advance/{id}"))).await).await;
    assert_eq!(json["ok"], true);
    assert_eq!(json["advanced"], true);
}

#[tokio::test]
async fn advance_marks_session_decided() {
    let h = Http::start("r");
    let id = h.open_review("frame");
    h.send(post_empty(&format!("/api/advance/{id}"))).await;
    let payload = read_session(h.send(get(&format!("/api/session/{id}"))).await).await;
    let SessionPayload::Review(r) = payload else {
        panic!("review");
    };
    assert_eq!(r.status, SessionStatus::Decided);
}

#[tokio::test]
async fn advance_unknown_session_404() {
    let h = Http::start("r");
    let resp = h.send(post_empty("/api/advance/ghost")).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn advance_is_idempotent() {
    let h = Http::start("r");
    let id = h.open_review("frame");
    for _ in 0..3 {
        let resp = h.send(post_empty(&format!("/api/advance/{id}"))).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }
    let payload = read_session(h.send(get(&format!("/api/session/{id}"))).await).await;
    let SessionPayload::Review(r) = payload else {
        panic!("review");
    };
    assert_eq!(r.status, SessionStatus::Decided);
}

#[tokio::test]
async fn advance_after_decide_progression() {
    // The canonical progression: decide a checkpoint, then advance past the gate.
    let h = Http::start("r");
    let id = h.open_review("frame");
    h.send(post_json(
        &format!("/review/{id}/decide"),
        &json!({ "decision": "approved" }),
    ))
    .await;
    let resp = h.send(post_empty(&format!("/api/advance/{id}"))).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let payload = read_session(h.send(get(&format!("/api/session/{id}"))).await).await;
    let SessionPayload::Review(r) = payload else {
        panic!("review");
    };
    // Advance overwrites the Approved status with Decided.
    assert_eq!(r.status, SessionStatus::Decided);
}

// ════════════════════════════════════════════════════════════════════════════
// Feedback: create
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn feedback_create_201() {
    let h = Http::start("r");
    let resp = h
        .send(post_json(
            &format!("/api/feedback/{}/frame", h.slug),
            &json!({ "title": "Tighten the spec", "body": "needs detail" }),
        ))
        .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn feedback_create_response_type() {
    let h = Http::start("r");
    let resp = h
        .send(post_json(
            &format!("/api/feedback/{}/frame", h.slug),
            &json!({ "title": "t", "body": "b" }),
        ))
        .await;
    let parsed: FeedbackCreateResponse = serde_json::from_slice(&body_bytes(resp).await).unwrap();
    assert_eq!(parsed.feedback_id, "FB-01");
    assert_eq!(parsed.status, FeedbackStatus::Pending);
    assert_eq!(parsed.file, "feedback/FB-01.md");
}

#[tokio::test]
async fn feedback_create_mints_sequential_ids() {
    let h = Http::start("r");
    let a = create_feedback(&h, "frame", "one", "b1").await;
    let b = create_feedback(&h, "frame", "two", "b2").await;
    let c = create_feedback(&h, "frame", "three", "b3").await;
    assert_eq!(a, "FB-01");
    assert_eq!(b, "FB-02");
    assert_eq!(c, "FB-03");
}

#[tokio::test]
async fn feedback_create_empty_title_400() {
    let h = Http::start("r");
    let resp = h
        .send(post_json(
            &format!("/api/feedback/{}/frame", h.slug),
            &json!({ "title": "   ", "body": "b" }),
        ))
        .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn feedback_create_empty_body_400() {
    let h = Http::start("r");
    let resp = h
        .send(post_json(
            &format!("/api/feedback/{}/frame", h.slug),
            &json!({ "title": "t", "body": "" }),
        ))
        .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn feedback_create_missing_fields_422() {
    let h = Http::start("r");
    let resp = h
        .send(post_json(
            &format!("/api/feedback/{}/frame", h.slug),
            &json!({ "title": "only title" }),
        ))
        .await;
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn feedback_create_trims_fields() {
    let h = Http::start("r");
    let id = create_feedback(&h, "frame", "  spaced  ", "  body  ").await;
    let list = h
        .send(get(&format!("/api/feedback/{}/frame", h.slug)))
        .await;
    let parsed: FeedbackListResponse = serde_json::from_slice(&body_bytes(list).await).unwrap();
    let item = parsed.items.iter().find(|i| i.feedback_id == id).unwrap();
    assert_eq!(item.title, "spaced");
    assert_eq!(item.body, "body");
}

#[tokio::test]
async fn feedback_create_stamps_user_author() {
    // The HTTP trust boundary discards any client author and stamps `user`.
    let h = Http::start("r");
    h.send(post_json(
        &format!("/api/feedback/{}/frame", h.slug),
        &json!({ "title": "t", "body": "b", "author": "attacker" }),
    ))
    .await;
    let list = h
        .send(get(&format!("/api/feedback/{}/frame", h.slug)))
        .await;
    let parsed: FeedbackListResponse = serde_json::from_slice(&body_bytes(list).await).unwrap();
    let item = &parsed.items[0];
    assert_eq!(item.author, "user");
    assert_eq!(item.author_type, AuthorType::Human);
    assert_eq!(item.origin, FeedbackOrigin::UserVisual);
}

// ════════════════════════════════════════════════════════════════════════════
// Feedback: list
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn feedback_list_empty_200() {
    let h = Http::start("r");
    let resp = h
        .send(get(&format!("/api/feedback/{}/frame", h.slug)))
        .await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn feedback_list_empty_response_type() {
    let h = Http::start("r");
    let resp = h
        .send(get(&format!("/api/feedback/{}/frame", h.slug)))
        .await;
    let parsed: FeedbackListResponse = serde_json::from_slice(&body_bytes(resp).await).unwrap();
    assert_eq!(parsed.count, 0);
    assert_eq!(parsed.run, h.slug);
    assert_eq!(parsed.station, "frame");
    assert!(parsed.items.is_empty());
}

#[tokio::test]
async fn feedback_list_returns_created_items() {
    let h = Http::start("r");
    create_feedback(&h, "frame", "a", "ba").await;
    create_feedback(&h, "frame", "b", "bb").await;
    let resp = h
        .send(get(&format!("/api/feedback/{}/frame", h.slug)))
        .await;
    let parsed: FeedbackListResponse = serde_json::from_slice(&body_bytes(resp).await).unwrap();
    assert_eq!(parsed.count, 2);
}

#[tokio::test]
async fn feedback_list_sorted_by_id() {
    let h = Http::start("r");
    for i in 0..5 {
        create_feedback(&h, "frame", &format!("t{i}"), &format!("b{i}")).await;
    }
    let resp = h
        .send(get(&format!("/api/feedback/{}/frame", h.slug)))
        .await;
    let parsed: FeedbackListResponse = serde_json::from_slice(&body_bytes(resp).await).unwrap();
    let ids: Vec<&str> = parsed.items.iter().map(|i| i.feedback_id.as_str()).collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted);
}

#[tokio::test]
async fn feedback_list_filters_by_station() {
    let h = Http::start("r");
    create_feedback(&h, "frame", "frame-fb", "b").await;
    create_feedback(&h, "build", "build-fb", "b").await;
    let frame = h
        .send(get(&format!("/api/feedback/{}/frame", h.slug)))
        .await;
    let parsed: FeedbackListResponse = serde_json::from_slice(&body_bytes(frame).await).unwrap();
    assert_eq!(parsed.count, 1);
    assert_eq!(parsed.items[0].title, "frame-fb");
}

#[tokio::test]
async fn feedback_list_each_item_is_api_type() {
    let h = Http::start("r");
    create_feedback(&h, "frame", "t", "b").await;
    let resp = h
        .send(get(&format!("/api/feedback/{}/frame", h.slug)))
        .await;
    let bytes = body_bytes(resp).await;
    // Confirm every item deserializes into the strict FeedbackItem type.
    let parsed: FeedbackListResponse = serde_json::from_slice(&bytes).unwrap();
    let _items: Vec<FeedbackItem> = parsed.items;
}

#[tokio::test]
async fn feedback_list_reads_engine_written_findings() {
    // A finding written directly by the engine (adversarial-review origin) is
    // surfaced over HTTP and parses into the wire type with an agent author.
    let h = Http::start("r");
    h.file_feedback("FB-01", "build", "pending", "adversarial-review", "found a bug");
    let resp = h
        .send(get(&format!("/api/feedback/{}/build", h.slug)))
        .await;
    let parsed: FeedbackListResponse = serde_json::from_slice(&body_bytes(resp).await).unwrap();
    assert_eq!(parsed.count, 1);
    let item = &parsed.items[0];
    assert_eq!(item.origin, FeedbackOrigin::AdversarialReview);
    assert_eq!(item.author_type, AuthorType::Agent);
    assert!(item.body.contains("found a bug"));
}

#[tokio::test]
async fn feedback_list_unstationed_matches_any() {
    // An item with no recorded station is legacy-tolerant: it matches every
    // station query.
    let h = Http::start("r");
    let doc = "---\nid: FB-01\nstatus: pending\norigin: drift\ntitle: stray\nauthor: agent\ncreated_at: 2026-05-30T00:00:00Z\nvisit: 0\n---\nbody\n";
    h.state
        .store
        .write_feedback_raw(&h.slug, "FB-01", doc)
        .unwrap();
    for station in STATIONS {
        let resp = h
            .send(get(&format!("/api/feedback/{}/{station}", h.slug)))
            .await;
        let parsed: FeedbackListResponse =
            serde_json::from_slice(&body_bytes(resp).await).unwrap();
        assert_eq!(parsed.count, 1, "station {station} should see unstationed item");
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Feedback: update
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn feedback_update_status_200() {
    let h = Http::start("r");
    let id = create_feedback(&h, "frame", "t", "b").await;
    let resp = h
        .send(put_json(
            &format!("/api/feedback/{}/frame/{id}", h.slug),
            &json!({ "status": "closed" }),
        ))
        .await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn feedback_update_response_type() {
    let h = Http::start("r");
    let id = create_feedback(&h, "frame", "t", "b").await;
    let resp = h
        .send(put_json(
            &format!("/api/feedback/{}/frame/{id}", h.slug),
            &json!({ "status": "closed" }),
        ))
        .await;
    let parsed: FeedbackUpdateResponse = serde_json::from_slice(&body_bytes(resp).await).unwrap();
    assert_eq!(parsed.feedback_id, id);
    assert!(parsed.updated_fields.contains(&"status".to_string()));
}

#[tokio::test]
async fn feedback_update_persists_status() {
    let h = Http::start("r");
    let id = create_feedback(&h, "frame", "t", "b").await;
    h.send(put_json(
        &format!("/api/feedback/{}/frame/{id}", h.slug),
        &json!({ "status": "answered" }),
    ))
    .await;
    let list = h
        .send(get(&format!("/api/feedback/{}/frame", h.slug)))
        .await;
    let parsed: FeedbackListResponse = serde_json::from_slice(&body_bytes(list).await).unwrap();
    let item = parsed.items.iter().find(|i| i.feedback_id == id).unwrap();
    assert_eq!(item.status, FeedbackStatus::Answered);
}

#[tokio::test]
async fn feedback_update_closed_by_field() {
    let h = Http::start("r");
    let id = create_feedback(&h, "frame", "t", "b").await;
    let resp = h
        .send(put_json(
            &format!("/api/feedback/{}/frame/{id}", h.slug),
            &json!({ "closed_by": "build-unit" }),
        ))
        .await;
    let parsed: FeedbackUpdateResponse = serde_json::from_slice(&body_bytes(resp).await).unwrap();
    assert!(parsed.updated_fields.contains(&"closed_by".to_string()));
    let list = h
        .send(get(&format!("/api/feedback/{}/frame", h.slug)))
        .await;
    let lp: FeedbackListResponse = serde_json::from_slice(&body_bytes(list).await).unwrap();
    let item = lp.items.iter().find(|i| i.feedback_id == id).unwrap();
    assert_eq!(item.closed_by.as_deref(), Some("build-unit"));
}

#[tokio::test]
async fn feedback_update_resolution_acknowledged() {
    let h = Http::start("r");
    let id = create_feedback(&h, "frame", "t", "b").await;
    let resp = h
        .send(put_json(
            &format!("/api/feedback/{}/frame/{id}", h.slug),
            &json!({ "resolution": "inline_fix" }),
        ))
        .await;
    let parsed: FeedbackUpdateResponse = serde_json::from_slice(&body_bytes(resp).await).unwrap();
    assert!(parsed.updated_fields.contains(&"resolution".to_string()));
}

#[tokio::test]
async fn feedback_update_empty_body_400() {
    let h = Http::start("r");
    let id = create_feedback(&h, "frame", "t", "b").await;
    let resp = h
        .send(put_json(
            &format!("/api/feedback/{}/frame/{id}", h.slug),
            &json!({}),
        ))
        .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn feedback_update_unknown_id_404() {
    let h = Http::start("r");
    let resp = h
        .send(put_json(
            &format!("/api/feedback/{}/frame/FB-99", h.slug),
            &json!({ "status": "closed" }),
        ))
        .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn feedback_update_multiple_fields() {
    let h = Http::start("r");
    let id = create_feedback(&h, "frame", "t", "b").await;
    let resp = h
        .send(put_json(
            &format!("/api/feedback/{}/frame/{id}", h.slug),
            &json!({ "status": "closed", "closed_by": "u", "resolution": "stage_revisit" }),
        ))
        .await;
    let parsed: FeedbackUpdateResponse = serde_json::from_slice(&body_bytes(resp).await).unwrap();
    assert_eq!(parsed.updated_fields.len(), 3);
}

// ════════════════════════════════════════════════════════════════════════════
// Feedback: replies
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn feedback_reply_201() {
    let h = Http::start("r");
    let id = create_feedback(&h, "frame", "t", "b").await;
    let resp = h
        .send(post_json(
            &format!("/api/feedback/{}/frame/{id}/replies", h.slug),
            &json!({ "body": "thanks, will fix" }),
        ))
        .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn feedback_reply_response_type() {
    let h = Http::start("r");
    let id = create_feedback(&h, "frame", "t", "b").await;
    let resp = h
        .send(post_json(
            &format!("/api/feedback/{}/frame/{id}/replies", h.slug),
            &json!({ "body": "first reply" }),
        ))
        .await;
    let parsed: FeedbackReplyCreateResponse =
        serde_json::from_slice(&body_bytes(resp).await).unwrap();
    assert_eq!(parsed.feedback_id, id);
    assert_eq!(parsed.reply_index, 0);
}

#[tokio::test]
async fn feedback_reply_index_increments() {
    let h = Http::start("r");
    let id = create_feedback(&h, "frame", "t", "b").await;
    for expected in 0..3u32 {
        let resp = h
            .send(post_json(
                &format!("/api/feedback/{}/frame/{id}/replies", h.slug),
                &json!({ "body": format!("reply {expected}") }),
            ))
            .await;
        let parsed: FeedbackReplyCreateResponse =
            serde_json::from_slice(&body_bytes(resp).await).unwrap();
        assert_eq!(parsed.reply_index as u32, expected);
    }
}

#[tokio::test]
async fn feedback_reply_empty_body_400() {
    let h = Http::start("r");
    let id = create_feedback(&h, "frame", "t", "b").await;
    let resp = h
        .send(post_json(
            &format!("/api/feedback/{}/frame/{id}/replies", h.slug),
            &json!({ "body": "   " }),
        ))
        .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn feedback_reply_unknown_id_404() {
    let h = Http::start("r");
    let resp = h
        .send(post_json(
            &format!("/api/feedback/{}/frame/FB-99/replies", h.slug),
            &json!({ "body": "hi" }),
        ))
        .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn feedback_reply_close_as_answered() {
    let h = Http::start("r");
    let id = create_feedback(&h, "frame", "t", "b").await;
    let resp = h
        .send(post_json(
            &format!("/api/feedback/{}/frame/{id}/replies", h.slug),
            &json!({ "body": "answered here", "close_as_answered": true }),
        ))
        .await;
    let parsed: FeedbackReplyCreateResponse =
        serde_json::from_slice(&body_bytes(resp).await).unwrap();
    assert_eq!(parsed.status, FeedbackStatus::Answered);
}

// ════════════════════════════════════════════════════════════════════════════
// Feedback: delete
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn feedback_delete_open_item_409() {
    // An open (pending) item blocks the gate — deletion is refused.
    let h = Http::start("r");
    let id = create_feedback(&h, "frame", "t", "b").await;
    let resp = h
        .send(delete(&format!("/api/feedback/{}/frame/{id}", h.slug)))
        .await;
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn feedback_delete_closed_item_200() {
    let h = Http::start("r");
    let id = create_feedback(&h, "frame", "t", "b").await;
    // Close it first, then delete is permitted.
    h.send(put_json(
        &format!("/api/feedback/{}/frame/{id}", h.slug),
        &json!({ "status": "closed" }),
    ))
    .await;
    let resp = h
        .send(delete(&format!("/api/feedback/{}/frame/{id}", h.slug)))
        .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let parsed: FeedbackDeleteResponse = serde_json::from_slice(&body_bytes(resp).await).unwrap();
    assert!(parsed.deleted);
    assert_eq!(parsed.feedback_id, id);
}

#[tokio::test]
async fn feedback_delete_removes_from_list() {
    let h = Http::start("r");
    let id = create_feedback(&h, "frame", "t", "b").await;
    h.send(put_json(
        &format!("/api/feedback/{}/frame/{id}", h.slug),
        &json!({ "status": "closed" }),
    ))
    .await;
    h.send(delete(&format!("/api/feedback/{}/frame/{id}", h.slug)))
        .await;
    let list = h
        .send(get(&format!("/api/feedback/{}/frame", h.slug)))
        .await;
    let parsed: FeedbackListResponse = serde_json::from_slice(&body_bytes(list).await).unwrap();
    assert_eq!(parsed.count, 0);
}

#[tokio::test]
async fn feedback_delete_unknown_id_404() {
    let h = Http::start("r");
    let resp = h
        .send(delete(&format!("/api/feedback/{}/frame/FB-99", h.slug)))
        .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn feedback_delete_fixing_item_409() {
    // `fixing` still blocks the gate.
    let h = Http::start("r");
    let id = create_feedback(&h, "frame", "t", "b").await;
    h.send(put_json(
        &format!("/api/feedback/{}/frame/{id}", h.slug),
        &json!({ "status": "fixing" }),
    ))
    .await;
    let resp = h
        .send(delete(&format!("/api/feedback/{}/frame/{id}", h.slug)))
        .await;
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

// ════════════════════════════════════════════════════════════════════════════
// Full feedback lifecycle — create -> reply -> update -> delete
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn feedback_full_lifecycle() {
    let h = Http::start("lifecycle");
    // Create.
    let id = create_feedback(&h, "build", "Race condition", "the worker double-fires").await;
    // List sees it pending.
    let list = h.send(get(&format!("/api/feedback/{}/build", h.slug))).await;
    let lp: FeedbackListResponse = serde_json::from_slice(&body_bytes(list).await).unwrap();
    assert_eq!(lp.count, 1);
    assert_eq!(lp.items[0].status, FeedbackStatus::Pending);
    // Reply.
    let reply = h
        .send(post_json(
            &format!("/api/feedback/{}/build/{id}/replies", h.slug),
            &json!({ "body": "patched, please verify" }),
        ))
        .await;
    assert_eq!(reply.status(), StatusCode::CREATED);
    // Update to closed.
    let upd = h
        .send(put_json(
            &format!("/api/feedback/{}/build/{id}", h.slug),
            &json!({ "status": "closed", "closed_by": "build-unit-1" }),
        ))
        .await;
    assert_eq!(upd.status(), StatusCode::OK);
    // Delete now permitted.
    let del = h
        .send(delete(&format!("/api/feedback/{}/build/{id}", h.slug)))
        .await;
    assert_eq!(del.status(), StatusCode::OK);
    // Gone.
    let list2 = h.send(get(&format!("/api/feedback/{}/build", h.slug))).await;
    let lp2: FeedbackListResponse = serde_json::from_slice(&body_bytes(list2).await).unwrap();
    assert_eq!(lp2.count, 0);
}

// ════════════════════════════════════════════════════════════════════════════
// Oversize body — 413 from the RequestBodyLimitLayer
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn oversize_body_rejected_413() {
    let h = Http::start("r");
    let id = h.open_review("frame");
    // 2 MiB exceeds the 1 MiB default body cap.
    let big = "x".repeat(2 * 1_048_576);
    let body = json!({ "decision": "approved", "feedback": big });
    let req = Request::builder()
        .method(Method::POST)
        .uri(format!("/review/{id}/decide"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = h.send(req).await;
    assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn under_limit_body_accepted() {
    let h = Http::start("r");
    let id = h.open_review("frame");
    // ~100 KiB is well under the 1 MiB cap.
    let modest = "y".repeat(100_000);
    let resp = h
        .send(post_json(
            &format!("/review/{id}/decide"),
            &json!({ "decision": "approved", "feedback": modest }),
        ))
        .await;
    assert_eq!(resp.status(), StatusCode::OK);
}

// ════════════════════════════════════════════════════════════════════════════
// Rate limiting — remote mode only (429 past the per-IP ceiling)
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn rate_limit_trips_in_remote_mode() {
    // Remote mode with a tiny ceiling: the in-process fallback IP shares one
    // counter, so request N+1 is rejected.
    let limits = Limits {
        remote: true,
        rate_limit_per_min: 3,
        ..Limits::default()
    };
    let h = Http::start_mode("r", limits);
    // Build a single router so the limiter state persists across requests.
    let router = h.router();
    let mut statuses = Vec::new();
    for _ in 0..5 {
        let resp = router.clone().oneshot(get("/health")).await.unwrap();
        statuses.push(resp.status());
    }
    assert_eq!(statuses[0], StatusCode::OK);
    assert_eq!(statuses[1], StatusCode::OK);
    assert_eq!(statuses[2], StatusCode::OK);
    assert_eq!(statuses[3], StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(statuses[4], StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn rate_limit_off_in_local_mode() {
    // Local (default) mode never rate-limits, regardless of volume.
    let h = Http::start("r");
    let router = h.router();
    for _ in 0..200 {
        let resp = router.clone().oneshot(get("/health")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}

#[tokio::test]
async fn rate_limit_boundary_exact_ceiling() {
    // Exactly `max` requests pass; the next trips.
    let limits = Limits {
        remote: true,
        rate_limit_per_min: 10,
        ..Limits::default()
    };
    let h = Http::start_mode("r", limits);
    let router = h.router();
    for i in 0..10 {
        let resp = router.clone().oneshot(get("/health")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "request {i} should pass");
    }
    let over = router.clone().oneshot(get("/health")).await.unwrap();
    assert_eq!(over.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn rate_limit_high_ceiling_does_not_trip() {
    // A generous ceiling lets a normal review session through untouched.
    let limits = Limits {
        remote: true,
        rate_limit_per_min: 100_000,
        ..Limits::default()
    };
    let h = Http::start_mode("r", limits);
    let id = h.open_review("frame");
    let router = h.router();
    // A realistic burst: fetch, decide, advance, list feedback.
    let r1 = router
        .clone()
        .oneshot(get(&format!("/api/session/{id}")))
        .await
        .unwrap();
    assert_eq!(r1.status(), StatusCode::OK);
    let r2 = router
        .clone()
        .oneshot(post_json(
            &format!("/review/{id}/decide"),
            &json!({ "decision": "approved" }),
        ))
        .await
        .unwrap();
    assert_eq!(r2.status(), StatusCode::OK);
}

// ════════════════════════════════════════════════════════════════════════════
// CORS posture — remote vs local
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn cors_header_present_in_remote_mode() {
    let limits = Limits {
        remote: true,
        rate_limit_per_min: 100_000,
        ..Limits::default()
    };
    let h = Http::start_mode("r", limits);
    let req = Request::builder()
        .uri("/health")
        .header(header::ORIGIN, "https://app.example.com")
        .body(Body::empty())
        .unwrap();
    let resp = h.send(req).await;
    assert!(resp
        .headers()
        .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN));
}

#[tokio::test]
async fn health_ok_under_remote_limits() {
    let limits = Limits {
        remote: true,
        rate_limit_per_min: 100_000,
        ..Limits::default()
    };
    let h = Http::start_mode("r", limits);
    let resp = h.send(get("/health")).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

// ════════════════════════════════════════════════════════════════════════════
// Cross-run isolation — feedback is scoped to its run
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn feedback_scoped_to_seeded_run() {
    let h = Http::start("run-a");
    create_feedback(&h, "frame", "a-fb", "b").await;
    // A different (unseeded) run slug reads an empty list — its dir is distinct.
    let other = h.send(get("/api/feedback/run-b/frame")).await;
    let parsed: FeedbackListResponse = serde_json::from_slice(&body_bytes(other).await).unwrap();
    assert_eq!(parsed.count, 0);
}

#[tokio::test]
async fn unknown_run_feedback_list_is_empty_not_error() {
    // Listing feedback for a run with no `.darkrun/` state is a graceful empty,
    // not a 500.
    let h = Http::start("r");
    let resp = h.send(get("/api/feedback/never-started/frame")).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let parsed: FeedbackListResponse = serde_json::from_slice(&body_bytes(resp).await).unwrap();
    assert_eq!(parsed.count, 0);
}

// ════════════════════════════════════════════════════════════════════════════
// Method / route shape — unknown routes and wrong methods
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn unknown_route_404() {
    let h = Http::start("r");
    let resp = h.send(get("/no/such/route")).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn wrong_method_on_health_405() {
    let h = Http::start("r");
    let resp = h.send(post_empty("/health")).await;
    assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn wrong_method_on_decide_405() {
    let h = Http::start("r");
    let id = h.open_review("frame");
    let resp = h.send(get(&format!("/review/{id}/decide"))).await;
    assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
}

// ════════════════════════════════════════════════════════════════════════════
// Per-station feedback create + reply matrix (broad coverage across stations)
// ════════════════════════════════════════════════════════════════════════════

macro_rules! station_feedback_tests {
    ($($create:ident, $reply:ident, $update:ident => $station:literal),* $(,)?) => {
        $(
            #[tokio::test]
            async fn $create() {
                let h = Http::start("r");
                let id = create_feedback(&h, $station, "finding", "details").await;
                assert_eq!(id, "FB-01");
                let list = h.send(get(&format!("/api/feedback/{}/{}", h.slug, $station))).await;
                let parsed: FeedbackListResponse =
                    serde_json::from_slice(&body_bytes(list).await).unwrap();
                assert_eq!(parsed.count, 1);
                assert_eq!(parsed.station, $station);
            }

            #[tokio::test]
            async fn $reply() {
                let h = Http::start("r");
                let id = create_feedback(&h, $station, "finding", "details").await;
                let resp = h.send(post_json(
                    &format!("/api/feedback/{}/{}/{id}/replies", h.slug, $station),
                    &json!({ "body": "ack" }),
                )).await;
                assert_eq!(resp.status(), StatusCode::CREATED);
            }

            #[tokio::test]
            async fn $update() {
                let h = Http::start("r");
                let id = create_feedback(&h, $station, "finding", "details").await;
                let resp = h.send(put_json(
                    &format!("/api/feedback/{}/{}/{id}", h.slug, $station),
                    &json!({ "status": "addressed" }),
                )).await;
                assert_eq!(resp.status(), StatusCode::OK);
            }
        )*
    };
}

station_feedback_tests! {
    fb_create_frame, fb_reply_frame, fb_update_frame => "frame",
    fb_create_specify, fb_reply_specify, fb_update_specify => "specify",
    fb_create_shape, fb_reply_shape, fb_update_shape => "shape",
    fb_create_build, fb_reply_build, fb_update_build => "build",
    fb_create_prove, fb_reply_prove, fb_update_prove => "prove",
    fb_create_harden, fb_reply_harden, fb_update_harden => "harden",
}

// ════════════════════════════════════════════════════════════════════════════
// Decide-then-feedback flow per station (review routes back as a finding)
// ════════════════════════════════════════════════════════════════════════════

macro_rules! decide_flow_tests {
    ($($approve:ident, $changes:ident => $station:literal),* $(,)?) => {
        $(
            #[tokio::test]
            async fn $approve() {
                let h = Http::start("r");
                let id = h.open_review($station);
                let resp = h.send(post_json(
                    &format!("/review/{id}/decide"),
                    &json!({ "decision": "approved" }),
                )).await;
                let parsed: ReviewDecisionResponse =
                    serde_json::from_slice(&body_bytes(resp).await).unwrap();
                assert_eq!(parsed.decision, ReviewDecision::Approved);
                let payload = read_session(
                    h.send(get(&format!("/api/session/{id}"))).await
                ).await;
                let SessionPayload::Review(r) = payload else { panic!("review") };
                assert_eq!(r.status, SessionStatus::Approved);
            }

            #[tokio::test]
            async fn $changes() {
                let h = Http::start("r");
                let id = h.open_review($station);
                let resp = h.send(post_json(
                    &format!("/review/{id}/decide"),
                    &json!({ "decision": "changes_requested", "feedback": "needs work" }),
                )).await;
                let parsed: ReviewDecisionResponse =
                    serde_json::from_slice(&body_bytes(resp).await).unwrap();
                assert_eq!(parsed.decision, ReviewDecision::ChangesRequested);
                let payload = read_session(
                    h.send(get(&format!("/api/session/{id}"))).await
                ).await;
                let SessionPayload::Review(r) = payload else { panic!("review") };
                assert_eq!(r.status, SessionStatus::ChangesRequested);
                assert_eq!(r.feedback.as_deref(), Some("needs work"));
            }
        )*
    };
}

decide_flow_tests! {
    decide_approve_frame, decide_changes_frame => "frame",
    decide_approve_specify, decide_changes_specify => "specify",
    decide_approve_shape, decide_changes_shape => "shape",
    decide_approve_build, decide_changes_build => "build",
    decide_approve_prove, decide_changes_prove => "prove",
    decide_approve_harden, decide_changes_harden => "harden",
}

// ════════════════════════════════════════════════════════════════════════════
// Status enum parametrized update coverage
// ════════════════════════════════════════════════════════════════════════════

macro_rules! status_update_tests {
    ($($name:ident => ($token:literal, $variant:expr)),* $(,)?) => {
        $(
            #[tokio::test]
            async fn $name() {
                let h = Http::start("r");
                let id = create_feedback(&h, "frame", "t", "b").await;
                let resp = h.send(put_json(
                    &format!("/api/feedback/{}/frame/{id}", h.slug),
                    &json!({ "status": $token }),
                )).await;
                assert_eq!(resp.status(), StatusCode::OK);
                let list = h.send(get(&format!("/api/feedback/{}/frame", h.slug))).await;
                let lp: FeedbackListResponse =
                    serde_json::from_slice(&body_bytes(list).await).unwrap();
                let item = lp.items.iter().find(|i| i.feedback_id == id).unwrap();
                assert_eq!(item.status, $variant);
            }
        )*
    };
}

status_update_tests! {
    update_to_fixing => ("fixing", FeedbackStatus::Fixing),
    update_to_addressed => ("addressed", FeedbackStatus::Addressed),
    update_to_answered => ("answered", FeedbackStatus::Answered),
    update_to_non_actionable => ("non_actionable", FeedbackStatus::NonActionable),
    update_to_escalated => ("escalated", FeedbackStatus::Escalated),
    update_to_closed => ("closed", FeedbackStatus::Closed),
    update_to_rejected => ("rejected", FeedbackStatus::Rejected),
}

// ════════════════════════════════════════════════════════════════════════════
// Decision canonicalization matrix (exhaustive raw-string coverage)
// ════════════════════════════════════════════════════════════════════════════

macro_rules! canonicalize_tests {
    ($($name:ident => ($raw:literal, $expect:expr)),* $(,)?) => {
        $(
            #[tokio::test]
            async fn $name() {
                let h = Http::start("r");
                let id = h.open_review("frame");
                let resp = h.send(post_json(
                    &format!("/review/{id}/decide"),
                    &json!({ "decision": $raw }),
                )).await;
                let parsed: ReviewDecisionResponse =
                    serde_json::from_slice(&body_bytes(resp).await).unwrap();
                assert_eq!(parsed.decision, $expect);
            }
        )*
    };
}

canonicalize_tests! {
    canon_approved_lower => ("approved", ReviewDecision::Approved),
    canon_approved_upper => ("APPROVED", ReviewDecision::Approved),
    canon_approved_mixed => ("ApProVeD", ReviewDecision::Approved),
    canon_approved_padded => ("  approved  ", ReviewDecision::Approved),
    canon_changes_requested => ("changes_requested", ReviewDecision::ChangesRequested),
    canon_reject => ("reject", ReviewDecision::ChangesRequested),
    canon_deny => ("deny", ReviewDecision::ChangesRequested),
    canon_empty => ("", ReviewDecision::ChangesRequested),
    canon_approve_partial => ("approve", ReviewDecision::ChangesRequested),
    canon_approved_typo => ("approvedd", ReviewDecision::ChangesRequested),
    canon_random => ("xyzzy", ReviewDecision::ChangesRequested),
    canon_yes => ("yes", ReviewDecision::ChangesRequested),
}
