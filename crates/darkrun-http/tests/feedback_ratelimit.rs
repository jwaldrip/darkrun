//! Comprehensive in-process integration tests for the darkrun-http review
//! server, focused on the Feedback CRUD routes, the per-IP rate limiter, the
//! body-size cap, the connection/WebSocket cap configuration, and the uniform
//! JSON error envelopes.
//!
//! Every test drives the crate's PUBLIC surface (`build_router` over an
//! `AppState`) via `tower::ServiceExt::oneshot` — no socket bind — so each case
//! exercises the full middleware stack (CORS, rate limit, body limit) and the
//! handler in one shot. Filesystem state lives in a per-test tempdir.
//!
//! The vocabulary mirrors the factory model: a Run carries Stations, each
//! Station accrues Feedback items routed back from a Checkpoint.
//!
//! Several tests assert ordering and positivity invariants over the crate's
//! default limit constants; those comparisons are compile-time constant, which
//! trips `clippy::assertions_on_constants` even though the guard is meaningful.
#![allow(clippy::assertions_on_constants)]

use std::net::SocketAddr;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use darkrun_api::{
    AuthorType, FeedbackOrigin, FeedbackResolution, FeedbackSeverity, FeedbackStatus,
};
use darkrun_core::StateStore;
use darkrun_http::{
    build_router, AppState, Limits, RouterState, SessionRegistry, DEFAULT_BODY_MAX_BYTES,
    DEFAULT_MAX_CONNECTIONS, DEFAULT_MAX_WS_SESSIONS, DEFAULT_RATE_LIMIT_PER_MIN,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

// ── Fixtures ────────────────────────────────────────────────────────────────

/// Build an `AppState` over a fresh tempdir, leaking the tempdir for the test's
/// lifetime so paths stay valid for the duration of the (short) request flow.
fn test_state() -> AppState {
    let tmp = tempfile::tempdir().expect("tmp");
    let store = StateStore::new(tmp.path());
    std::mem::forget(tmp);
    AppState::new(store, Limits::default())
}

/// Build an `AppState` returning the tempdir guard for callers that want to
/// inspect on-disk state after the request.
fn test_state_with_dir() -> (AppState, tempfile::TempDir) {
    let tmp = tempfile::tempdir().expect("tmp");
    let store = StateStore::new(tmp.path());
    (AppState::new(store, Limits::default()), tmp)
}

/// Remote-mode state with an explicit per-minute rate cap.
fn remote_state(rate_limit_per_min: u64) -> AppState {
    let tmp = tempfile::tempdir().expect("tmp");
    let store = StateStore::new(tmp.path());
    std::mem::forget(tmp);
    AppState::new(
        store,
        Limits {
            remote: true,
            rate_limit_per_min,
            ..Limits::default()
        },
    )
}

async fn body_json(resp: axum::response::Response) -> Value {
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&body).unwrap()
}

async fn body_bytes(resp: axum::response::Response) -> Vec<u8> {
    resp.into_body().collect().await.unwrap().to_bytes().to_vec()
}

/// Issue a single request against a freshly-built router over `state`.
async fn send(state: AppState, req: Request<Body>) -> axum::response::Response {
    build_router(state).oneshot(req).await.unwrap()
}

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

fn post_json(uri: &str, v: &Value) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(v).unwrap()))
        .unwrap()
}

fn put_json(uri: &str, v: &Value) -> Request<Body> {
    Request::builder()
        .method(Method::PUT)
        .uri(uri)
        .header("content-type", "application/json")
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

/// Seed a raw feedback doc on disk with explicit station + status.
fn seed_doc(state: &AppState, run: &str, id: &str, station: &str, status: &str, title: &str) {
    let raw = format!(
        "---\nid: {id}\nstation: {station}\nstatus: {status}\ntitle: {title}\n---\nbody for {id}"
    );
    state.store.write_feedback_raw(run, id, &raw).unwrap();
}

// ── Create: happy path + minting ─────────────────────────────────────────────

#[tokio::test]
async fn create_returns_201_with_minted_id() {
    let state = test_state();
    let resp = send(
        state,
        post_json(
            "/api/feedback/run-a/frame",
            &json!({ "title": "T", "body": "B" }),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let j = body_json(resp).await;
    assert_eq!(j["feedback_id"], "FB-01");
    assert_eq!(j["status"], "pending");
    assert_eq!(j["file"], "feedback/FB-01.md");
    assert_eq!(j["message"], "created FB-01");
}

#[tokio::test]
async fn create_persists_to_disk_with_station() {
    let (state, _tmp) = test_state_with_dir();
    let resp = send(
        state.clone(),
        post_json("/api/feedback/r/build", &json!({ "title": "T", "body": "B" })),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let raw = state.store.read_feedback_raw("r").unwrap();
    let doc = raw.get("FB-01").unwrap();
    assert!(doc.contains("station: build"));
    assert!(doc.contains("status: pending"));
}

#[tokio::test]
async fn create_forces_author_to_user_ignoring_client() {
    let (state, _tmp) = test_state_with_dir();
    send(
        state.clone(),
        post_json(
            "/api/feedback/r/frame",
            &json!({ "title": "T", "body": "B", "author": "intruder" }),
        ),
    )
    .await;
    let raw = state.store.read_feedback_raw("r").unwrap();
    let doc = raw.get("FB-01").unwrap();
    assert!(doc.contains("author: user"));
    assert!(!doc.contains("intruder"));
}

#[tokio::test]
async fn create_trims_title_and_body() {
    let (state, _tmp) = test_state_with_dir();
    send(
        state.clone(),
        post_json(
            "/api/feedback/r/frame",
            &json!({ "title": "  spaced title  ", "body": "  spaced body  " }),
        ),
    )
    .await;
    let raw = state.store.read_feedback_raw("r").unwrap();
    let doc = raw.get("FB-01").unwrap();
    assert!(doc.contains("title: spaced title"));
    assert!(doc.contains("spaced body"));
    // The trailing/leading whitespace must be gone (not "  spaced").
    assert!(!doc.contains("title:   spaced"));
}

#[tokio::test]
async fn create_second_item_mints_fb_02() {
    let state = test_state();
    let app = build_router(state);
    let r1 = app
        .clone()
        .oneshot(post_json("/api/feedback/r/frame", &json!({"title":"a","body":"b"})))
        .await
        .unwrap();
    assert_eq!(body_json(r1).await["feedback_id"], "FB-01");
    let r2 = app
        .oneshot(post_json("/api/feedback/r/frame", &json!({"title":"c","body":"d"})))
        .await
        .unwrap();
    assert_eq!(body_json(r2).await["feedback_id"], "FB-02");
}

#[tokio::test]
async fn create_mint_skips_gaps_to_max_plus_one() {
    let state = test_state();
    seed_doc(&state, "r", "FB-01", "frame", "closed", "a");
    seed_doc(&state, "r", "FB-05", "frame", "closed", "b");
    let resp = send(state, post_json("/api/feedback/r/frame", &json!({"title":"t","body":"b"}))).await;
    assert_eq!(body_json(resp).await["feedback_id"], "FB-06");
}

#[tokio::test]
async fn create_ids_are_per_run_independent() {
    let state = test_state();
    let app = build_router(state);
    let a = app
        .clone()
        .oneshot(post_json("/api/feedback/run-one/frame", &json!({"title":"t","body":"b"})))
        .await
        .unwrap();
    let b = app
        .oneshot(post_json("/api/feedback/run-two/frame", &json!({"title":"t","body":"b"})))
        .await
        .unwrap();
    // Distinct runs each start their own FB-01 sequence.
    assert_eq!(body_json(a).await["feedback_id"], "FB-01");
    assert_eq!(body_json(b).await["feedback_id"], "FB-01");
}

// ── Create: validation ───────────────────────────────────────────────────────

#[tokio::test]
async fn create_empty_title_is_400() {
    let resp = send(
        test_state(),
        post_json("/api/feedback/r/frame", &json!({"title":"","body":"B"})),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let j = body_json(resp).await;
    assert_eq!(j["error"], "title and body are required");
}

#[tokio::test]
async fn create_empty_body_is_400() {
    let resp = send(
        test_state(),
        post_json("/api/feedback/r/frame", &json!({"title":"T","body":""})),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn create_whitespace_only_title_is_400() {
    let resp = send(
        test_state(),
        post_json("/api/feedback/r/frame", &json!({"title":"   ","body":"B"})),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn create_whitespace_only_body_is_400() {
    let resp = send(
        test_state(),
        post_json("/api/feedback/r/frame", &json!({"title":"T","body":"\t\n "})),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn create_both_empty_is_400() {
    let resp = send(
        test_state(),
        post_json("/api/feedback/r/frame", &json!({"title":"","body":""})),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn create_missing_title_field_is_422() {
    // Title is a required (non-Option) field → serde rejects with 422.
    let resp = send(
        test_state(),
        post_json("/api/feedback/r/frame", &json!({"body":"B"})),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn create_missing_body_field_is_422() {
    let resp = send(
        test_state(),
        post_json("/api/feedback/r/frame", &json!({"title":"T"})),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn create_malformed_json_is_400() {
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/feedback/r/frame")
        .header("content-type", "application/json")
        .body(Body::from("{ broken"))
        .unwrap();
    let resp = send(test_state(), req).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn create_missing_content_type_is_415() {
    // axum's Json extractor requires an application/json content-type.
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/feedback/r/frame")
        .body(Body::from(r#"{"title":"T","body":"B"}"#))
        .unwrap();
    let resp = send(test_state(), req).await;
    assert_eq!(resp.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
}

#[tokio::test]
async fn create_wrong_content_type_is_415() {
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/feedback/r/frame")
        .header("content-type", "text/plain")
        .body(Body::from(r#"{"title":"T","body":"B"}"#))
        .unwrap();
    let resp = send(test_state(), req).await;
    assert_eq!(resp.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
}

#[tokio::test]
async fn create_accepts_optional_origin_and_source_ref() {
    // Optional fields present should still create cleanly (server stamps user-visual anyway).
    let resp = send(
        test_state(),
        post_json(
            "/api/feedback/r/frame",
            &json!({"title":"T","body":"B","origin":"user-chat","source_ref":"spec.md"}),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
}

// ── Create: body size cap ────────────────────────────────────────────────────

#[tokio::test]
async fn create_oversize_body_is_413() {
    let huge = vec![b'a'; DEFAULT_BODY_MAX_BYTES + 1];
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/feedback/r/frame")
        .header("content-type", "application/json")
        .body(Body::from(huge))
        .unwrap();
    let resp = send(test_state(), req).await;
    assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn create_body_at_cap_is_not_413() {
    // A valid JSON document sized just under the cap must pass the limit layer.
    let pad = "x".repeat(DEFAULT_BODY_MAX_BYTES / 2);
    let payload = json!({ "title": "T", "body": pad });
    let bytes = serde_json::to_vec(&payload).unwrap();
    assert!(bytes.len() <= DEFAULT_BODY_MAX_BYTES);
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/feedback/r/frame")
        .header("content-type", "application/json")
        .body(Body::from(bytes))
        .unwrap();
    let resp = send(test_state(), req).await;
    assert_eq!(resp.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn create_large_body_persists_full_content() {
    let (state, _tmp) = test_state_with_dir();
    let pad = "Z".repeat(10_000);
    send(
        state.clone(),
        post_json("/api/feedback/r/frame", &json!({"title":"T","body": pad})),
    )
    .await;
    let raw = state.store.read_feedback_raw("r").unwrap();
    assert!(raw.get("FB-01").unwrap().contains(&"Z".repeat(10_000)));
}

// ── List ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn list_unknown_run_is_empty() {
    let resp = send(test_state(), get("/api/feedback/ghost/frame")).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let j = body_json(resp).await;
    assert_eq!(j["run"], "ghost");
    assert_eq!(j["station"], "frame");
    assert_eq!(j["count"], 0);
    assert_eq!(j["items"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn list_echoes_run_and_station() {
    let resp = send(test_state(), get("/api/feedback/my-run/my-station")).await;
    let j = body_json(resp).await;
    assert_eq!(j["run"], "my-run");
    assert_eq!(j["station"], "my-station");
}

#[tokio::test]
async fn list_returns_seeded_item() {
    let state = test_state();
    seed_doc(&state, "r", "FB-01", "frame", "pending", "Hello");
    let resp = send(state, get("/api/feedback/r/frame")).await;
    let j = body_json(resp).await;
    assert_eq!(j["count"], 1);
    assert_eq!(j["items"][0]["feedback_id"], "FB-01");
    assert_eq!(j["items"][0]["title"], "Hello");
    assert_eq!(j["items"][0]["status"], "pending");
}

#[tokio::test]
async fn list_filters_by_station() {
    let state = test_state();
    seed_doc(&state, "r", "FB-01", "frame", "pending", "A");
    seed_doc(&state, "r", "FB-02", "build", "pending", "B");
    let resp = send(state, get("/api/feedback/r/frame")).await;
    let j = body_json(resp).await;
    assert_eq!(j["count"], 1);
    assert_eq!(j["items"][0]["feedback_id"], "FB-01");
}

#[tokio::test]
async fn list_station_with_no_recorded_station_matches_any() {
    let state = test_state();
    // No station field → legacy-tolerant, matches every station.
    state
        .store
        .write_feedback_raw("r", "FB-01", "---\nstatus: pending\ntitle: legacy\n---\nx")
        .unwrap();
    let resp = send(state, get("/api/feedback/r/whatever-station")).await;
    let j = body_json(resp).await;
    assert_eq!(j["count"], 1);
}

#[tokio::test]
async fn list_is_sorted_by_feedback_id() {
    let state = test_state();
    seed_doc(&state, "r", "FB-03", "frame", "pending", "third");
    seed_doc(&state, "r", "FB-01", "frame", "pending", "first");
    seed_doc(&state, "r", "FB-02", "frame", "pending", "second");
    let resp = send(state, get("/api/feedback/r/frame")).await;
    let j = body_json(resp).await;
    let ids: Vec<&str> = j["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["feedback_id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["FB-01", "FB-02", "FB-03"]);
}

#[tokio::test]
async fn list_item_carries_origin_and_author_type() {
    let state = test_state();
    seed_doc(&state, "r", "FB-01", "frame", "pending", "t");
    let resp = send(state, get("/api/feedback/r/frame")).await;
    let j = body_json(resp).await;
    // Default origin is user-visual → human author type.
    assert_eq!(j["items"][0]["origin"], "user-visual");
    assert_eq!(j["items"][0]["author_type"], "human");
    assert_eq!(j["items"][0]["author"], "user");
}

#[tokio::test]
async fn list_count_matches_items_length() {
    let state = test_state();
    for i in 1..=5 {
        seed_doc(&state, "r", &format!("FB-0{i}"), "frame", "pending", "t");
    }
    let resp = send(state, get("/api/feedback/r/frame")).await;
    let j = body_json(resp).await;
    assert_eq!(j["count"], 5);
    assert_eq!(j["items"].as_array().unwrap().len(), 5);
}

#[tokio::test]
async fn list_create_roundtrip() {
    let state = test_state();
    let app = build_router(state);
    for t in ["one", "two", "three"] {
        app.clone()
            .oneshot(post_json("/api/feedback/rt/frame", &json!({"title":t,"body":"b"})))
            .await
            .unwrap();
    }
    let resp = app.oneshot(get("/api/feedback/rt/frame")).await.unwrap();
    let j = body_json(resp).await;
    assert_eq!(j["count"], 3);
}

#[tokio::test]
async fn list_reflects_every_status_token() {
    let state = test_state();
    let statuses = [
        "pending",
        "fixing",
        "addressed",
        "answered",
        "non_actionable",
        "escalated",
        "closed",
        "rejected",
    ];
    for (i, s) in statuses.iter().enumerate() {
        seed_doc(&state, "r", &format!("FB-{:02}", i + 1), "frame", s, "t");
    }
    let resp = send(state, get("/api/feedback/r/frame")).await;
    let j = body_json(resp).await;
    let got: Vec<&str> = j["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["status"].as_str().unwrap())
        .collect();
    for s in statuses {
        assert!(got.contains(&s), "missing status {s}");
    }
}

#[tokio::test]
async fn list_unknown_status_token_folds_to_pending() {
    let state = test_state();
    seed_doc(&state, "r", "FB-01", "frame", "wild-unknown", "t");
    let resp = send(state, get("/api/feedback/r/frame")).await;
    let j = body_json(resp).await;
    assert_eq!(j["items"][0]["status"], "pending");
}

// ── Update ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn update_status_changes_on_disk() {
    let (state, _tmp) = test_state_with_dir();
    seed_doc(&state, "r", "FB-01", "frame", "pending", "t");
    let resp = send(
        state.clone(),
        put_json("/api/feedback/r/frame/FB-01", &json!({"status":"closed"})),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let j = body_json(resp).await;
    assert_eq!(j["feedback_id"], "FB-01");
    assert!(j["updated_fields"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f == "status"));
    assert!(state
        .store
        .read_feedback_raw("r")
        .unwrap()
        .get("FB-01")
        .unwrap()
        .contains("status: closed"));
}

#[tokio::test]
async fn update_closed_by_persists() {
    let (state, _tmp) = test_state_with_dir();
    seed_doc(&state, "r", "FB-01", "frame", "pending", "t");
    let resp = send(
        state.clone(),
        put_json("/api/feedback/r/frame/FB-01", &json!({"closed_by":"unit-09"})),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(state
        .store
        .read_feedback_raw("r")
        .unwrap()
        .get("FB-01")
        .unwrap()
        .contains("closed_by: unit-09"));
}

#[tokio::test]
async fn update_reports_all_changed_fields() {
    let state = test_state();
    seed_doc(&state, "r", "FB-01", "frame", "pending", "t");
    let resp = send(
        state,
        put_json(
            "/api/feedback/r/frame/FB-01",
            &json!({"status":"closed","closed_by":"u","resolution":"inline_fix"}),
        ),
    )
    .await;
    let j = body_json(resp).await;
    let fields: Vec<&str> = j["updated_fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap())
        .collect();
    assert!(fields.contains(&"status"));
    assert!(fields.contains(&"closed_by"));
    assert!(fields.contains(&"resolution"));
}

#[tokio::test]
async fn update_resolution_only_is_acknowledged_not_persisted() {
    let (state, _tmp) = test_state_with_dir();
    seed_doc(&state, "r", "FB-01", "frame", "pending", "t");
    let resp = send(
        state.clone(),
        put_json("/api/feedback/r/frame/FB-01", &json!({"resolution":"question"})),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let j = body_json(resp).await;
    assert!(j["updated_fields"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f == "resolution"));
    // resolution is wire-only; status stays pending on disk.
    assert!(state
        .store
        .read_feedback_raw("r")
        .unwrap()
        .get("FB-01")
        .unwrap()
        .contains("status: pending"));
}

#[tokio::test]
async fn update_empty_body_is_400() {
    let state = test_state();
    seed_doc(&state, "r", "FB-01", "frame", "pending", "t");
    let resp = send(state, put_json("/api/feedback/r/frame/FB-01", &json!({}))).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let j = body_json(resp).await;
    assert!(j["error"]
        .as_str()
        .unwrap()
        .contains("status"));
}

#[tokio::test]
async fn update_unknown_id_is_404() {
    let state = test_state();
    seed_doc(&state, "r", "FB-01", "frame", "pending", "t");
    let resp = send(
        state,
        put_json("/api/feedback/r/frame/FB-99", &json!({"status":"closed"})),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let j = body_json(resp).await;
    assert_eq!(j["error"], "feedback not found");
    assert_eq!(j["id"], "FB-99");
}

#[tokio::test]
async fn update_unknown_run_is_404() {
    let resp = send(
        test_state(),
        put_json("/api/feedback/ghost/frame/FB-01", &json!({"status":"closed"})),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn update_ignores_station_segment() {
    // The update handler ignores the station path segment; a wrong station still
    // finds the item by id within the run.
    let (state, _tmp) = test_state_with_dir();
    seed_doc(&state, "r", "FB-01", "frame", "pending", "t");
    let resp = send(
        state.clone(),
        put_json("/api/feedback/r/WRONG-STATION/FB-01", &json!({"status":"closed"})),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(state
        .store
        .read_feedback_raw("r")
        .unwrap()
        .get("FB-01")
        .unwrap()
        .contains("status: closed"));
}

#[tokio::test]
async fn update_malformed_json_is_400() {
    let state = test_state();
    seed_doc(&state, "r", "FB-01", "frame", "pending", "t");
    let req = Request::builder()
        .method(Method::PUT)
        .uri("/api/feedback/r/frame/FB-01")
        .header("content-type", "application/json")
        .body(Body::from("{not json"))
        .unwrap();
    let resp = send(state, req).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn update_invalid_status_value_is_422() {
    // FeedbackStatus deserializes strictly; an unknown token is a serde 422.
    let state = test_state();
    seed_doc(&state, "r", "FB-01", "frame", "pending", "t");
    let resp = send(
        state,
        put_json("/api/feedback/r/frame/FB-01", &json!({"status":"banana"})),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn update_then_list_reflects_status() {
    let state = test_state();
    seed_doc(&state, "r", "FB-01", "frame", "pending", "t");
    let app = build_router(state);
    app.clone()
        .oneshot(put_json("/api/feedback/r/frame/FB-01", &json!({"status":"answered"})))
        .await
        .unwrap();
    let resp = app.oneshot(get("/api/feedback/r/frame")).await.unwrap();
    let j = body_json(resp).await;
    assert_eq!(j["items"][0]["status"], "answered");
}

#[tokio::test]
async fn update_idempotent_repeated_close() {
    let state = test_state();
    seed_doc(&state, "r", "FB-01", "frame", "pending", "t");
    let app = build_router(state);
    let first = app
        .clone()
        .oneshot(put_json("/api/feedback/r/frame/FB-01", &json!({"status":"closed"})))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    let second = app
        .oneshot(put_json("/api/feedback/r/frame/FB-01", &json!({"status":"closed"})))
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::OK);
}

// ── Delete ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn delete_closed_item_succeeds() {
    let (state, _tmp) = test_state_with_dir();
    seed_doc(&state, "r", "FB-01", "frame", "closed", "t");
    let resp = send(state.clone(), delete("/api/feedback/r/frame/FB-01")).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let j = body_json(resp).await;
    assert_eq!(j["deleted"], true);
    assert_eq!(j["feedback_id"], "FB-01");
    assert!(state.store.read_feedback_raw("r").unwrap().is_empty());
}

#[tokio::test]
async fn delete_rejected_item_succeeds() {
    let state = test_state();
    seed_doc(&state, "r", "FB-01", "frame", "rejected", "t");
    let resp = send(state, delete("/api/feedback/r/frame/FB-01")).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn delete_answered_item_succeeds() {
    let state = test_state();
    seed_doc(&state, "r", "FB-01", "frame", "answered", "t");
    let resp = send(state, delete("/api/feedback/r/frame/FB-01")).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn delete_pending_item_is_409() {
    let (state, _tmp) = test_state_with_dir();
    seed_doc(&state, "r", "FB-01", "frame", "pending", "t");
    let resp = send(state.clone(), delete("/api/feedback/r/frame/FB-01")).await;
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let j = body_json(resp).await;
    assert_eq!(j["error"], "cannot delete an open feedback item");
    assert_eq!(j["feedback_id"], "FB-01");
    assert_eq!(j["status"], "pending");
    // Still present on disk.
    assert!(state
        .store
        .read_feedback_raw("r")
        .unwrap()
        .contains_key("FB-01"));
}

#[tokio::test]
async fn delete_fixing_item_is_409() {
    let state = test_state();
    seed_doc(&state, "r", "FB-01", "frame", "fixing", "t");
    let resp = send(state, delete("/api/feedback/r/frame/FB-01")).await;
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn delete_escalated_item_succeeds() {
    // escalated is a human-intervention waypoint, NOT gate-blocking → deletable.
    let state = test_state();
    seed_doc(&state, "r", "FB-01", "frame", "escalated", "t");
    let resp = send(state, delete("/api/feedback/r/frame/FB-01")).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn delete_addressed_item_succeeds() {
    let state = test_state();
    seed_doc(&state, "r", "FB-01", "frame", "addressed", "t");
    let resp = send(state, delete("/api/feedback/r/frame/FB-01")).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn delete_non_actionable_item_succeeds() {
    let state = test_state();
    seed_doc(&state, "r", "FB-01", "frame", "non_actionable", "t");
    let resp = send(state, delete("/api/feedback/r/frame/FB-01")).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn delete_unknown_id_is_404() {
    let resp = send(test_state(), delete("/api/feedback/r/frame/FB-77")).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let j = body_json(resp).await;
    assert_eq!(j["error"], "feedback not found");
}

#[tokio::test]
async fn delete_unknown_run_is_404() {
    let resp = send(test_state(), delete("/api/feedback/ghost/frame/FB-01")).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_then_list_is_empty() {
    let state = test_state();
    seed_doc(&state, "r", "FB-01", "frame", "closed", "t");
    let app = build_router(state);
    app.clone()
        .oneshot(delete("/api/feedback/r/frame/FB-01"))
        .await
        .unwrap();
    let resp = app.oneshot(get("/api/feedback/r/frame")).await.unwrap();
    assert_eq!(body_json(resp).await["count"], 0);
}

#[tokio::test]
async fn delete_one_keeps_siblings() {
    let (state, _tmp) = test_state_with_dir();
    seed_doc(&state, "r", "FB-01", "frame", "closed", "a");
    seed_doc(&state, "r", "FB-02", "frame", "closed", "b");
    let resp = send(state.clone(), delete("/api/feedback/r/frame/FB-01")).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let raw = state.store.read_feedback_raw("r").unwrap();
    assert!(!raw.contains_key("FB-01"));
    assert!(raw.contains_key("FB-02"));
}

// ── Replies ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn reply_appends_and_returns_index_zero() {
    let (state, _tmp) = test_state_with_dir();
    seed_doc(&state, "r", "FB-01", "frame", "pending", "t");
    let resp = send(
        state.clone(),
        post_json("/api/feedback/r/frame/FB-01/replies", &json!({"body":"first reply"})),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let j = body_json(resp).await;
    assert_eq!(j["reply_index"], 0);
    assert_eq!(j["feedback_id"], "FB-01");
    assert!(state
        .store
        .read_feedback_raw("r")
        .unwrap()
        .get("FB-01")
        .unwrap()
        .contains("first reply"));
}

#[tokio::test]
async fn reply_second_index_is_one() {
    let state = test_state();
    seed_doc(&state, "r", "FB-01", "frame", "pending", "t");
    let app = build_router(state);
    let r1 = app
        .clone()
        .oneshot(post_json("/api/feedback/r/frame/FB-01/replies", &json!({"body":"a"})))
        .await
        .unwrap();
    assert_eq!(body_json(r1).await["reply_index"], 0);
    let r2 = app
        .oneshot(post_json("/api/feedback/r/frame/FB-01/replies", &json!({"body":"b"})))
        .await
        .unwrap();
    assert_eq!(body_json(r2).await["reply_index"], 1);
}

#[tokio::test]
async fn reply_close_as_answered_transitions_status() {
    let (state, _tmp) = test_state_with_dir();
    seed_doc(&state, "r", "FB-01", "frame", "pending", "t");
    let resp = send(
        state.clone(),
        post_json(
            "/api/feedback/r/frame/FB-01/replies",
            &json!({"body":"resolved","close_as_answered":true}),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    assert_eq!(body_json(resp).await["status"], "answered");
    assert!(state
        .store
        .read_feedback_raw("r")
        .unwrap()
        .get("FB-01")
        .unwrap()
        .contains("status: answered"));
}

#[tokio::test]
async fn reply_without_close_keeps_status() {
    let state = test_state();
    seed_doc(&state, "r", "FB-01", "frame", "pending", "t");
    let resp = send(
        state,
        post_json("/api/feedback/r/frame/FB-01/replies", &json!({"body":"note"})),
    )
    .await;
    assert_eq!(body_json(resp).await["status"], "pending");
}

#[tokio::test]
async fn reply_close_false_keeps_status() {
    let state = test_state();
    seed_doc(&state, "r", "FB-01", "frame", "pending", "t");
    let resp = send(
        state,
        post_json(
            "/api/feedback/r/frame/FB-01/replies",
            &json!({"body":"note","close_as_answered":false}),
        ),
    )
    .await;
    assert_eq!(body_json(resp).await["status"], "pending");
}

#[tokio::test]
async fn reply_uses_default_author_user() {
    let (state, _tmp) = test_state_with_dir();
    seed_doc(&state, "r", "FB-01", "frame", "pending", "t");
    send(
        state.clone(),
        post_json("/api/feedback/r/frame/FB-01/replies", &json!({"body":"hi"})),
    )
    .await;
    assert!(state
        .store
        .read_feedback_raw("r")
        .unwrap()
        .get("FB-01")
        .unwrap()
        .contains("user: hi"));
}

#[tokio::test]
async fn reply_honors_explicit_author() {
    let (state, _tmp) = test_state_with_dir();
    seed_doc(&state, "r", "FB-01", "frame", "pending", "t");
    send(
        state.clone(),
        post_json(
            "/api/feedback/r/frame/FB-01/replies",
            &json!({"body":"hi","author":"worker-7"}),
        ),
    )
    .await;
    assert!(state
        .store
        .read_feedback_raw("r")
        .unwrap()
        .get("FB-01")
        .unwrap()
        .contains("worker-7: hi"));
}

#[tokio::test]
async fn reply_blank_author_falls_back_to_user() {
    let (state, _tmp) = test_state_with_dir();
    seed_doc(&state, "r", "FB-01", "frame", "pending", "t");
    send(
        state.clone(),
        post_json(
            "/api/feedback/r/frame/FB-01/replies",
            &json!({"body":"hi","author":"   "}),
        ),
    )
    .await;
    assert!(state
        .store
        .read_feedback_raw("r")
        .unwrap()
        .get("FB-01")
        .unwrap()
        .contains("user: hi"));
}

#[tokio::test]
async fn reply_trims_body() {
    let (state, _tmp) = test_state_with_dir();
    seed_doc(&state, "r", "FB-01", "frame", "pending", "t");
    send(
        state.clone(),
        post_json("/api/feedback/r/frame/FB-01/replies", &json!({"body":"  spaced  "})),
    )
    .await;
    let doc = state.store.read_feedback_raw("r").unwrap();
    let content = doc.get("FB-01").unwrap();
    assert!(content.contains("user: spaced"));
    assert!(!content.contains("user:   spaced"));
}

#[tokio::test]
async fn reply_empty_body_is_400() {
    let state = test_state();
    seed_doc(&state, "r", "FB-01", "frame", "pending", "t");
    let resp = send(
        state,
        post_json("/api/feedback/r/frame/FB-01/replies", &json!({"body":""})),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(resp).await["error"], "reply body is required");
}

#[tokio::test]
async fn reply_whitespace_body_is_400() {
    let state = test_state();
    seed_doc(&state, "r", "FB-01", "frame", "pending", "t");
    let resp = send(
        state,
        post_json("/api/feedback/r/frame/FB-01/replies", &json!({"body":"  \n  "})),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn reply_missing_body_field_is_422() {
    let state = test_state();
    seed_doc(&state, "r", "FB-01", "frame", "pending", "t");
    let resp = send(
        state,
        post_json("/api/feedback/r/frame/FB-01/replies", &json!({"author":"x"})),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn reply_unknown_id_is_404() {
    let state = test_state();
    seed_doc(&state, "r", "FB-01", "frame", "pending", "t");
    let resp = send(
        state,
        post_json("/api/feedback/r/frame/FB-99/replies", &json!({"body":"x"})),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn reply_unknown_run_is_404() {
    let resp = send(
        test_state(),
        post_json("/api/feedback/ghost/frame/FB-01/replies", &json!({"body":"x"})),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn reply_then_close_via_two_calls() {
    let state = test_state();
    seed_doc(&state, "r", "FB-01", "frame", "pending", "t");
    let app = build_router(state);
    app.clone()
        .oneshot(post_json("/api/feedback/r/frame/FB-01/replies", &json!({"body":"q?"})))
        .await
        .unwrap();
    let resp = app
        .oneshot(post_json(
            "/api/feedback/r/frame/FB-01/replies",
            &json!({"body":"a!","close_as_answered":true}),
        ))
        .await
        .unwrap();
    let j = body_json(resp).await;
    assert_eq!(j["reply_index"], 1);
    assert_eq!(j["status"], "answered");
}

#[tokio::test]
async fn reply_preserves_existing_replies() {
    let (state, _tmp) = test_state_with_dir();
    seed_doc(&state, "r", "FB-01", "frame", "pending", "t");
    let app = build_router(state.clone());
    app.clone()
        .oneshot(post_json("/api/feedback/r/frame/FB-01/replies", &json!({"body":"alpha"})))
        .await
        .unwrap();
    app.oneshot(post_json("/api/feedback/r/frame/FB-01/replies", &json!({"body":"beta"})))
        .await
        .unwrap();
    let content = state.store.read_feedback_raw("r").unwrap();
    let doc = content.get("FB-01").unwrap();
    assert!(doc.contains("alpha"));
    assert!(doc.contains("beta"));
}

// ── Rate limit ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn rate_limit_over_cap_is_429() {
    let app = build_router(remote_state(3));
    let mut statuses = Vec::new();
    for _ in 0..4 {
        let resp = app.clone().oneshot(get("/health")).await.unwrap();
        statuses.push(resp.status());
    }
    assert_eq!(statuses[0], StatusCode::OK);
    assert_eq!(statuses[1], StatusCode::OK);
    assert_eq!(statuses[2], StatusCode::OK);
    assert_eq!(statuses[3], StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn rate_limit_under_cap_all_ok() {
    let app = build_router(remote_state(10));
    for _ in 0..10 {
        let resp = app.clone().oneshot(get("/health")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}

#[tokio::test]
async fn rate_limit_cap_of_one_blocks_second() {
    let app = build_router(remote_state(1));
    let first = app.clone().oneshot(get("/health")).await.unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    let second = app.oneshot(get("/health")).await.unwrap();
    assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn rate_limit_disabled_in_local_mode() {
    let app = build_router(test_state());
    for _ in 0..(DEFAULT_RATE_LIMIT_PER_MIN + 5) {
        let resp = app.clone().oneshot(get("/health")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}

#[tokio::test]
async fn rate_limit_counts_across_distinct_routes() {
    // The fixed-window counter is per-IP, not per-route: hits to different
    // endpoints share the same bucket.
    let app = build_router(remote_state(3));
    let s1 = app.clone().oneshot(get("/health")).await.unwrap().status();
    let s2 = app
        .clone()
        .oneshot(get("/api/feedback/r/frame"))
        .await
        .unwrap()
        .status();
    let s3 = app.clone().oneshot(get("/health")).await.unwrap().status();
    let s4 = app
        .oneshot(get("/api/feedback/r/frame"))
        .await
        .unwrap()
        .status();
    assert_eq!(s1, StatusCode::OK);
    assert_eq!(s2, StatusCode::OK);
    assert_eq!(s3, StatusCode::OK);
    assert_eq!(s4, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn rate_limit_429_has_empty_body() {
    let app = build_router(remote_state(1));
    app.clone().oneshot(get("/health")).await.unwrap();
    let resp = app.oneshot(get("/health")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    // The middleware returns a bare StatusCode → no JSON body.
    assert!(body_bytes(resp).await.is_empty());
}

#[tokio::test]
async fn rate_limit_does_not_apply_to_404_in_local_mode() {
    let app = build_router(test_state());
    for _ in 0..(DEFAULT_RATE_LIMIT_PER_MIN + 5) {
        let resp = app.clone().oneshot(get("/no/such/route")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}

#[tokio::test]
async fn rate_limit_blocks_before_handler_runs() {
    // Over the cap, even a would-be-201 create gets short-circuited to 429
    // before touching the store.
    let tmp = tempfile::tempdir().expect("tmp");
    let state = AppState::new(
        StateStore::new(tmp.path()),
        Limits {
            remote: true,
            rate_limit_per_min: 1,
            ..Limits::default()
        },
    );
    let app = build_router(state.clone());
    app.clone().oneshot(get("/health")).await.unwrap();
    let resp = app
        .oneshot(post_json("/api/feedback/r/frame", &json!({"title":"t","body":"b"})))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    // Nothing was written.
    assert!(state.store.read_feedback_raw("r").unwrap().is_empty());
    drop(tmp);
}

#[tokio::test]
async fn rate_limit_zero_cap_blocks_everything() {
    // A cap of 0 means the first request already exceeds it.
    let app = build_router(remote_state(0));
    let resp = app.oneshot(get("/health")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn rate_limiter_check_under_cap() {
    let limiter = darkrun_http::RateLimiter::new();
    let ip = std::net::IpAddr::V4(std::net::Ipv4Addr::new(10, 0, 0, 1));
    assert!(limiter.check(ip, 3));
    assert!(limiter.check(ip, 3));
    assert!(limiter.check(ip, 3));
}

#[tokio::test]
async fn rate_limiter_check_over_cap_returns_false() {
    let limiter = darkrun_http::RateLimiter::new();
    let ip = std::net::IpAddr::V4(std::net::Ipv4Addr::new(10, 0, 0, 2));
    assert!(limiter.check(ip, 2));
    assert!(limiter.check(ip, 2));
    assert!(!limiter.check(ip, 2));
    assert!(!limiter.check(ip, 2));
}

#[tokio::test]
async fn rate_limiter_distinct_ips_have_separate_buckets() {
    let limiter = darkrun_http::RateLimiter::new();
    let a = std::net::IpAddr::V4(std::net::Ipv4Addr::new(1, 1, 1, 1));
    let b = std::net::IpAddr::V4(std::net::Ipv4Addr::new(2, 2, 2, 2));
    assert!(limiter.check(a, 1));
    assert!(!limiter.check(a, 1));
    // b has its own counter, untouched by a's flood.
    assert!(limiter.check(b, 1));
    assert!(!limiter.check(b, 1));
}

#[tokio::test]
async fn rate_limiter_cap_of_zero_rejects_first() {
    let limiter = darkrun_http::RateLimiter::new();
    let ip = std::net::IpAddr::V4(std::net::Ipv4Addr::new(3, 3, 3, 3));
    assert!(!limiter.check(ip, 0));
}

#[tokio::test]
async fn rate_limiter_ipv6_tracked_independently() {
    let limiter = darkrun_http::RateLimiter::new();
    let v4 = std::net::IpAddr::V4(std::net::Ipv4Addr::new(9, 9, 9, 9));
    let v6 = std::net::IpAddr::V6(std::net::Ipv6Addr::LOCALHOST);
    assert!(limiter.check(v4, 1));
    assert!(!limiter.check(v4, 1));
    assert!(limiter.check(v6, 1));
}

// ── Limits / connection / ws cap configuration ───────────────────────────────

#[tokio::test]
async fn limits_default_values() {
    let l = Limits::default();
    assert_eq!(l.rate_limit_per_min, DEFAULT_RATE_LIMIT_PER_MIN);
    assert_eq!(l.max_connections, DEFAULT_MAX_CONNECTIONS);
    assert_eq!(l.max_ws_sessions, DEFAULT_MAX_WS_SESSIONS);
    assert!(!l.remote);
}

#[tokio::test]
async fn limits_default_constants_sane() {
    assert_eq!(DEFAULT_RATE_LIMIT_PER_MIN, 60);
    assert_eq!(DEFAULT_MAX_CONNECTIONS, 256);
    assert_eq!(DEFAULT_MAX_WS_SESSIONS, 128);
    assert_eq!(DEFAULT_BODY_MAX_BYTES, 1_048_576);
    assert!(DEFAULT_MAX_CONNECTIONS > 0);
    assert!(DEFAULT_MAX_WS_SESSIONS > 0);
}

#[tokio::test]
async fn limits_is_copy_and_clone() {
    let l = Limits {
        remote: true,
        max_connections: 7,
        max_ws_sessions: 3,
        rate_limit_per_min: 9,
    };
    let copied = l; // Copy
    let cloned = l;
    assert_eq!(copied.max_connections, 7);
    assert_eq!(cloned.max_ws_sessions, 3);
    assert_eq!(l.rate_limit_per_min, 9);
    assert!(l.remote);
}

#[tokio::test]
async fn limits_custom_caps_preserved_on_appstate() {
    let tmp = tempfile::tempdir().unwrap();
    let limits = Limits {
        remote: true,
        max_connections: 11,
        max_ws_sessions: 5,
        rate_limit_per_min: 42,
    };
    let state = AppState::new(StateStore::new(tmp.path()), limits);
    assert_eq!(state.limits.max_connections, 11);
    assert_eq!(state.limits.max_ws_sessions, 5);
    assert_eq!(state.limits.rate_limit_per_min, 42);
    assert!(state.limits.remote);
}

#[tokio::test]
async fn ws_slot_cap_acquire_and_release() {
    let reg = SessionRegistry::new();
    let s1 = reg.try_acquire_ws_slot(2);
    let s2 = reg.try_acquire_ws_slot(2);
    assert!(s1.is_some());
    assert!(s2.is_some());
    // Cap of 2 reached → third is refused.
    assert!(reg.try_acquire_ws_slot(2).is_none());
    drop(s1);
    // Released a slot → a new one can be acquired.
    assert!(reg.try_acquire_ws_slot(2).is_some());
    drop(s2);
}

#[tokio::test]
async fn ws_slot_cap_of_zero_refuses() {
    let reg = SessionRegistry::new();
    assert!(reg.try_acquire_ws_slot(0).is_none());
}

#[tokio::test]
async fn ws_slot_release_on_drop_frees_capacity() {
    let reg = SessionRegistry::new();
    {
        let _slot = reg.try_acquire_ws_slot(1).expect("first slot");
        assert!(reg.try_acquire_ws_slot(1).is_none());
    }
    // Scope ended → slot dropped → capacity restored.
    assert!(reg.try_acquire_ws_slot(1).is_some());
}

#[tokio::test]
async fn ws_slot_default_cap_allows_many() {
    let reg = SessionRegistry::new();
    let mut held = Vec::new();
    for _ in 0..DEFAULT_MAX_WS_SESSIONS {
        held.push(reg.try_acquire_ws_slot(DEFAULT_MAX_WS_SESSIONS).expect("slot"));
    }
    // All slots up to the cap acquired; one more is refused.
    assert!(reg.try_acquire_ws_slot(DEFAULT_MAX_WS_SESSIONS).is_none());
}

#[tokio::test]
async fn router_state_projects_app_and_limiter() {
    // RouterState is public; FromRef projections drive the handler/middleware
    // extraction. Construct one and confirm the projected limits survive.
    let tmp = tempfile::tempdir().unwrap();
    let limits = Limits {
        remote: true,
        rate_limit_per_min: 5,
        ..Limits::default()
    };
    let app = AppState::new(StateStore::new(tmp.path()), limits);
    let rs = RouterState {
        app: app.clone(),
        limiter: darkrun_http::RateLimiter::new(),
    };
    assert!(rs.app.limits.remote);
    assert_eq!(rs.app.limits.rate_limit_per_min, 5);
}

// ── Error envelopes ──────────────────────────────────────────────────────────

#[tokio::test]
async fn not_found_envelope_shape() {
    let resp = send(test_state(), put_json("/api/feedback/r/s/FB-9", &json!({"status":"closed"}))).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let j = body_json(resp).await;
    assert!(j.get("error").is_some());
    assert!(j.get("id").is_some());
    assert_eq!(j["error"], "feedback not found");
}

#[tokio::test]
async fn bad_request_envelope_shape() {
    let state = test_state();
    seed_doc(&state, "r", "FB-01", "frame", "pending", "t");
    let resp = send(state, put_json("/api/feedback/r/frame/FB-01", &json!({}))).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let j = body_json(resp).await;
    assert!(j.get("error").is_some());
    assert!(j["error"].is_string());
}

#[tokio::test]
async fn conflict_envelope_carries_status() {
    let state = test_state();
    seed_doc(&state, "r", "FB-01", "frame", "pending", "t");
    let resp = send(state, delete("/api/feedback/r/frame/FB-01")).await;
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let j = body_json(resp).await;
    assert_eq!(j["error"], "cannot delete an open feedback item");
    assert_eq!(j["feedback_id"], "FB-01");
    assert_eq!(j["status"], "pending");
}

#[tokio::test]
async fn error_envelope_is_json_content_type() {
    let resp = send(test_state(), delete("/api/feedback/r/frame/FB-9")).await;
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(ct.contains("application/json"), "got content-type {ct}");
}

#[tokio::test]
async fn create_success_is_json_content_type() {
    let resp = send(
        test_state(),
        post_json("/api/feedback/r/frame", &json!({"title":"t","body":"b"})),
    )
    .await;
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(ct.contains("application/json"));
}

// ── Routing edges ────────────────────────────────────────────────────────────

#[tokio::test]
async fn feedback_list_wrong_method_delete_on_collection_is_405() {
    // The collection route only supports GET/POST; DELETE is method-not-allowed.
    let resp = send(test_state(), delete("/api/feedback/r/frame")).await;
    assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn feedback_item_get_is_405() {
    // The item route supports PUT/DELETE only; GET is method-not-allowed.
    let resp = send(test_state(), get("/api/feedback/r/frame/FB-01")).await;
    assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn replies_get_is_405() {
    let resp = send(test_state(), get("/api/feedback/r/frame/FB-01/replies")).await;
    assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn unknown_route_is_404() {
    let resp = send(test_state(), get("/api/feedback")).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn feedback_collection_post_then_get_consistent() {
    let state = test_state();
    let app = build_router(state);
    app.clone()
        .oneshot(post_json("/api/feedback/r/frame", &json!({"title":"x","body":"y"})))
        .await
        .unwrap();
    let resp = app.oneshot(get("/api/feedback/r/frame")).await.unwrap();
    let j = body_json(resp).await;
    assert_eq!(j["count"], 1);
    assert_eq!(j["items"][0]["title"], "x");
}

// ── Special characters / encoding ────────────────────────────────────────────

#[tokio::test]
async fn create_title_with_colon_roundtrips() {
    let state = test_state();
    let app = build_router(state);
    app.clone()
        .oneshot(post_json(
            "/api/feedback/r/frame",
            &json!({"title":"Fix: the spec","body":"b"}),
        ))
        .await
        .unwrap();
    let resp = app.oneshot(get("/api/feedback/r/frame")).await.unwrap();
    let j = body_json(resp).await;
    // A colon in the title must survive the frontmatter quote/unquote cycle.
    assert_eq!(j["items"][0]["title"], "Fix: the spec");
}

#[tokio::test]
async fn create_title_with_quotes_roundtrips() {
    let state = test_state();
    let app = build_router(state);
    app.clone()
        .oneshot(post_json(
            "/api/feedback/r/frame",
            &json!({"title":"say \"hi\"","body":"b"}),
        ))
        .await
        .unwrap();
    let resp = app.oneshot(get("/api/feedback/r/frame")).await.unwrap();
    let j = body_json(resp).await;
    assert_eq!(j["items"][0]["title"], "say \"hi\"");
}

#[tokio::test]
async fn create_body_with_unicode_roundtrips() {
    let state = test_state();
    let app = build_router(state);
    app.clone()
        .oneshot(post_json(
            "/api/feedback/r/frame",
            &json!({"title":"t","body":"café — naïve — 日本語 — 🚀"}),
        ))
        .await
        .unwrap();
    let resp = app.oneshot(get("/api/feedback/r/frame")).await.unwrap();
    let j = body_json(resp).await;
    let body = j["items"][0]["body"].as_str().unwrap();
    assert!(body.contains("café"));
    assert!(body.contains("日本語"));
    assert!(body.contains("🚀"));
}

#[tokio::test]
async fn create_multiline_body_roundtrips() {
    let state = test_state();
    let app = build_router(state);
    app.clone()
        .oneshot(post_json(
            "/api/feedback/r/frame",
            &json!({"title":"t","body":"line one\nline two\nline three"}),
        ))
        .await
        .unwrap();
    let resp = app.oneshot(get("/api/feedback/r/frame")).await.unwrap();
    let j = body_json(resp).await;
    let body = j["items"][0]["body"].as_str().unwrap();
    assert!(body.contains("line one"));
    assert!(body.contains("line two"));
    assert!(body.contains("line three"));
}

#[tokio::test]
async fn reply_body_with_colon_roundtrips_to_disk() {
    let (state, _tmp) = test_state_with_dir();
    seed_doc(&state, "r", "FB-01", "frame", "pending", "t");
    send(
        state.clone(),
        post_json(
            "/api/feedback/r/frame/FB-01/replies",
            &json!({"body":"see: the issue"}),
        ),
    )
    .await;
    // Reply persisted and reloads — re-read confirms the colon survived.
    let content = state.store.read_feedback_raw("r").unwrap();
    let doc = darkrun_http_reparse(content.get("FB-01").unwrap());
    assert!(doc.contains("see: the issue"));
}

/// Helper to re-stringify a doc (the on-disk content is already a String, this
/// just makes the intent of "reload" explicit for the colon test).
fn darkrun_http_reparse(raw: &str) -> String {
    raw.to_string()
}

// ── Origin / author-type wire mapping (via list) ─────────────────────────────

#[tokio::test]
async fn list_origin_user_chat_is_human() {
    let state = test_state();
    state
        .store
        .write_feedback_raw(
            "r",
            "FB-01",
            "---\nstation: frame\nstatus: pending\norigin: user-chat\ntitle: t\n---\nb",
        )
        .unwrap();
    let resp = send(state, get("/api/feedback/r/frame")).await;
    let j = body_json(resp).await;
    assert_eq!(j["items"][0]["origin"], "user-chat");
    assert_eq!(j["items"][0]["author_type"], "human");
}

#[tokio::test]
async fn list_origin_agent_is_agent_author_type() {
    let state = test_state();
    state
        .store
        .write_feedback_raw(
            "r",
            "FB-01",
            "---\nstation: frame\nstatus: pending\norigin: agent\ntitle: t\n---\nb",
        )
        .unwrap();
    let resp = send(state, get("/api/feedback/r/frame")).await;
    let j = body_json(resp).await;
    assert_eq!(j["items"][0]["origin"], "agent");
    assert_eq!(j["items"][0]["author_type"], "agent");
}

#[tokio::test]
async fn list_origin_adversarial_review_is_agent() {
    let state = test_state();
    state
        .store
        .write_feedback_raw(
            "r",
            "FB-01",
            "---\nstation: frame\nstatus: pending\norigin: adversarial-review\ntitle: t\n---\nb",
        )
        .unwrap();
    let resp = send(state, get("/api/feedback/r/frame")).await;
    let j = body_json(resp).await;
    assert_eq!(j["items"][0]["origin"], "adversarial-review");
    assert_eq!(j["items"][0]["author_type"], "agent");
}

#[tokio::test]
async fn list_origin_external_pr_is_human() {
    let state = test_state();
    state
        .store
        .write_feedback_raw(
            "r",
            "FB-01",
            "---\nstation: frame\nstatus: pending\norigin: external-pr\ntitle: t\n---\nb",
        )
        .unwrap();
    let resp = send(state, get("/api/feedback/r/frame")).await;
    let j = body_json(resp).await;
    assert_eq!(j["items"][0]["author_type"], "human");
}

#[tokio::test]
async fn list_unknown_origin_folds_to_user_visual() {
    let state = test_state();
    state
        .store
        .write_feedback_raw(
            "r",
            "FB-01",
            "---\nstation: frame\nstatus: pending\norigin: not-a-real-origin\ntitle: t\n---\nb",
        )
        .unwrap();
    let resp = send(state, get("/api/feedback/r/frame")).await;
    let j = body_json(resp).await;
    assert_eq!(j["items"][0]["origin"], "user-visual");
}

#[tokio::test]
async fn list_severity_blocker_surfaces() {
    let state = test_state();
    state
        .store
        .write_feedback_raw(
            "r",
            "FB-01",
            "---\nstation: frame\nstatus: pending\nseverity: blocker\ntitle: t\n---\nb",
        )
        .unwrap();
    let resp = send(state, get("/api/feedback/r/frame")).await;
    let j = body_json(resp).await;
    assert_eq!(j["items"][0]["severity"], "blocker");
}

#[tokio::test]
async fn list_no_severity_omits_field() {
    let state = test_state();
    seed_doc(&state, "r", "FB-01", "frame", "pending", "t");
    let resp = send(state, get("/api/feedback/r/frame")).await;
    let j = body_json(resp).await;
    // severity is skip_serializing_if None → absent, not null.
    assert!(j["items"][0].get("severity").is_none());
}

#[tokio::test]
async fn list_visit_counter_parsed() {
    let state = test_state();
    state
        .store
        .write_feedback_raw(
            "r",
            "FB-01",
            "---\nstation: frame\nstatus: pending\nvisit: 3\ntitle: t\n---\nb",
        )
        .unwrap();
    let resp = send(state, get("/api/feedback/r/frame")).await;
    let j = body_json(resp).await;
    assert_eq!(j["items"][0]["visit"], 3);
}

#[tokio::test]
async fn list_closed_by_surfaces() {
    let state = test_state();
    state
        .store
        .write_feedback_raw(
            "r",
            "FB-01",
            "---\nstation: frame\nstatus: closed\nclosed_by: unit-12\ntitle: t\n---\nb",
        )
        .unwrap();
    let resp = send(state, get("/api/feedback/r/frame")).await;
    let j = body_json(resp).await;
    assert_eq!(j["items"][0]["closed_by"], "unit-12");
}

// ── Wire-type serde roundtrips (pure, fast) ──────────────────────────────────

#[test]
fn feedback_status_serde_tokens() {
    let cases = [
        (FeedbackStatus::Pending, "\"pending\""),
        (FeedbackStatus::Fixing, "\"fixing\""),
        (FeedbackStatus::Addressed, "\"addressed\""),
        (FeedbackStatus::Answered, "\"answered\""),
        (FeedbackStatus::NonActionable, "\"non_actionable\""),
        (FeedbackStatus::Escalated, "\"escalated\""),
        (FeedbackStatus::Closed, "\"closed\""),
        (FeedbackStatus::Rejected, "\"rejected\""),
    ];
    for (val, tok) in cases {
        assert_eq!(serde_json::to_string(&val).unwrap(), tok);
        let back: FeedbackStatus = serde_json::from_str(tok).unwrap();
        assert_eq!(back, val);
    }
}

#[test]
fn feedback_status_canonicalize_known() {
    assert_eq!(FeedbackStatus::canonicalize("fixing"), FeedbackStatus::Fixing);
    assert_eq!(FeedbackStatus::canonicalize("CLOSED"), FeedbackStatus::Closed);
    assert_eq!(
        FeedbackStatus::canonicalize("  Rejected  "),
        FeedbackStatus::Rejected
    );
    assert_eq!(
        FeedbackStatus::canonicalize("non_actionable"),
        FeedbackStatus::NonActionable
    );
}

#[test]
fn feedback_status_canonicalize_unknown_is_pending() {
    assert_eq!(FeedbackStatus::canonicalize("xyz"), FeedbackStatus::Pending);
    assert_eq!(FeedbackStatus::canonicalize(""), FeedbackStatus::Pending);
}

#[test]
fn feedback_status_as_str_roundtrips_canonicalize() {
    for s in [
        FeedbackStatus::Pending,
        FeedbackStatus::Fixing,
        FeedbackStatus::Addressed,
        FeedbackStatus::Answered,
        FeedbackStatus::NonActionable,
        FeedbackStatus::Escalated,
        FeedbackStatus::Closed,
        FeedbackStatus::Rejected,
    ] {
        assert_eq!(FeedbackStatus::canonicalize(s.as_str()), s);
    }
}

#[test]
fn feedback_status_blocks_gate_only_pending_fixing() {
    assert!(FeedbackStatus::Pending.blocks_gate());
    assert!(FeedbackStatus::Fixing.blocks_gate());
    for s in [
        FeedbackStatus::Addressed,
        FeedbackStatus::Answered,
        FeedbackStatus::NonActionable,
        FeedbackStatus::Escalated,
        FeedbackStatus::Closed,
        FeedbackStatus::Rejected,
    ] {
        assert!(!s.blocks_gate(), "{s:?} should not block gate");
    }
}

#[test]
fn feedback_origin_serde_kebab_case() {
    let cases = [
        (FeedbackOrigin::AdversarialReview, "\"adversarial-review\""),
        (FeedbackOrigin::UserVisual, "\"user-visual\""),
        (FeedbackOrigin::UserChat, "\"user-chat\""),
        (FeedbackOrigin::ExternalPr, "\"external-pr\""),
        (FeedbackOrigin::Agent, "\"agent\""),
    ];
    for (val, tok) in cases {
        assert_eq!(serde_json::to_string(&val).unwrap(), tok);
        let back: FeedbackOrigin = serde_json::from_str(tok).unwrap();
        assert_eq!(back, val);
    }
}

#[test]
fn feedback_origin_author_type_mapping() {
    assert_eq!(FeedbackOrigin::UserVisual.author_type(), AuthorType::Human);
    assert_eq!(FeedbackOrigin::UserChat.author_type(), AuthorType::Human);
    assert_eq!(FeedbackOrigin::UserQuestion.author_type(), AuthorType::Human);
    assert_eq!(FeedbackOrigin::UserRevisit.author_type(), AuthorType::Human);
    assert_eq!(FeedbackOrigin::ExternalPr.author_type(), AuthorType::Human);
    assert_eq!(FeedbackOrigin::ExternalMr.author_type(), AuthorType::Human);
    assert_eq!(FeedbackOrigin::Agent.author_type(), AuthorType::Agent);
    assert_eq!(FeedbackOrigin::Drift.author_type(), AuthorType::Agent);
    assert_eq!(FeedbackOrigin::Discovery.author_type(), AuthorType::Agent);
    assert_eq!(
        FeedbackOrigin::AdversarialReview.author_type(),
        AuthorType::Agent
    );
    assert_eq!(FeedbackOrigin::StudioReview.author_type(), AuthorType::Agent);
    assert_eq!(FeedbackOrigin::EngineReview.author_type(), AuthorType::Agent);
}

#[test]
fn feedback_severity_serde_tokens() {
    for (val, tok) in [
        (FeedbackSeverity::Blocker, "\"blocker\""),
        (FeedbackSeverity::High, "\"high\""),
        (FeedbackSeverity::Medium, "\"medium\""),
        (FeedbackSeverity::Low, "\"low\""),
    ] {
        assert_eq!(serde_json::to_string(&val).unwrap(), tok);
        let back: FeedbackSeverity = serde_json::from_str(tok).unwrap();
        assert_eq!(back, val);
    }
}

#[test]
fn feedback_resolution_serde_tokens() {
    for (val, tok) in [
        (FeedbackResolution::Question, "\"question\""),
        (FeedbackResolution::InlineFix, "\"inline_fix\""),
        (FeedbackResolution::StageRevisit, "\"stage_revisit\""),
    ] {
        assert_eq!(serde_json::to_string(&val).unwrap(), tok);
        let back: FeedbackResolution = serde_json::from_str(tok).unwrap();
        assert_eq!(back, val);
    }
}

#[test]
fn author_type_serde_tokens() {
    assert_eq!(serde_json::to_string(&AuthorType::Human).unwrap(), "\"human\"");
    assert_eq!(serde_json::to_string(&AuthorType::Agent).unwrap(), "\"agent\"");
    assert_eq!(
        serde_json::to_string(&AuthorType::System).unwrap(),
        "\"system\""
    );
}

// ── Determinism / idempotency on filesystem ──────────────────────────────────

#[tokio::test]
async fn list_is_deterministic_across_repeat_calls() {
    let state = test_state();
    for i in 1..=4 {
        seed_doc(&state, "r", &format!("FB-0{i}"), "frame", "pending", "t");
    }
    let app = build_router(state);
    let a = body_json(app.clone().oneshot(get("/api/feedback/r/frame")).await.unwrap()).await;
    let b = body_json(app.oneshot(get("/api/feedback/r/frame")).await.unwrap()).await;
    assert_eq!(a, b);
}

#[tokio::test]
async fn update_does_not_disturb_other_items() {
    let (state, _tmp) = test_state_with_dir();
    seed_doc(&state, "r", "FB-01", "frame", "pending", "first");
    seed_doc(&state, "r", "FB-02", "frame", "pending", "second");
    send(
        state.clone(),
        put_json("/api/feedback/r/frame/FB-01", &json!({"status":"closed"})),
    )
    .await;
    let raw = state.store.read_feedback_raw("r").unwrap();
    assert!(raw.get("FB-01").unwrap().contains("status: closed"));
    assert!(raw.get("FB-02").unwrap().contains("status: pending"));
}

#[tokio::test]
async fn create_preserves_existing_items() {
    let (state, _tmp) = test_state_with_dir();
    seed_doc(&state, "r", "FB-01", "frame", "closed", "old");
    send(
        state.clone(),
        post_json("/api/feedback/r/frame", &json!({"title":"new","body":"b"})),
    )
    .await;
    let raw = state.store.read_feedback_raw("r").unwrap();
    assert!(raw.contains_key("FB-01"));
    assert!(raw.contains_key("FB-02"));
}

// ── Body limit applies to replies + update ──────────────────────────────────

#[tokio::test]
async fn reply_oversize_body_is_413() {
    let state = test_state();
    seed_doc(&state, "r", "FB-01", "frame", "pending", "t");
    let huge = vec![b'a'; DEFAULT_BODY_MAX_BYTES + 1];
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/feedback/r/frame/FB-01/replies")
        .header("content-type", "application/json")
        .body(Body::from(huge))
        .unwrap();
    let resp = send(state, req).await;
    assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn update_oversize_body_is_413() {
    let state = test_state();
    seed_doc(&state, "r", "FB-01", "frame", "pending", "t");
    let huge = vec![b'a'; DEFAULT_BODY_MAX_BYTES + 1];
    let req = Request::builder()
        .method(Method::PUT)
        .uri("/api/feedback/r/frame/FB-01")
        .header("content-type", "application/json")
        .body(Body::from(huge))
        .unwrap();
    let resp = send(state, req).await;
    assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

// ── End-to-end bound server: feedback over a real socket ─────────────────────

#[tokio::test]
async fn bound_server_create_and_list_over_socket() {
    let tmp = tempfile::tempdir().unwrap();
    let store = StateStore::new(tmp.path());
    let state = AppState::new(store, Limits::default());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let bound = listener.local_addr().unwrap();
    let app = build_router(state);
    let handle = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });

    // POST a feedback item over raw HTTP/1.1.
    let payload = r#"{"title":"socket title","body":"socket body"}"#;
    let post = format!(
        "POST /api/feedback/sock/frame HTTP/1.1\r\nHost: {bound}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    );
    let resp = raw_send(&bound.to_string(), &post).await;
    assert!(resp.contains("201") || resp.contains("FB-01"), "got: {resp}");
    assert!(resp.contains("FB-01"));

    // GET the list back.
    let get = format!("GET /api/feedback/sock/frame HTTP/1.1\r\nHost: {bound}\r\nConnection: close\r\n\r\n");
    let listed = raw_send(&bound.to_string(), &get).await;
    assert!(listed.contains("socket title"), "got: {listed}");
    handle.abort();
}

/// Minimal raw HTTP request/response over a TCP socket.
async fn raw_send(authority: &str, request: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(authority).await.unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    String::from_utf8_lossy(&buf).to_string()
}

// ── Persistence-fault arms (write/remove failures) ───────────────────────────

/// The feedback handlers surface a filesystem write/remove failure as a 500
/// rather than a panic. Create fails when the collection path is unwritable;
/// update/reply/delete fail when the (readable) collection dir is read-only so
/// the read succeeds but the mutation cannot persist.
#[tokio::test]
async fn feedback_writes_surface_persistence_faults() {
    use std::os::unix::fs::PermissionsExt;

    // create: a feedback dir that is a FILE makes the persisting write fail.
    let (state, _dir) = test_state_with_dir();
    std::fs::create_dir_all(state.store.run_dir("wfail")).unwrap();
    std::fs::write(state.store.feedback_dir("wfail"), b"x").unwrap();
    let resp = send(
        state,
        post_json("/api/feedback/wfail/frame", &json!({ "title": "T", "body": "B" })),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);

    // update / reply: seed a settled doc and make the doc FILE read-only — the
    // read succeeds but truncating it for the rewrite fails.
    let (state2, _dir2) = test_state_with_dir();
    seed_doc(&state2, "ro", "fb-1", "frame", "closed", "T");
    let doc_path = state2.store.feedback_dir("ro").join("fb-1.md");
    std::fs::set_permissions(&doc_path, std::fs::Permissions::from_mode(0o444)).unwrap();

    let upd = send(
        state2.clone(),
        put_json("/api/feedback/ro/frame/fb-1", &json!({ "status": "answered" })),
    )
    .await;
    assert_eq!(upd.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let rep = send(
        state2.clone(),
        post_json("/api/feedback/ro/frame/fb-1/replies", &json!({ "body": "noted" })),
    )
    .await;
    assert_eq!(rep.status(), StatusCode::INTERNAL_SERVER_ERROR);
    std::fs::set_permissions(&doc_path, std::fs::Permissions::from_mode(0o644)).unwrap();

    // delete: a read-only collection DIR lets the read succeed but blocks the
    // unlink (removing an entry needs write on the directory).
    let (state3, _dir3) = test_state_with_dir();
    seed_doc(&state3, "rod", "fb-2", "frame", "closed", "T");
    let fbdir = state3.store.feedback_dir("rod");
    std::fs::set_permissions(&fbdir, std::fs::Permissions::from_mode(0o555)).unwrap();
    let del = send(state3.clone(), delete("/api/feedback/rod/frame/fb-2")).await;
    assert_eq!(del.status(), StatusCode::INTERNAL_SERVER_ERROR);
    std::fs::set_permissions(&fbdir, std::fs::Permissions::from_mode(0o755)).unwrap();
}
