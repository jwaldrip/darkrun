//! Coverage for the surface-aware verification + visual-review routes:
//!   - `POST /visual-review/:id/annotate` — annotate an output screenshot,
//!     producing FEEDBACK.
//!   - `POST /api/proof/:run`             — attach a run's objective evidence.
//!   - `GET  /api/proof/:run`             — read it back.
//!   - `GET  /api/session/:id`            — read the view artifact-browser +
//!     visual-review + proof session payloads.
//!
//! In-process routes are driven via `tower::ServiceExt::oneshot`; the live
//! WebSocket push driven by the visual-review POST is exercised over a real
//! loopback bind via `tokio-tungstenite`.

use std::net::SocketAddr;
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use darkrun_api::session::{
    ProofSessionPayload, ViewArtifact, ViewArtifactKind, VisualReviewSessionPayload,
};
use darkrun_api::{
    BenchProof, Proof, SessionPayload, SessionStatus, Surface, ViewMode, ViewScope,
    ViewSessionPayload, ViewStatus,
};
use darkrun_core::StateStore;
use darkrun_http::{build_router, AppState, Limits};
use futures_util::{SinkExt, StreamExt};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tower::ServiceExt;

// ── Fixtures ────────────────────────────────────────────────────────────────

fn test_state() -> AppState {
    let tmp = tempfile::tempdir().expect("tmp");
    let store = StateStore::new(tmp.path());
    std::mem::forget(tmp);
    AppState::new(store, Limits::default())
}

fn view(id: &str) -> SessionPayload {
    SessionPayload::View(ViewSessionPayload {
        session_id: id.into(),
        status: ViewStatus::Open,
        run_slug: "run".into(),
        scope: ViewScope::Station,
        artifacts: vec![
            ViewArtifact {
                id: "a1".into(),
                path: "build/index.html".into(),
                kind: ViewArtifactKind::File,
                label: "index.html".into(),
                thumbnail_url: None,
                url: Some("/view/v/a1".into()),
            },
            ViewArtifact {
                id: "a2".into(),
                path: "build/home.png".into(),
                kind: ViewArtifactKind::Screenshot,
                label: "Home".into(),
                thumbnail_url: Some("/thumb/a2.png".into()),
                url: Some("/view/v/a2".into()),
            },
        ],
        factory: None,
        station: Some("build".into()),
        artifact: Some("a2".into()),
        mode: ViewMode::Viewer,
        boot_port: None,
        boot_command: None,
    })
}

fn visual_review(id: &str) -> SessionPayload {
    SessionPayload::VisualReview(VisualReviewSessionPayload {
        session_id: id.into(),
        status: SessionStatus::Pending,
        run_slug: Some("run".into()),
        station: Some("build".into()),
        artifact_id: Some("a2".into()),
        artifact_path: Some("build/home.png".into()),
        screenshot_url: Some("/shot/home.png".into()),
        prompt: Some("Review the home page".into()),
        annotations: None,
    })
}

fn proof_session(id: &str) -> SessionPayload {
    SessionPayload::Proof(ProofSessionPayload {
        session_id: id.into(),
        status: SessionStatus::Pending,
        run_slug: Some("run".into()),
        station: Some("prove".into()),
        proof: Proof::bench(
            Surface::Library,
            BenchProof {
                p50: Some(0.4),
                p95: Some(1.1),
                p99: Some(2.2),
                throughput: Some(48_000.0),
                samples: Some(1024),
            },
        ),
    })
}

// ── Request helpers ───────────────────────────────────────────────────────────

async fn send(app: axum::Router, req: Request<Body>) -> axum::response::Response {
    app.oneshot(req).await.unwrap()
}

fn post_json(uri: &str, v: &Value) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(v).unwrap()))
        .unwrap()
}

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

// ════════════════════════════════════════════════════════════════════════════
// GET /api/session/:id — view artifact browser + visual-review + proof reads
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn get_view_session_exposes_artifact_browser() {
    let state = test_state();
    state.sessions.upsert(view("v"));
    let resp = send(build_router(state), get("/api/session/v")).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["session_type"], "view");
    assert_eq!(json["scope"], "station");
    assert_eq!(json["artifacts"][0]["id"], "a1");
    assert_eq!(json["artifacts"][1]["kind"], "screenshot");
    assert_eq!(json["artifacts"][1]["thumbnail_url"], "/thumb/a2.png");
}

#[tokio::test]
async fn get_visual_review_session_exposes_screenshot() {
    let state = test_state();
    state.sessions.upsert(visual_review("vr"));
    let resp = send(build_router(state), get("/api/session/vr")).await;
    let json = body_json(resp).await;
    assert_eq!(json["session_type"], "visual_review");
    assert_eq!(json["artifact_id"], "a2");
    assert_eq!(json["screenshot_url"], "/shot/home.png");
}

#[tokio::test]
async fn get_proof_session_exposes_numbers() {
    let state = test_state();
    state.sessions.upsert(proof_session("pf"));
    let resp = send(build_router(state), get("/api/session/pf")).await;
    let json = body_json(resp).await;
    assert_eq!(json["session_type"], "proof");
    assert_eq!(json["proof"]["surface"], "library");
    assert_eq!(json["proof"]["bench"]["p95"], 1.1);
}

// ════════════════════════════════════════════════════════════════════════════
// POST /visual-review/:id/annotate
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn visual_review_annotate_produces_feedback() {
    let state = test_state();
    state.sessions.upsert(visual_review("vr"));
    let body = json!({
        "annotations": {
            "pins": [{ "x": 0.5, "y": 0.25, "note": "button too small" }],
            "comments": ["fix the header"]
        },
        "title": "home review"
    });
    let resp = send(build_router(state.clone()), post_json("/visual-review/vr/annotate", &body)).await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let json = body_json(resp).await;
    assert_eq!(json["ok"], true);
    assert_eq!(json["pins"], 1);
    assert_eq!(json["comments"], 1);
    let fb_id = json["feedback_id"].as_str().unwrap().to_string();

    // The session recorded the annotations + flipped to decided.
    let SessionPayload::VisualReview(vr) = state.sessions.get("vr").unwrap() else {
        panic!("expected visual review");
    };
    assert_eq!(vr.status, SessionStatus::Decided);
    let anns = vr.annotations.expect("annotations recorded");
    assert_eq!(anns.pins[0].note, "button too small");

    // The feedback item is listable on the run's station.
    let resp = send(build_router(state), get("/api/feedback/run/build")).await;
    let list = body_json(resp).await;
    assert_eq!(list["count"], 1);
    assert_eq!(list["items"][0]["feedback_id"], fb_id);
    assert_eq!(list["items"][0]["origin"], "user-visual");
    assert!(list["items"][0]["body"]
        .as_str()
        .unwrap()
        .contains("button too small"));
}

#[tokio::test]
async fn request_unit_reset_surfaces_a_persistence_fault() {
    use std::os::unix::fs::PermissionsExt;
    let state = test_state();
    // Seed a unit, then make its doc file read-only so the read succeeds but the
    // reset-flag write fails → 500.
    let unit = darkrun_core::domain::Unit {
        slug: "u".into(),
        frontmatter: darkrun_core::domain::UnitFrontmatter { station: Some("build".into()), ..Default::default() },
        title: "u".into(),
        body: String::new(),
    };
    state.store.write_unit("run", &unit).unwrap();
    let path = state.store.units_dir("run").join("u.md");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).unwrap();
    let resp = send(build_router(state), post_json("/api/unit/run/u/reset", &json!({}))).await;
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
}

#[tokio::test]
async fn visual_review_annotate_surfaces_a_persistence_fault() {
    let state = test_state();
    state.sessions.upsert(visual_review("vr"));
    // Plant a FILE where the run's feedback dir belongs → persisting the
    // visual-review feedback fails, surfacing a 500 rather than a panic.
    std::fs::create_dir_all(state.store.run_dir("run")).unwrap();
    std::fs::write(state.store.feedback_dir("run"), b"x").unwrap();
    let body = json!({
        "annotations": { "pins": [{ "x": 0.5, "y": 0.25, "note": "n" }], "comments": ["c"] },
        "title": "review"
    });
    let resp = send(build_router(state), post_json("/visual-review/vr/annotate", &body)).await;
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn visual_review_empty_annotation_is_422() {
    let state = test_state();
    state.sessions.upsert(visual_review("vr"));
    let resp = send(
        build_router(state),
        post_json("/visual-review/vr/annotate", &json!({ "annotations": {} })),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn visual_review_unknown_session_is_404() {
    let resp = send(
        build_router(test_state()),
        post_json(
            "/visual-review/ghost/annotate",
            &json!({ "annotations": { "comments": ["x"] } }),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn visual_review_on_non_review_session_is_409() {
    let state = test_state();
    state.sessions.upsert(view("v"));
    let resp = send(
        build_router(state),
        post_json(
            "/visual-review/v/annotate",
            &json!({ "annotations": { "comments": ["x"] } }),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let json = body_json(resp).await;
    assert_eq!(json["error"], "session is not a visual-review session");
}

// ════════════════════════════════════════════════════════════════════════════
// POST/GET /api/proof/:run
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn attach_then_get_proof_roundtrips() {
    let state = test_state();
    let body = json!({
        "proof": {
            "surface": "web_ui",
            "web": {
                "vitals": { "lcp": 980.0, "cls": 0.02 },
                "audits": [{ "name": "contrast", "value": "4.8:1", "pass": true }],
                "screenshot_url": "/shot.png"
            }
        },
        "station": "prove"
    });
    let resp = send(build_router(state.clone()), post_json("/api/proof/run", &body)).await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let json = body_json(resp).await;
    assert_eq!(json["ok"], true);
    assert_eq!(json["run"], "run");
    assert_eq!(json["surface"], "web_ui");
    assert_eq!(json["block_matches_surface"], true);

    let resp = send(build_router(state), get("/api/proof/run")).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let got = body_json(resp).await;
    assert_eq!(got["run"], "run");
    assert_eq!(got["station"], "prove");
    assert_eq!(got["proof"]["surface"], "web_ui");
    assert_eq!(got["proof"]["web"]["vitals"]["lcp"], 980.0);
    assert_eq!(got["proof"]["web"]["audits"][0]["pass"], true);
}

#[tokio::test]
async fn get_proof_unknown_run_is_404() {
    let resp = send(build_router(test_state()), get("/api/proof/ghost")).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn attach_proof_block_surface_mismatch_is_422() {
    // A web_ui surface carrying only a bench block does not match its route.
    let state = test_state();
    let body = json!({
        "proof": {
            "surface": "web_ui",
            "bench": { "p50": 1.0 }
        }
    });
    let resp = send(build_router(state), post_json("/api/proof/run", &body)).await;
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let json = body_json(resp).await;
    assert_eq!(json["error"], "proof block does not match surface");
}

#[tokio::test]
async fn attach_proof_last_write_wins() {
    let state = test_state();
    let app = build_router(state.clone());
    let _ = send(
        app.clone(),
        post_json("/api/proof/run", &json!({ "proof": { "surface": "cli" } })),
    )
    .await;
    let _ = send(
        app,
        post_json(
            "/api/proof/run",
            &json!({ "proof": { "surface": "library", "bench": { "p50": 2.0 } } }),
        ),
    )
    .await;
    let (proof, _) = state.proofs.get("run").unwrap();
    assert_eq!(proof.surface, Surface::Library);
    assert_eq!(proof.bench.unwrap().p50, Some(2.0));
}

// ════════════════════════════════════════════════════════════════════════════
// WebSocket push driven by the visual-review annotate route
// ════════════════════════════════════════════════════════════════════════════

async fn spawn(state: AppState) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let bound = listener.local_addr().unwrap();
    let app = build_router(state);
    let handle = tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await;
    });
    (bound, handle)
}

async fn next_json<S>(socket: &mut S) -> Value
where
    S: futures_util::Stream<
            Item = Result<
                tokio_tungstenite::tungstenite::Message,
                tokio_tungstenite::tungstenite::Error,
            >,
        > + Unpin,
{
    let text = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match socket.next().await {
                Some(Ok(WsMessage::Text(t))) => return t.to_string(),
                Some(Ok(_)) => continue,
                other => panic!("expected text frame, got {other:?}"),
            }
        }
    })
    .await
    .expect("timed out waiting for a frame");
    serde_json::from_str(&text).expect("valid json")
}

#[tokio::test]
async fn ws_pushes_visual_review_annotation_update() {
    let state = test_state();
    state.sessions.upsert(visual_review("ws-vr"));
    let registry = state.sessions.clone();
    let (bound, handle) = spawn(state).await;

    let (mut socket, _) =
        tokio_tungstenite::connect_async(format!("ws://{bound}/ws/session/ws-vr"))
            .await
            .unwrap();
    let snap = next_json(&mut socket).await;
    assert_eq!(snap["status"], "pending");

    // Mutate exactly as the handler would: record annotations + flip status.
    let SessionPayload::VisualReview(mut vr) = registry.get("ws-vr").unwrap() else {
        unreachable!()
    };
    vr.annotations = Some(darkrun_api::VisualReviewAnnotations {
        pins: vec![darkrun_api::VisualReviewPin {
            x: 0.1,
            y: 0.1,
            note: "tweak".into(),
        }],
        comments: vec![],
    });
    vr.status = SessionStatus::Decided;
    registry.upsert(SessionPayload::VisualReview(vr));

    let upd = next_json(&mut socket).await;
    assert_eq!(upd["status"], "decided");
    assert_eq!(upd["annotations"]["pins"][0]["note"], "tweak");

    socket.send(WsMessage::Close(None)).await.ok();
    handle.abort();
}

#[tokio::test]
async fn visual_review_without_run_slug_is_unprocessable() {
    // A visual-review session that names no run can't be routed to a feedback file.
    let state = test_state();
    let SessionPayload::VisualReview(mut vr) = visual_review("vr") else { unreachable!() };
    vr.run_slug = None;
    state.sessions.upsert(SessionPayload::VisualReview(vr));
    let body = json!({ "annotations": { "pins": [], "comments": ["x"] } });
    let resp = send(build_router(state), post_json("/visual-review/vr/annotate", &body)).await;
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn visual_review_title_defaults_from_artifact_path_or_a_generic_label() {
    // No explicit title + an artifact_path → "Visual review: <path>".
    let state = test_state();
    state.sessions.upsert(visual_review("vr"));
    let resp = send(
        build_router(state.clone()),
        post_json("/visual-review/vr/annotate", &json!({ "annotations": { "comments": ["fix"] } })),
    ).await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let list = body_json(send(build_router(state), get("/api/feedback/run/build")).await).await;
    assert!(list["items"][0]["title"].as_str().unwrap().contains("build/home.png"));

    // No artifact_path → the generic "Visual review of output" label.
    let state2 = test_state();
    let SessionPayload::VisualReview(mut vr) = visual_review("vr2") else { unreachable!() };
    vr.artifact_path = None;
    state2.sessions.upsert(SessionPayload::VisualReview(vr));
    let resp2 = send(
        build_router(state2.clone()),
        post_json("/visual-review/vr2/annotate", &json!({ "annotations": { "comments": ["fix"] } })),
    ).await;
    assert_eq!(resp2.status(), StatusCode::CREATED);
    let list2 = body_json(send(build_router(state2), get("/api/feedback/run/build")).await).await;
    assert!(list2["items"][0]["title"].as_str().unwrap().contains("Visual review of output"));
}

#[tokio::test]
async fn live_connections_starts_at_zero() {
    // A fresh state has no WebSocket subscribers (the engine's "launch the
    // desktop" signal).
    let state = test_state();
    assert_eq!(state.sessions.live_connections(), 0);
}
