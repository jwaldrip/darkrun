//! Comprehensive coverage for the VISUAL interactive session routes:
//!   - `POST /question/:id/answer`   — answer a visual question.
//!   - `POST /direction/:id/select`  — give a design direction (choose + annotate).
//!   - `POST /picker/:id/select`     — choose a picker option.
//!
//! Plus the `GET /api/session/:id` reads of the question/direction/picker
//! payloads and the live WebSocket push that every mutating POST drives.
//!
//! In-process routes are driven via `tower::ServiceExt::oneshot`; the WebSocket
//! push is exercised over a real loopback bind via `tokio-tungstenite`.

use std::net::SocketAddr;
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use darkrun_api::{
    DirectionArchetype, DirectionSessionPayload, PickerKind, PickerOption, PickerSessionPayload,
    QuestionOption, QuestionSessionPayload, ReviewSessionPayload, SessionPayload, SessionStatus,
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

fn question(id: &str) -> SessionPayload {
    SessionPayload::Question(QuestionSessionPayload {
        session_id: id.into(),
        status: SessionStatus::Pending,
        run_slug: None,
        title: Some("Pick a mockup".into()),
        prompt: "Which option feels right?".into(),
        context: Some("Generated three options".into()),
        options: vec![
            QuestionOption {
                id: "opt-a".into(),
                label: "Crimson".into(),
                image_url: Some("/mock/a.png".into()),
                image_url_light: None,
                description: Some("bold + warm".into()),
            },
            QuestionOption {
                id: "opt-b".into(),
                label: "Cobalt".into(),
                image_url: Some("/mock/b.png".into()),
                image_url_light: None,
                description: None,
            },
        ],
        multi_select: false,
        answer: None,
        image_urls: vec!["/ref/surface.png".into()],
    })
}

fn multi_question(id: &str) -> SessionPayload {
    let SessionPayload::Question(mut q) = question(id) else {
        unreachable!()
    };
    q.multi_select = true;
    SessionPayload::Question(q)
}

fn direction(id: &str) -> SessionPayload {
    SessionPayload::Direction(DirectionSessionPayload {
        session_id: id.into(),
        status: SessionStatus::Pending,
        title: Some("Design direction".into()),
        run_slug: Some("run-d".into()),
        prompt: "Choose a direction".into(),
        context: None,
        archetypes: vec![
            DirectionArchetype {
                id: "brutalist".into(),
                label: "Brutalist".into(),
                image_url: "/mock/brutalist.png".into(),
                image_url_light: None,
                description: "raw concrete".into(),
            },
            DirectionArchetype {
                id: "soft".into(),
                label: "Soft".into(),
                image_url: "/mock/soft.png".into(),
                image_url_light: None,
                description: "rounded + warm".into(),
            },
        ],
        chosen_archetype: None,
        annotations: None,
    })
}

fn direction_no_archetypes(id: &str) -> SessionPayload {
    SessionPayload::Direction(DirectionSessionPayload {
        session_id: id.into(),
        status: SessionStatus::Pending,
        prompt: "Open direction".into(),
        ..Default::default()
    })
}

fn picker(id: &str) -> SessionPayload {
    SessionPayload::Picker(PickerSessionPayload {
        session_id: id.into(),
        status: SessionStatus::Pending,
        run_slug: Some("run-p".into()),
        kind: PickerKind::Factory,
        title: "Pick a factory".into(),
        prompt: "Which one?".into(),
        options: vec![
            PickerOption {
                id: "software".into(),
                label: "Software".into(),
                description: None,
                secondary: None,
            },
            PickerOption {
                id: "design".into(),
                label: "Design".into(),
                description: None,
                secondary: None,
            },
        ],
        selection: None,
    })
}

fn review(id: &str) -> SessionPayload {
    SessionPayload::Review(ReviewSessionPayload {
        session_id: id.into(),
        status: SessionStatus::Pending,
        ..Default::default()
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
// GET /api/session/:id — visual payload reads
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn get_question_session_exposes_options_and_images() {
    let state = test_state();
    state.sessions.upsert(question("q"));
    let resp = send(build_router(state), get("/api/session/q")).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["session_type"], "question");
    assert_eq!(json["prompt"], "Which option feels right?");
    assert_eq!(json["multi_select"], false);
    assert_eq!(json["options"][0]["id"], "opt-a");
    assert_eq!(json["options"][0]["image_url"], "/mock/a.png");
    assert_eq!(json["options"][0]["description"], "bold + warm");
    assert_eq!(json["image_urls"][0], "/ref/surface.png");
}

#[tokio::test]
async fn get_direction_session_exposes_archetypes() {
    let state = test_state();
    state.sessions.upsert(direction("d"));
    let resp = send(build_router(state), get("/api/session/d")).await;
    let json = body_json(resp).await;
    assert_eq!(json["session_type"], "direction");
    assert_eq!(json["prompt"], "Choose a direction");
    assert_eq!(json["archetypes"][0]["id"], "brutalist");
    assert_eq!(json["archetypes"][0]["image_url"], "/mock/brutalist.png");
    assert_eq!(json["archetypes"][1]["id"], "soft");
    // No choice yet.
    assert!(json.get("chosen_archetype").is_none());
}

#[tokio::test]
async fn get_picker_session_exposes_options() {
    let state = test_state();
    state.sessions.upsert(picker("p"));
    let resp = send(build_router(state), get("/api/session/p")).await;
    let json = body_json(resp).await;
    assert_eq!(json["session_type"], "picker");
    assert_eq!(json["options"][0]["id"], "software");
    assert_eq!(json["options"][1]["id"], "design");
}

// ════════════════════════════════════════════════════════════════════════════
// POST /question/:id/answer
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn question_answer_records_selection_and_responds() {
    let state = test_state();
    state.sessions.upsert(question("q"));
    let resp = send(
        build_router(state.clone()),
        post_json("/question/q/answer", &json!({ "selected": ["opt-a"] })),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["ok"], true);
    assert_eq!(json["answer"]["selected"][0], "opt-a");

    // The session was mutated and flipped to answered.
    let SessionPayload::Question(q) = state.sessions.get("q").unwrap() else {
        panic!("expected question");
    };
    assert_eq!(q.status, SessionStatus::Answered);
    let answer = q.answer.expect("answer recorded");
    assert_eq!(answer.selected, vec!["opt-a".to_string()]);
}

#[tokio::test]
async fn question_answer_carries_free_text() {
    let state = test_state();
    state.sessions.upsert(question("q"));
    let resp = send(
        build_router(state.clone()),
        post_json(
            "/question/q/answer",
            &json!({ "selected": ["opt-b"], "text": "love the calm" }),
        ),
    )
    .await;
    let json = body_json(resp).await;
    assert_eq!(json["answer"]["text"], "love the calm");
    let SessionPayload::Question(q) = state.sessions.get("q").unwrap() else {
        panic!("expected question");
    };
    assert_eq!(q.answer.unwrap().text.as_deref(), Some("love the calm"));
}

#[tokio::test]
async fn question_answer_multi_select_accepts_many() {
    let state = test_state();
    state.sessions.upsert(multi_question("q"));
    let resp = send(
        build_router(state.clone()),
        post_json("/question/q/answer", &json!({ "selected": ["opt-a", "opt-b"] })),
    )
    .await;
    let json = body_json(resp).await;
    assert_eq!(json["answer"]["selected"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn question_answer_text_only_no_selection() {
    let state = test_state();
    state.sessions.upsert(question("q"));
    let resp = send(
        build_router(state),
        post_json("/question/q/answer", &json!({ "text": "none of these" })),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["answer"]["text"], "none of these");
    assert!(json["answer"].get("selected").is_none());
}

#[tokio::test]
async fn question_answer_empty_body_is_accepted() {
    // Both fields default; an empty object is a valid (empty) answer.
    let state = test_state();
    state.sessions.upsert(question("q"));
    let resp = send(
        build_router(state),
        post_json("/question/q/answer", &json!({})),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn question_answer_unknown_session_is_404() {
    let resp = send(
        build_router(test_state()),
        post_json("/question/ghost/answer", &json!({ "selected": ["x"] })),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn question_answer_on_non_question_session_is_409() {
    let state = test_state();
    state.sessions.upsert(review("r"));
    let resp = send(
        build_router(state),
        post_json("/question/r/answer", &json!({ "selected": ["x"] })),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let json = body_json(resp).await;
    assert_eq!(json["error"], "session is not a question session");
    assert_eq!(json["session_id"], "r");
}

#[tokio::test]
async fn question_answer_malformed_json_is_400() {
    let state = test_state();
    state.sessions.upsert(question("q"));
    let req = Request::builder()
        .method(Method::POST)
        .uri("/question/q/answer")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{ bad"))
        .unwrap();
    let resp = send(build_router(state), req).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn question_answer_get_method_is_405() {
    let state = test_state();
    state.sessions.upsert(question("q"));
    let resp = send(build_router(state), get("/question/q/answer")).await;
    assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn question_answer_last_write_wins() {
    let state = test_state();
    state.sessions.upsert(question("q"));
    let app = build_router(state.clone());
    let _ = send(
        app.clone(),
        post_json("/question/q/answer", &json!({ "selected": ["opt-a"] })),
    )
    .await;
    let _ = send(
        app,
        post_json("/question/q/answer", &json!({ "selected": ["opt-b"] })),
    )
    .await;
    let SessionPayload::Question(q) = state.sessions.get("q").unwrap() else {
        panic!("expected question");
    };
    assert_eq!(q.answer.unwrap().selected, vec!["opt-b".to_string()]);
}

// ════════════════════════════════════════════════════════════════════════════
// POST /direction/:id/select
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn direction_select_records_choice_and_responds() {
    let state = test_state();
    state.sessions.upsert(direction("d"));
    let resp = send(
        build_router(state.clone()),
        post_json("/direction/d/select", &json!({ "archetype": "brutalist" })),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["ok"], true);
    assert_eq!(json["archetype"], "brutalist");

    let SessionPayload::Direction(d) = state.sessions.get("d").unwrap() else {
        panic!("expected direction");
    };
    assert_eq!(d.status, SessionStatus::Decided);
    assert_eq!(d.chosen_archetype.as_deref(), Some("brutalist"));
}

#[tokio::test]
async fn direction_select_persists_annotations() {
    let state = test_state();
    state.sessions.upsert(direction("d"));
    let body = json!({
        "archetype": "soft",
        "annotations": {
            "pins": [{ "x": 0.5, "y": 0.25, "note": "more rounding here" }],
            "screenshot": "data:image/png;base64,AA",
            "comments": ["love it", "ship this one"]
        }
    });
    let resp = send(build_router(state.clone()), post_json("/direction/d/select", &body)).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let SessionPayload::Direction(d) = state.sessions.get("d").unwrap() else {
        panic!("expected direction");
    };
    assert_eq!(d.chosen_archetype.as_deref(), Some("soft"));
    let annotations = d.annotations.expect("annotations recorded");
    assert_eq!(annotations.pins.len(), 1);
    assert_eq!(annotations.pins[0].note, "more rounding here");
    assert_eq!(annotations.screenshot.as_deref(), Some("data:image/png;base64,AA"));
    assert_eq!(annotations.comments.len(), 2);
}

#[tokio::test]
async fn direction_select_unknown_archetype_is_422() {
    let state = test_state();
    state.sessions.upsert(direction("d"));
    let resp = send(
        build_router(state.clone()),
        post_json("/direction/d/select", &json!({ "archetype": "neon" })),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let json = body_json(resp).await;
    assert_eq!(json["error"], "unknown archetype id");
    assert_eq!(json["value"], "neon");
    // Session unchanged.
    let SessionPayload::Direction(d) = state.sessions.get("d").unwrap() else {
        panic!("expected direction");
    };
    assert_eq!(d.status, SessionStatus::Pending);
    assert!(d.chosen_archetype.is_none());
}

#[tokio::test]
async fn direction_select_unconstrained_when_no_archetypes() {
    // An empty archetype list means the decision is not validated against it.
    let state = test_state();
    state.sessions.upsert(direction_no_archetypes("d"));
    let resp = send(
        build_router(state.clone()),
        post_json("/direction/d/select", &json!({ "archetype": "whatever" })),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let SessionPayload::Direction(d) = state.sessions.get("d").unwrap() else {
        panic!("expected direction");
    };
    assert_eq!(d.chosen_archetype.as_deref(), Some("whatever"));
}

#[tokio::test]
async fn direction_select_unknown_session_is_404() {
    let resp = send(
        build_router(test_state()),
        post_json("/direction/ghost/select", &json!({ "archetype": "x" })),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn direction_select_on_non_direction_session_is_409() {
    let state = test_state();
    state.sessions.upsert(question("q"));
    let resp = send(
        build_router(state),
        post_json("/direction/q/select", &json!({ "archetype": "x" })),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let json = body_json(resp).await;
    assert_eq!(json["error"], "session is not a direction session");
}

#[tokio::test]
async fn direction_select_missing_archetype_is_422() {
    // archetype is a required field → the Json extractor rejects with 422.
    let state = test_state();
    state.sessions.upsert(direction("d"));
    let resp = send(
        build_router(state),
        post_json("/direction/d/select", &json!({ "annotations": {} })),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

// ════════════════════════════════════════════════════════════════════════════
// POST /picker/:id/select
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn picker_select_records_choice_and_responds() {
    let state = test_state();
    state.sessions.upsert(picker("p"));
    let resp = send(
        build_router(state.clone()),
        post_json("/picker/p/select", &json!({ "id": "design" })),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["ok"], true);
    assert_eq!(json["id"], "design");

    let SessionPayload::Picker(p) = state.sessions.get("p").unwrap() else {
        panic!("expected picker");
    };
    assert_eq!(p.status, SessionStatus::Decided);
    assert_eq!(p.selection.unwrap().id, "design");
}

#[tokio::test]
async fn picker_select_unknown_option_is_422() {
    let state = test_state();
    state.sessions.upsert(picker("p"));
    let resp = send(
        build_router(state.clone()),
        post_json("/picker/p/select", &json!({ "id": "marketing" })),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let json = body_json(resp).await;
    assert_eq!(json["error"], "unknown option id");
    assert_eq!(json["value"], "marketing");
    let SessionPayload::Picker(p) = state.sessions.get("p").unwrap() else {
        panic!("expected picker");
    };
    assert_eq!(p.status, SessionStatus::Pending);
}

#[tokio::test]
async fn picker_select_unknown_session_is_404() {
    let resp = send(
        build_router(test_state()),
        post_json("/picker/ghost/select", &json!({ "id": "x" })),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn picker_select_on_non_picker_session_is_409() {
    let state = test_state();
    state.sessions.upsert(direction("d"));
    let resp = send(
        build_router(state),
        post_json("/picker/d/select", &json!({ "id": "x" })),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let json = body_json(resp).await;
    assert_eq!(json["error"], "session is not a picker session");
}

#[tokio::test]
async fn picker_select_missing_id_is_422() {
    let state = test_state();
    state.sessions.upsert(picker("p"));
    let resp = send(
        build_router(state),
        post_json("/picker/p/select", &json!({})),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

// ════════════════════════════════════════════════════════════════════════════
// GET-then-POST-then-GET lifecycle through the routes
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn question_lifecycle_get_answer_get() {
    let state = test_state();
    state.sessions.upsert(question("q"));
    let app = build_router(state);

    let before = body_json(send(app.clone(), get("/api/session/q")).await).await;
    assert_eq!(before["status"], "pending");
    assert!(before.get("answer").is_none());

    let _ = send(
        app.clone(),
        post_json("/question/q/answer", &json!({ "selected": ["opt-a"], "text": "yep" })),
    )
    .await;

    let after = body_json(send(app, get("/api/session/q")).await).await;
    assert_eq!(after["status"], "answered");
    assert_eq!(after["answer"]["selected"][0], "opt-a");
    assert_eq!(after["answer"]["text"], "yep");
}

#[tokio::test]
async fn direction_lifecycle_get_select_get() {
    let state = test_state();
    state.sessions.upsert(direction("d"));
    let app = build_router(state);

    let _ = send(
        app.clone(),
        post_json(
            "/direction/d/select",
            &json!({ "archetype": "soft", "annotations": { "comments": ["nice"] } }),
        ),
    )
    .await;

    let after = body_json(send(app, get("/api/session/d")).await).await;
    assert_eq!(after["status"], "decided");
    assert_eq!(after["chosen_archetype"], "soft");
    assert_eq!(after["annotations"]["comments"][0], "nice");
}

// ════════════════════════════════════════════════════════════════════════════
// WebSocket push driven by the mutating POST routes
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

/// A bound server + an HTTP client over a raw socket would be heavy; instead we
/// open the WS, then drive the mutation through the same registry the router
/// shares (the POST handler path is covered by the oneshot tests above).
#[tokio::test]
async fn ws_pushes_question_answer_update() {
    let state = test_state();
    state.sessions.upsert(question("ws-q"));
    let registry = state.sessions.clone();
    let (bound, handle) = spawn(state).await;

    let (mut socket, _) =
        tokio_tungstenite::connect_async(format!("ws://{bound}/ws/session/ws-q"))
            .await
            .unwrap();
    let snap = next_json(&mut socket).await;
    assert_eq!(snap["status"], "pending");

    // Mutate exactly as the handler would: record the answer + flip status.
    let SessionPayload::Question(mut q) = registry.get("ws-q").unwrap() else {
        unreachable!()
    };
    q.answer = Some(darkrun_api::QuestionAnswer {
        selected: vec!["opt-a".into()],
        text: None,
    });
    q.status = SessionStatus::Answered;
    registry.upsert(SessionPayload::Question(q));

    let upd = next_json(&mut socket).await;
    assert_eq!(upd["status"], "answered");
    assert_eq!(upd["answer"]["selected"][0], "opt-a");

    socket.send(WsMessage::Close(None)).await.ok();
    handle.abort();
}

#[tokio::test]
async fn ws_pushes_direction_select_update() {
    let state = test_state();
    state.sessions.upsert(direction("ws-d"));
    let registry = state.sessions.clone();
    let (bound, handle) = spawn(state).await;

    let (mut socket, _) =
        tokio_tungstenite::connect_async(format!("ws://{bound}/ws/session/ws-d"))
            .await
            .unwrap();
    let _snap = next_json(&mut socket).await;

    let SessionPayload::Direction(mut d) = registry.get("ws-d").unwrap() else {
        unreachable!()
    };
    d.chosen_archetype = Some("brutalist".into());
    d.status = SessionStatus::Decided;
    registry.upsert(SessionPayload::Direction(d));

    let upd = next_json(&mut socket).await;
    assert_eq!(upd["status"], "decided");
    assert_eq!(upd["chosen_archetype"], "brutalist");

    socket.send(WsMessage::Close(None)).await.ok();
    handle.abort();
}

#[tokio::test]
async fn ws_pushes_picker_select_update() {
    let state = test_state();
    state.sessions.upsert(picker("ws-p"));
    let registry = state.sessions.clone();
    let (bound, handle) = spawn(state).await;

    let (mut socket, _) =
        tokio_tungstenite::connect_async(format!("ws://{bound}/ws/session/ws-p"))
            .await
            .unwrap();
    let _snap = next_json(&mut socket).await;

    let SessionPayload::Picker(mut p) = registry.get("ws-p").unwrap() else {
        unreachable!()
    };
    p.selection = Some(darkrun_api::PickerSelection { id: "design".into() });
    p.status = SessionStatus::Decided;
    registry.upsert(SessionPayload::Picker(p));

    let upd = next_json(&mut socket).await;
    assert_eq!(upd["status"], "decided");
    assert_eq!(upd["selection"]["id"], "design");

    socket.send(WsMessage::Close(None)).await.ok();
    handle.abort();
}

// ── On-demand session materialization ────────────────────────────────────────

#[tokio::test]
async fn a_session_miss_materializes_on_demand() {
    // The engine installs a materializer that builds a session when the id
    // names something real (a run slug). A GET for a not-yet-pushed session
    // builds it instead of 404ing — clicking a run in the desktop sidebar
    // works before the engine's first tick.
    let state = test_state();
    let sessions = state.sessions.clone();
    let state = state.with_session_materializer(move |id| {
        if id == "lazy-run" {
            sessions.upsert(review("lazy-run"));
            true
        } else {
            false
        }
    });

    let resp = send(build_router(state.clone()), get("/api/session/lazy-run")).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["session_id"], "lazy-run");

    // Heartbeat materializes too; an id the materializer rejects still 404s.
    let hb = Request::head("/api/session/lazy-run/heartbeat").body(Body::empty()).unwrap();
    assert_eq!(send(build_router(state.clone()), hb).await.status(), StatusCode::OK);
    let resp = send(build_router(state), get("/api/session/nope")).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
