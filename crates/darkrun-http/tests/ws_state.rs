//! Comprehensive integration coverage for darkrun-http's WebSocket upgrade
//! path and the shared session-state store (`SessionRegistry`).
//!
//! Two surfaces are exercised here:
//!
//!  * The `/ws/session/:id` upgrade — snapshot push on connect, live updates
//!    over the per-session broadcast channel, concurrent observers, and the
//!    unknown-session close path. Driven over a real loopback bind via
//!    `tokio-tungstenite`, mirroring the existing smoke checks in `server.rs`.
//!  * The `SessionRegistry` itself — register / observe / mutate, the WS-slot
//!    reservation cap, broadcast fan-out semantics, and determinism of the
//!    serialized frames. Driven straight off the public API.
//!
//! All filesystem state lives in a tempdir; no global state is shared between
//! tests.

use std::net::SocketAddr;
use std::time::Duration;

use darkrun_api::{
    ApproveAction, ApproveActionKind, DirectionArchetype, DirectionSessionPayload, GateType,
    PickerKind, PickerOption, PickerSessionPayload, QuestionOption, QuestionSessionPayload,
    ReviewSessionPayload, SessionPayload, SessionStatus, ViewMode, ViewScope, ViewSessionPayload,
    ViewStatus,
};
use darkrun_core::StateStore;
use darkrun_http::{build_router, AppState, Limits, SessionRegistry};
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message as WsMessage;

// ── Fixtures ────────────────────────────────────────────────────────────────

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

fn review_with_status(session_id: &str, status: SessionStatus) -> SessionPayload {
    SessionPayload::Review(ReviewSessionPayload {
        session_id: session_id.into(),
        status,
        ..Default::default()
    })
}

fn question(session_id: &str) -> SessionPayload {
    SessionPayload::Question(QuestionSessionPayload {
        session_id: session_id.into(),
        status: SessionStatus::Pending,
        run_slug: None,
        title: Some("Pick a path".into()),
        prompt: "Which station?".into(),
        context: Some("some context".into()),
        options: vec![
            QuestionOption {
                id: "frame".into(),
                label: "Frame".into(),
                image_url: Some("/mock/frame.png".into()),
                image_url_light: None,
                description: None,
            },
            QuestionOption {
                id: "build".into(),
                label: "Build".into(),
                image_url: None,
                image_url_light: None,
                description: None,
            },
        ],
        multi_select: false,
        ..Default::default()
    })
}

fn direction(session_id: &str) -> SessionPayload {
    SessionPayload::Direction(DirectionSessionPayload {
        session_id: session_id.into(),
        status: SessionStatus::Pending,
        title: Some("Choose a direction".into()),
        run_slug: Some("dir-run".into()),
        prompt: "Pick a design direction".into(),
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
        run_slug: Some("pick-run".into()),
        kind: PickerKind::Station,
        title: "Pick a station".into(),
        prompt: "which one?".into(),
        options: vec![PickerOption {
            id: "frame".into(),
            label: "Frame".into(),
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
        run_slug: "view-run".into(),
        scope: ViewScope::Run,
        artifacts: vec![],
        factory: None,
        station: None,
        artifact: None,
        mode: ViewMode::Viewer,
        boot_port: None,
        boot_command: None,
    })
}

fn empty_registry() -> SessionRegistry {
    SessionRegistry::new()
}

fn app_state() -> (AppState, tempfile::TempDir) {
    let tmp = tempfile::tempdir().expect("tmp");
    let store = StateStore::new(tmp.path());
    (AppState::new(store, Limits::default()), tmp)
}

/// Bind a router on an ephemeral loopback port and spawn it; returns the bound
/// address and the server task handle. The state's registry is cloned out for
/// the caller so it can register/mutate sessions while the server runs.
async fn spawn_server(state: AppState) -> (SocketAddr, tokio::task::JoinHandle<()>) {
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

/// Read the next text frame, skipping non-text frames. Panics on close/error.
async fn next_text<S>(socket: &mut S) -> String
where
    S: futures_util::Stream<
            Item = Result<
                tokio_tungstenite::tungstenite::Message,
                tokio_tungstenite::tungstenite::Error,
            >,
        > + Unpin,
{
    loop {
        match socket.next().await {
            Some(Ok(WsMessage::Text(t))) => return t.to_string(),
            Some(Ok(WsMessage::Ping(_))) | Some(Ok(WsMessage::Pong(_))) => continue,
            other => panic!("expected a text frame, got {other:?}"),
        }
    }
}

/// Await the next text frame within a timeout, parsing it as JSON.
async fn next_json<S>(socket: &mut S) -> serde_json::Value
where
    S: futures_util::Stream<
            Item = Result<
                tokio_tungstenite::tungstenite::Message,
                tokio_tungstenite::tungstenite::Error,
            >,
        > + Unpin,
{
    let text = tokio::time::timeout(Duration::from_secs(5), next_text(socket))
        .await
        .expect("timed out waiting for a frame");
    serde_json::from_str(&text).expect("frame is valid json")
}

// ════════════════════════════════════════════════════════════════════════════
// SessionRegistry — register / observe / mutate (no server, pure store)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn new_registry_is_empty() {
    let reg = empty_registry();
    assert!(!reg.contains("anything"));
    assert!(reg.get("anything").is_none());
}

#[test]
fn default_registry_is_empty() {
    let reg = SessionRegistry::default();
    assert!(reg.get("x").is_none());
    assert!(!reg.contains("x"));
}

#[test]
fn upsert_then_get_returns_payload() {
    let reg = empty_registry();
    reg.upsert(review("s1"));
    let got = reg.get("s1").expect("registered");
    assert_eq!(got.session_id(), "s1");
    assert_eq!(got.session_type(), "review");
}

#[test]
fn upsert_then_contains_is_true() {
    let reg = empty_registry();
    reg.upsert(review("s1"));
    assert!(reg.contains("s1"));
}

#[test]
fn contains_is_false_for_unregistered() {
    let reg = empty_registry();
    reg.upsert(review("s1"));
    assert!(!reg.contains("s2"));
}

#[test]
fn get_unknown_is_none() {
    let reg = empty_registry();
    reg.upsert(review("s1"));
    assert!(reg.get("nope").is_none());
}

#[test]
fn upsert_replaces_existing_payload() {
    let reg = empty_registry();
    reg.upsert(review_with_status("s", SessionStatus::Pending));
    reg.upsert(review_with_status("s", SessionStatus::Approved));
    let SessionPayload::Review(r) = reg.get("s").unwrap() else {
        panic!("expected review");
    };
    assert_eq!(r.status, SessionStatus::Approved);
}

#[test]
fn upsert_replace_keeps_single_entry() {
    let reg = empty_registry();
    for _ in 0..10 {
        reg.upsert(review("dup"));
    }
    // Replacing the same id repeatedly leaves exactly one observable session.
    assert!(reg.contains("dup"));
    assert!(reg.get("dup").is_some());
}

#[test]
fn upsert_can_change_variant_for_same_id() {
    let reg = empty_registry();
    reg.upsert(review("s"));
    assert_eq!(reg.get("s").unwrap().session_type(), "review");
    reg.upsert(question("s"));
    assert_eq!(reg.get("s").unwrap().session_type(), "question");
}

#[test]
fn remove_returns_payload_and_clears() {
    let reg = empty_registry();
    reg.upsert(review("s1"));
    let removed = reg.remove("s1").expect("was present");
    assert_eq!(removed.session_id(), "s1");
    assert!(!reg.contains("s1"));
    assert!(reg.get("s1").is_none());
}

#[test]
fn remove_unknown_is_none() {
    let reg = empty_registry();
    assert!(reg.remove("ghost").is_none());
}

#[test]
fn remove_is_idempotent() {
    let reg = empty_registry();
    reg.upsert(review("s1"));
    assert!(reg.remove("s1").is_some());
    assert!(reg.remove("s1").is_none());
    assert!(reg.remove("s1").is_none());
}

#[test]
fn reupsert_after_remove_restores_session() {
    let reg = empty_registry();
    reg.upsert(review("s1"));
    reg.remove("s1");
    assert!(!reg.contains("s1"));
    reg.upsert(review("s1"));
    assert!(reg.contains("s1"));
}

#[test]
fn many_distinct_sessions_coexist() {
    let reg = empty_registry();
    for i in 0..200 {
        reg.upsert(review(&format!("s-{i}")));
    }
    for i in 0..200 {
        assert!(reg.contains(&format!("s-{i}")), "session s-{i} missing");
    }
    assert!(!reg.contains("s-200"));
}

#[test]
fn removing_one_session_leaves_others() {
    let reg = empty_registry();
    reg.upsert(review("a"));
    reg.upsert(review("b"));
    reg.upsert(review("c"));
    reg.remove("b");
    assert!(reg.contains("a"));
    assert!(!reg.contains("b"));
    assert!(reg.contains("c"));
}

#[test]
fn clones_share_backing_map() {
    let reg = empty_registry();
    let clone = reg.clone();
    reg.upsert(review("shared"));
    // The clone observes the mutation made through the original.
    assert!(clone.contains("shared"));
    assert_eq!(clone.get("shared").unwrap().session_id(), "shared");
}

#[test]
fn clone_remove_visible_through_original() {
    let reg = empty_registry();
    let clone = reg.clone();
    reg.upsert(review("x"));
    clone.remove("x");
    assert!(!reg.contains("x"));
}

#[test]
fn empty_string_id_is_a_valid_key() {
    let reg = empty_registry();
    reg.upsert(review(""));
    assert!(reg.contains(""));
    assert_eq!(reg.get("").unwrap().session_id(), "");
}

#[test]
fn unicode_and_special_ids_roundtrip() {
    let reg = empty_registry();
    for id in ["héllo", "日本語", "with space", "slash/in/id", "emoji-🚀", "tab\tchar"] {
        reg.upsert(review(id));
        assert!(reg.contains(id), "missing {id:?}");
        assert_eq!(reg.get(id).unwrap().session_id(), id);
    }
}

#[test]
fn ids_are_case_sensitive() {
    let reg = empty_registry();
    reg.upsert(review("Session"));
    assert!(reg.contains("Session"));
    assert!(!reg.contains("session"));
    assert!(!reg.contains("SESSION"));
}

#[test]
fn whitespace_only_id_is_distinct_from_empty() {
    let reg = empty_registry();
    reg.upsert(review(" "));
    assert!(reg.contains(" "));
    assert!(!reg.contains(""));
}

#[test]
fn get_returns_independent_clone() {
    let reg = empty_registry();
    reg.upsert(review_with_status("s", SessionStatus::Pending));
    let mut first = reg.get("s").unwrap();
    if let SessionPayload::Review(ref mut r) = first {
        r.status = SessionStatus::Approved;
    }
    // Mutating the returned clone does not write back into the registry.
    let SessionPayload::Review(stored) = reg.get("s").unwrap() else {
        panic!("review");
    };
    assert_eq!(stored.status, SessionStatus::Pending);
}

// ── Variant coverage through the registry ───────────────────────────────────

#[test]
fn registry_holds_review_variant() {
    let reg = empty_registry();
    reg.upsert(review("r"));
    assert_eq!(reg.get("r").unwrap().session_type(), "review");
}

#[test]
fn registry_holds_question_variant() {
    let reg = empty_registry();
    reg.upsert(question("q"));
    assert_eq!(reg.get("q").unwrap().session_type(), "question");
}

#[test]
fn registry_holds_direction_variant() {
    let reg = empty_registry();
    reg.upsert(direction("d"));
    assert_eq!(reg.get("d").unwrap().session_type(), "direction");
}

#[test]
fn registry_holds_picker_variant() {
    let reg = empty_registry();
    reg.upsert(picker("p"));
    assert_eq!(reg.get("p").unwrap().session_type(), "picker");
}

#[test]
fn registry_holds_view_variant() {
    let reg = empty_registry();
    reg.upsert(view("v"));
    assert_eq!(reg.get("v").unwrap().session_type(), "view");
}

#[test]
fn all_five_variants_coexist() {
    let reg = empty_registry();
    reg.upsert(review("r"));
    reg.upsert(question("q"));
    reg.upsert(direction("d"));
    reg.upsert(picker("p"));
    reg.upsert(view("v"));
    assert_eq!(reg.get("r").unwrap().session_type(), "review");
    assert_eq!(reg.get("q").unwrap().session_type(), "question");
    assert_eq!(reg.get("d").unwrap().session_type(), "direction");
    assert_eq!(reg.get("p").unwrap().session_type(), "picker");
    assert_eq!(reg.get("v").unwrap().session_type(), "view");
}

// ════════════════════════════════════════════════════════════════════════════
// subscribe() — observe semantics
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn subscribe_unknown_is_none() {
    let reg = empty_registry();
    assert!(reg.subscribe("nope").is_none());
}

#[test]
fn subscribe_known_is_some() {
    let reg = empty_registry();
    reg.upsert(review("s"));
    assert!(reg.subscribe("s").is_some());
}

#[test]
fn subscribe_after_remove_is_none() {
    let reg = empty_registry();
    reg.upsert(review("s"));
    reg.remove("s");
    assert!(reg.subscribe("s").is_none());
}

#[tokio::test]
async fn upsert_broadcasts_frame_to_subscriber() {
    let reg = empty_registry();
    reg.upsert(review("s"));
    let mut rx = reg.subscribe("s").unwrap();
    reg.upsert(review_with_status("s", SessionStatus::Approved));
    let frame = rx.recv().await.expect("a frame");
    let json: serde_json::Value = serde_json::from_str(&frame).unwrap();
    assert_eq!(json["session_id"], "s");
    assert_eq!(json["status"], "approved");
}

#[tokio::test]
async fn subscriber_misses_the_upsert_that_created_the_entry() {
    // The broadcast happens before subscribe could exist, so subscribing only
    // sees *subsequent* upserts, not the creating one.
    let reg = empty_registry();
    reg.upsert(review("s"));
    let mut rx = reg.subscribe("s").unwrap();
    // No update yet → the channel has nothing buffered.
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn multiple_upserts_each_broadcast() {
    let reg = empty_registry();
    reg.upsert(review("s"));
    let mut rx = reg.subscribe("s").unwrap();
    reg.upsert(review_with_status("s", SessionStatus::Decided));
    reg.upsert(review_with_status("s", SessionStatus::Approved));
    let first: serde_json::Value = serde_json::from_str(&rx.recv().await.unwrap()).unwrap();
    let second: serde_json::Value = serde_json::from_str(&rx.recv().await.unwrap()).unwrap();
    assert_eq!(first["status"], "decided");
    assert_eq!(second["status"], "approved");
}

#[tokio::test]
async fn two_subscribers_both_receive_update() {
    let reg = empty_registry();
    reg.upsert(review("s"));
    let mut rx1 = reg.subscribe("s").unwrap();
    let mut rx2 = reg.subscribe("s").unwrap();
    reg.upsert(review_with_status("s", SessionStatus::Approved));
    let a: serde_json::Value = serde_json::from_str(&rx1.recv().await.unwrap()).unwrap();
    let b: serde_json::Value = serde_json::from_str(&rx2.recv().await.unwrap()).unwrap();
    assert_eq!(a["status"], "approved");
    assert_eq!(b["status"], "approved");
}

#[tokio::test]
async fn many_subscribers_all_receive_update() {
    let reg = empty_registry();
    reg.upsert(review("s"));
    let mut subs: Vec<_> = (0..32).map(|_| reg.subscribe("s").unwrap()).collect();
    reg.upsert(review_with_status("s", SessionStatus::Approved));
    for rx in subs.iter_mut() {
        let v: serde_json::Value = serde_json::from_str(&rx.recv().await.unwrap()).unwrap();
        assert_eq!(v["status"], "approved");
    }
}

#[tokio::test]
async fn subscribers_only_see_their_session() {
    let reg = empty_registry();
    reg.upsert(review("a"));
    reg.upsert(review("b"));
    let mut rx_a = reg.subscribe("a").unwrap();
    // Mutating "b" must not push to a's channel.
    reg.upsert(review_with_status("b", SessionStatus::Approved));
    assert!(rx_a.try_recv().is_err());
    // But mutating "a" does.
    reg.upsert(review_with_status("a", SessionStatus::Decided));
    assert!(rx_a.recv().await.is_ok());
}

#[tokio::test]
async fn remove_closes_subscriber_channel() {
    let reg = empty_registry();
    reg.upsert(review("s"));
    let mut rx = reg.subscribe("s").unwrap();
    reg.remove("s");
    // The sender was dropped with the entry → recv resolves to Closed.
    match rx.recv().await {
        Err(tokio::sync::broadcast::error::RecvError::Closed) => {}
        other => panic!("expected Closed, got {other:?}"),
    }
}

#[tokio::test]
async fn reupsert_after_remove_creates_fresh_channel() {
    let reg = empty_registry();
    reg.upsert(review("s"));
    let mut old_rx = reg.subscribe("s").unwrap();
    reg.remove("s");
    reg.upsert(review("s"));
    // The old receiver is closed; a new subscribe yields a live channel.
    assert!(matches!(
        old_rx.recv().await,
        Err(tokio::sync::broadcast::error::RecvError::Closed)
    ));
    let mut new_rx = reg.subscribe("s").unwrap();
    reg.upsert(review_with_status("s", SessionStatus::Approved));
    assert!(new_rx.recv().await.is_ok());
}

#[tokio::test]
async fn subscribe_via_clone_observes_original_upserts() {
    let reg = empty_registry();
    reg.upsert(review("s"));
    let clone = reg.clone();
    let mut rx = clone.subscribe("s").unwrap();
    reg.upsert(review_with_status("s", SessionStatus::Approved));
    let v: serde_json::Value = serde_json::from_str(&rx.recv().await.unwrap()).unwrap();
    assert_eq!(v["status"], "approved");
}

#[tokio::test]
async fn lagging_subscriber_reports_lagged_then_recovers() {
    // The broadcast channel capacity is bounded (64). Overflowing it before
    // draining yields a Lagged error, after which recv continues.
    let reg = empty_registry();
    reg.upsert(review("s"));
    let mut rx = reg.subscribe("s").unwrap();
    for _ in 0..200 {
        reg.upsert(review_with_status("s", SessionStatus::Approved));
    }
    use tokio::sync::broadcast::error::TryRecvError;
    let mut saw_lag = false;
    let mut delivered = 0;
    loop {
        match rx.try_recv() {
            Ok(_) => delivered += 1,
            Err(TryRecvError::Lagged(_)) => saw_lag = true,
            Err(TryRecvError::Closed) | Err(TryRecvError::Empty) => break,
        }
    }
    assert!(saw_lag, "expected a Lagged signal after overflowing capacity");
    assert!(delivered > 0, "expected some frames after the lag");
}

// ── Broadcast payload fidelity ──────────────────────────────────────────────

#[tokio::test]
async fn broadcast_frame_matches_get_serialization() {
    let reg = empty_registry();
    reg.upsert(review("s"));
    let mut rx = reg.subscribe("s").unwrap();
    let pushed = review_with_status("s", SessionStatus::Approved);
    reg.upsert(pushed.clone());
    let frame = rx.recv().await.unwrap();
    let from_get = serde_json::to_string(&reg.get("s").unwrap()).unwrap();
    assert_eq!(frame, from_get);
}

#[tokio::test]
async fn broadcast_frame_carries_full_variant_payload() {
    let reg = empty_registry();
    reg.upsert(question("q"));
    let mut rx = reg.subscribe("q").unwrap();
    reg.upsert(question("q"));
    let json: serde_json::Value = serde_json::from_str(&rx.recv().await.unwrap()).unwrap();
    assert_eq!(json["session_type"], "question");
    assert_eq!(json["title"], "Pick a path");
    assert_eq!(json["prompt"], "Which station?");
    assert_eq!(json["options"][0]["id"], "frame");
}

// ════════════════════════════════════════════════════════════════════════════
// WS-slot reservation cap
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn ws_slot_acquire_under_cap() {
    let reg = empty_registry();
    let slot = reg.try_acquire_ws_slot(2);
    assert!(slot.is_some());
}

#[test]
fn ws_slot_cap_of_zero_rejects() {
    let reg = empty_registry();
    assert!(reg.try_acquire_ws_slot(0).is_none());
}

#[test]
fn ws_slot_cap_enforced() {
    let reg = empty_registry();
    let _a = reg.try_acquire_ws_slot(2).expect("first");
    let _b = reg.try_acquire_ws_slot(2).expect("second");
    assert!(reg.try_acquire_ws_slot(2).is_none(), "third over cap");
}

#[test]
fn ws_slot_releases_on_drop() {
    let reg = empty_registry();
    {
        let _a = reg.try_acquire_ws_slot(1).expect("first");
        assert!(reg.try_acquire_ws_slot(1).is_none());
    }
    // After the guard drops the slot is reclaimable.
    assert!(reg.try_acquire_ws_slot(1).is_some());
}

#[test]
fn ws_slot_full_release_cycle() {
    let reg = empty_registry();
    let slots: Vec<_> = (0..5).map(|_| reg.try_acquire_ws_slot(5).unwrap()).collect();
    assert!(reg.try_acquire_ws_slot(5).is_none());
    drop(slots);
    // All five slots returned → can re-acquire the full set.
    let again: Vec<_> = (0..5).map(|_| reg.try_acquire_ws_slot(5).unwrap()).collect();
    assert_eq!(again.len(), 5);
}

#[test]
fn ws_slot_partial_release_frees_one() {
    let reg = empty_registry();
    let a = reg.try_acquire_ws_slot(2).unwrap();
    let _b = reg.try_acquire_ws_slot(2).unwrap();
    assert!(reg.try_acquire_ws_slot(2).is_none());
    drop(a);
    assert!(reg.try_acquire_ws_slot(2).is_some());
}

#[test]
fn ws_slot_cap_shared_across_clones() {
    let reg = empty_registry();
    let clone = reg.clone();
    let _a = reg.try_acquire_ws_slot(1).expect("one");
    // The clone shares the same counter, so it sees the cap as reached.
    assert!(clone.try_acquire_ws_slot(1).is_none());
}

#[test]
fn ws_slot_release_visible_to_clone() {
    let reg = empty_registry();
    let clone = reg.clone();
    {
        let _a = reg.try_acquire_ws_slot(1).expect("one");
        assert!(clone.try_acquire_ws_slot(1).is_none());
    }
    assert!(clone.try_acquire_ws_slot(1).is_some());
}

#[test]
fn ws_slot_count_independent_of_session_count() {
    let reg = empty_registry();
    // Slots and registered sessions are tracked separately.
    reg.upsert(review("a"));
    reg.upsert(review("b"));
    let _s = reg.try_acquire_ws_slot(1).unwrap();
    assert!(reg.try_acquire_ws_slot(1).is_none());
    // Sessions still present; the cap only counts WS slots.
    assert!(reg.contains("a"));
    assert!(reg.contains("b"));
}

#[test]
fn ws_slot_large_cap_allows_many() {
    let reg = empty_registry();
    let slots: Vec<_> = (0..100).map(|_| reg.try_acquire_ws_slot(128).unwrap()).collect();
    assert_eq!(slots.len(), 100);
    // Still 28 left under a cap of 128.
    assert!(reg.try_acquire_ws_slot(128).is_some());
}

#[tokio::test]
async fn ws_slot_acquire_is_thread_safe() {
    use std::sync::Arc;
    let reg = Arc::new(empty_registry());
    let cap = 50usize;
    let mut handles = Vec::new();
    for _ in 0..200 {
        let reg = Arc::clone(&reg);
        handles.push(tokio::task::spawn_blocking(move || {
            // Hold each acquired slot briefly so concurrency actually competes.
            reg.try_acquire_ws_slot(cap).inspect(|_slot| {
                std::thread::yield_now();
            })
        }));
    }
    let mut held = Vec::new();
    for h in handles {
        if let Some(slot) = h.await.unwrap() {
            held.push(slot);
        }
    }
    // The cap must never be exceeded under contention.
    assert!(held.len() <= cap, "acquired {} > cap {cap}", held.len());
    assert!(!held.is_empty(), "expected at least some acquisitions");
}

// ════════════════════════════════════════════════════════════════════════════
// /ws/session/:id over a real loopback bind
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn ws_pushes_initial_snapshot() {
    let (state, _tmp) = app_state();
    state.sessions.upsert(review("ws-snap"));
    let (bound, handle) = spawn_server(state).await;

    let (mut socket, _resp) =
        tokio_tungstenite::connect_async(format!("ws://{bound}/ws/session/ws-snap"))
            .await
            .unwrap();
    let json = next_json(&mut socket).await;
    assert_eq!(json["session_id"], "ws-snap");
    assert_eq!(json["session_type"], "review");
    assert_eq!(json["gate_type"], "ask");

    socket.send(WsMessage::Close(None)).await.ok();
    handle.abort();
}

#[tokio::test]
async fn ws_snapshot_for_each_variant() {
    for (id, make, ty) in [
        ("v-review", review as fn(&str) -> SessionPayload, "review"),
        ("v-question", question, "question"),
        ("v-direction", direction, "direction"),
        ("v-picker", picker, "picker"),
        ("v-view", view, "view"),
    ] {
        let (state, _tmp) = app_state();
        state.sessions.upsert(make(id));
        let (bound, handle) = spawn_server(state).await;
        let (mut socket, _r) =
            tokio_tungstenite::connect_async(format!("ws://{bound}/ws/session/{id}"))
                .await
                .unwrap();
        let json = next_json(&mut socket).await;
        assert_eq!(json["session_id"], id);
        assert_eq!(json["session_type"], ty, "variant {ty}");
        socket.send(WsMessage::Close(None)).await.ok();
        handle.abort();
    }
}

#[tokio::test]
async fn ws_pushes_update_after_snapshot() {
    let (state, _tmp) = app_state();
    state.sessions.upsert(review("ws-upd"));
    let registry = state.sessions.clone();
    let (bound, handle) = spawn_server(state).await;

    let (mut socket, _resp) =
        tokio_tungstenite::connect_async(format!("ws://{bound}/ws/session/ws-upd"))
            .await
            .unwrap();
    // Snapshot first.
    let snap = next_json(&mut socket).await;
    assert_eq!(snap["status"], "pending");

    // Mutate; the update arrives as the next frame.
    registry.upsert(review_with_status("ws-upd", SessionStatus::Approved));
    let upd = next_json(&mut socket).await;
    assert_eq!(upd["session_id"], "ws-upd");
    assert_eq!(upd["status"], "approved");

    socket.send(WsMessage::Close(None)).await.ok();
    handle.abort();
}

#[tokio::test]
async fn ws_streams_several_sequential_updates() {
    let (state, _tmp) = app_state();
    state.sessions.upsert(review("ws-seq"));
    let registry = state.sessions.clone();
    let (bound, handle) = spawn_server(state).await;

    let (mut socket, _resp) =
        tokio_tungstenite::connect_async(format!("ws://{bound}/ws/session/ws-seq"))
            .await
            .unwrap();
    let _snap = next_json(&mut socket).await;

    for status in [
        SessionStatus::Decided,
        SessionStatus::ChangesRequested,
        SessionStatus::Approved,
    ] {
        registry.upsert(review_with_status("ws-seq", status));
        let frame = next_json(&mut socket).await;
        let expect = match status {
            SessionStatus::Decided => "decided",
            SessionStatus::ChangesRequested => "changes_requested",
            SessionStatus::Approved => "approved",
            _ => unreachable!(),
        };
        assert_eq!(frame["status"], expect);
    }

    socket.send(WsMessage::Close(None)).await.ok();
    handle.abort();
}

#[tokio::test]
async fn ws_unknown_session_closes() {
    let (state, _tmp) = app_state();
    let (bound, handle) = spawn_server(state).await;

    let (mut socket, _resp) =
        tokio_tungstenite::connect_async(format!("ws://{bound}/ws/session/ghost"))
            .await
            .unwrap();
    let mut saw_close = false;
    while let Some(msg) = socket.next().await {
        match msg {
            Ok(WsMessage::Close(_)) => {
                saw_close = true;
                break;
            }
            Ok(_) => continue,
            Err(_) => break,
        }
    }
    assert!(saw_close, "expected a close frame for an unknown session");
    handle.abort();
}

#[tokio::test]
async fn ws_unknown_session_does_not_push_snapshot() {
    let (state, _tmp) = app_state();
    let (bound, handle) = spawn_server(state).await;
    let (mut socket, _resp) =
        tokio_tungstenite::connect_async(format!("ws://{bound}/ws/session/ghost"))
            .await
            .unwrap();
    // The first non-close message should never be a text snapshot frame.
    match tokio::time::timeout(Duration::from_secs(5), socket.next()).await {
        Ok(Some(Ok(WsMessage::Close(_)))) => {}
        Ok(Some(Ok(WsMessage::Text(t)))) => panic!("unexpected snapshot for unknown session: {t}"),
        Ok(other) => panic!("expected a close frame, got {other:?}"),
        Err(_) => panic!("timed out; expected a prompt close"),
    }
    handle.abort();
}

#[tokio::test]
async fn ws_session_removed_after_connect_closes_stream() {
    let (state, _tmp) = app_state();
    state.sessions.upsert(review("ws-rm"));
    let registry = state.sessions.clone();
    let (bound, handle) = spawn_server(state).await;

    let (mut socket, _resp) =
        tokio_tungstenite::connect_async(format!("ws://{bound}/ws/session/ws-rm"))
            .await
            .unwrap();
    let _snap = next_json(&mut socket).await;

    // Removing the session drops the broadcast sender → the server loop ends
    // and the socket is closed from the server side.
    registry.remove("ws-rm");

    let mut closed = false;
    while let Some(msg) = tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .expect("not timed out")
    {
        match msg {
            Ok(WsMessage::Close(_)) => {
                closed = true;
                break;
            }
            Ok(_) => continue,
            Err(_) => {
                closed = true;
                break;
            }
        }
    }
    assert!(closed, "expected the socket to close after session removal");
    handle.abort();
}

#[tokio::test]
async fn ws_client_text_frame_is_ignored() {
    // The WS is push-only: inbound client frames are drained, not acted on.
    let (state, _tmp) = app_state();
    state.sessions.upsert(review("ws-in"));
    let registry = state.sessions.clone();
    let (bound, handle) = spawn_server(state).await;

    let (mut socket, _resp) =
        tokio_tungstenite::connect_async(format!("ws://{bound}/ws/session/ws-in"))
            .await
            .unwrap();
    let _snap = next_json(&mut socket).await;

    // Send junk; the server must keep streaming and not crash.
    socket
        .send(WsMessage::Text("please mutate me".into()))
        .await
        .unwrap();

    registry.upsert(review_with_status("ws-in", SessionStatus::Approved));
    let upd = next_json(&mut socket).await;
    assert_eq!(upd["status"], "approved");

    socket.send(WsMessage::Close(None)).await.ok();
    handle.abort();
}

#[tokio::test]
async fn ws_client_binary_frame_is_ignored() {
    let (state, _tmp) = app_state();
    state.sessions.upsert(review("ws-bin"));
    let registry = state.sessions.clone();
    let (bound, handle) = spawn_server(state).await;

    let (mut socket, _resp) =
        tokio_tungstenite::connect_async(format!("ws://{bound}/ws/session/ws-bin"))
            .await
            .unwrap();
    let _snap = next_json(&mut socket).await;
    socket
        .send(WsMessage::Binary(vec![0xDE, 0xAD, 0xBE, 0xEF].into()))
        .await
        .unwrap();
    registry.upsert(review_with_status("ws-bin", SessionStatus::Decided));
    let upd = next_json(&mut socket).await;
    assert_eq!(upd["status"], "decided");
    socket.send(WsMessage::Close(None)).await.ok();
    handle.abort();
}

#[tokio::test]
async fn ws_client_close_ends_session_cleanly() {
    let (state, _tmp) = app_state();
    state.sessions.upsert(review("ws-close"));
    let (bound, handle) = spawn_server(state).await;

    let (mut socket, _resp) =
        tokio_tungstenite::connect_async(format!("ws://{bound}/ws/session/ws-close"))
            .await
            .unwrap();
    let _snap = next_json(&mut socket).await;
    // Client-initiated close; server should not error or hang.
    socket.send(WsMessage::Close(None)).await.ok();
    // Drain whatever the server sends back (echoed close).
    let _ = tokio::time::timeout(Duration::from_secs(2), async {
        while let Some(m) = socket.next().await {
            if matches!(m, Ok(WsMessage::Close(_)) | Err(_)) {
                break;
            }
        }
    })
    .await;
    handle.abort();
}

// ── Concurrent observers over the real socket ───────────────────────────────

#[tokio::test]
async fn two_concurrent_observers_both_get_snapshot() {
    let (state, _tmp) = app_state();
    state.sessions.upsert(review("ws-multi"));
    let (bound, handle) = spawn_server(state).await;

    let (mut s1, _) =
        tokio_tungstenite::connect_async(format!("ws://{bound}/ws/session/ws-multi"))
            .await
            .unwrap();
    let (mut s2, _) =
        tokio_tungstenite::connect_async(format!("ws://{bound}/ws/session/ws-multi"))
            .await
            .unwrap();
    let j1 = next_json(&mut s1).await;
    let j2 = next_json(&mut s2).await;
    assert_eq!(j1["session_id"], "ws-multi");
    assert_eq!(j2["session_id"], "ws-multi");

    s1.send(WsMessage::Close(None)).await.ok();
    s2.send(WsMessage::Close(None)).await.ok();
    handle.abort();
}

#[tokio::test]
async fn two_concurrent_observers_both_get_update() {
    let (state, _tmp) = app_state();
    state.sessions.upsert(review("ws-fan"));
    let registry = state.sessions.clone();
    let (bound, handle) = spawn_server(state).await;

    let (mut s1, _) = tokio_tungstenite::connect_async(format!("ws://{bound}/ws/session/ws-fan"))
        .await
        .unwrap();
    let (mut s2, _) = tokio_tungstenite::connect_async(format!("ws://{bound}/ws/session/ws-fan"))
        .await
        .unwrap();
    let _ = next_json(&mut s1).await;
    let _ = next_json(&mut s2).await;

    registry.upsert(review_with_status("ws-fan", SessionStatus::Approved));
    let u1 = next_json(&mut s1).await;
    let u2 = next_json(&mut s2).await;
    assert_eq!(u1["status"], "approved");
    assert_eq!(u2["status"], "approved");

    s1.send(WsMessage::Close(None)).await.ok();
    s2.send(WsMessage::Close(None)).await.ok();
    handle.abort();
}

#[tokio::test]
async fn many_concurrent_observers_all_get_update() {
    let (state, _tmp) = app_state();
    state.sessions.upsert(review("ws-storm"));
    let registry = state.sessions.clone();
    let (bound, handle) = spawn_server(state).await;

    let mut sockets = Vec::new();
    for _ in 0..16 {
        let (mut s, _) =
            tokio_tungstenite::connect_async(format!("ws://{bound}/ws/session/ws-storm"))
                .await
                .unwrap();
        // Drain the snapshot before pushing the update.
        let _ = next_json(&mut s).await;
        sockets.push(s);
    }

    registry.upsert(review_with_status("ws-storm", SessionStatus::Approved));
    for s in sockets.iter_mut() {
        let u = next_json(s).await;
        assert_eq!(u["status"], "approved");
    }
    for mut s in sockets {
        s.send(WsMessage::Close(None)).await.ok();
    }
    handle.abort();
}

#[tokio::test]
async fn observers_on_distinct_sessions_are_isolated() {
    let (state, _tmp) = app_state();
    state.sessions.upsert(review("alpha"));
    state.sessions.upsert(review("beta"));
    let registry = state.sessions.clone();
    let (bound, handle) = spawn_server(state).await;

    let (mut sa, _) = tokio_tungstenite::connect_async(format!("ws://{bound}/ws/session/alpha"))
        .await
        .unwrap();
    let (mut sb, _) = tokio_tungstenite::connect_async(format!("ws://{bound}/ws/session/beta"))
        .await
        .unwrap();
    let _ = next_json(&mut sa).await;
    let _ = next_json(&mut sb).await;

    // Only "alpha" mutates; sb must see no traffic.
    registry.upsert(review_with_status("alpha", SessionStatus::Approved));
    let ua = next_json(&mut sa).await;
    assert_eq!(ua["session_id"], "alpha");
    assert_eq!(ua["status"], "approved");

    let quiet = tokio::time::timeout(Duration::from_millis(300), sb.next()).await;
    assert!(quiet.is_err(), "beta observer should have seen no frames");

    sa.send(WsMessage::Close(None)).await.ok();
    sb.send(WsMessage::Close(None)).await.ok();
    handle.abort();
}

// ── WS-session cap over the socket ──────────────────────────────────────────

#[tokio::test]
async fn ws_cap_rejects_over_limit_connection() {
    let tmp = tempfile::tempdir().unwrap();
    let store = StateStore::new(tmp.path());
    let limits = Limits {
        max_ws_sessions: 1,
        ..Limits::default()
    };
    let state = AppState::new(store, limits);
    state.sessions.upsert(review("cap"));
    let (bound, handle) = spawn_server(state).await;

    // First connection takes the single slot and stays open.
    let (mut first, _) = tokio_tungstenite::connect_async(format!("ws://{bound}/ws/session/cap"))
        .await
        .unwrap();
    let _ = next_json(&mut first).await;

    // Second connection should be closed (1013) without a snapshot.
    let (mut second, _) = tokio_tungstenite::connect_async(format!("ws://{bound}/ws/session/cap"))
        .await
        .unwrap();
    let mut closed = false;
    while let Some(msg) = tokio::time::timeout(Duration::from_secs(5), second.next())
        .await
        .expect("not timed out")
    {
        match msg {
            Ok(WsMessage::Close(frame)) => {
                if let Some(f) = frame {
                    assert_eq!(u16::from(f.code), 1013, "expected try-again-later code");
                }
                closed = true;
                break;
            }
            Ok(WsMessage::Text(t)) => panic!("over-cap connection got a snapshot: {t}"),
            Ok(_) => continue,
            Err(_) => {
                closed = true;
                break;
            }
        }
    }
    assert!(closed, "over-cap connection should be closed");

    first.send(WsMessage::Close(None)).await.ok();
    handle.abort();
}

#[tokio::test]
async fn ws_slot_freed_after_disconnect_allows_new_connection() {
    let tmp = tempfile::tempdir().unwrap();
    let store = StateStore::new(tmp.path());
    let limits = Limits {
        max_ws_sessions: 1,
        ..Limits::default()
    };
    let state = AppState::new(store, limits);
    state.sessions.upsert(review("reuse"));
    let (bound, handle) = spawn_server(state).await;

    {
        let (mut first, _) =
            tokio_tungstenite::connect_async(format!("ws://{bound}/ws/session/reuse"))
                .await
                .unwrap();
        let _ = next_json(&mut first).await;
        first.send(WsMessage::Close(None)).await.ok();
        // Let the server observe the close and free the slot.
        let _ = tokio::time::timeout(Duration::from_secs(2), async {
            while let Some(m) = first.next().await {
                if matches!(m, Ok(WsMessage::Close(_)) | Err(_)) {
                    break;
                }
            }
        })
        .await;
    }

    // Poll: a fresh connection should eventually succeed (slot reclaimed).
    let mut got_snapshot = false;
    for _ in 0..40 {
        if let Ok((mut s, _)) =
            tokio_tungstenite::connect_async(format!("ws://{bound}/ws/session/reuse")).await
        {
            match tokio::time::timeout(Duration::from_millis(300), next_text(&mut s)).await {
                Ok(text) => {
                    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
                    if json["session_id"] == "reuse" {
                        got_snapshot = true;
                        s.send(WsMessage::Close(None)).await.ok();
                        break;
                    }
                }
                Err(_) => { /* slot not yet freed; retry */ }
            }
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(got_snapshot, "slot should be reusable after disconnect");
    handle.abort();
}

// ── Snapshot reflects the *current* payload at connect time ─────────────────

#[tokio::test]
async fn ws_snapshot_reflects_latest_upsert() {
    let (state, _tmp) = app_state();
    // Register, then mutate before any client connects.
    state.sessions.upsert(review_with_status("latest", SessionStatus::Pending));
    state.sessions.upsert(review_with_status("latest", SessionStatus::Approved));
    let (bound, handle) = spawn_server(state).await;

    let (mut socket, _) =
        tokio_tungstenite::connect_async(format!("ws://{bound}/ws/session/latest"))
            .await
            .unwrap();
    let json = next_json(&mut socket).await;
    // The snapshot is the most-recent state, not the original.
    assert_eq!(json["status"], "approved");
    socket.send(WsMessage::Close(None)).await.ok();
    handle.abort();
}

#[tokio::test]
async fn ws_late_subscriber_after_updates_still_gets_current_snapshot() {
    let (state, _tmp) = app_state();
    state.sessions.upsert(review("late"));
    let registry = state.sessions.clone();
    let (bound, handle) = spawn_server(state).await;

    // Mutate a few times before the client ever connects.
    for status in [SessionStatus::Decided, SessionStatus::ChangesRequested] {
        registry.upsert(review_with_status("late", status));
    }
    let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://{bound}/ws/session/late"))
        .await
        .unwrap();
    let json = next_json(&mut socket).await;
    // A late subscriber sees the latest snapshot, not the missed intermediate
    // frames.
    assert_eq!(json["status"], "changes_requested");
    socket.send(WsMessage::Close(None)).await.ok();
    handle.abort();
}

// ════════════════════════════════════════════════════════════════════════════
// Serde determinism / roundtrips through the registry frames
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn serialized_frame_roundtrips_through_payload() {
    let reg = empty_registry();
    reg.upsert(review("rt"));
    let stored = reg.get("rt").unwrap();
    let frame = serde_json::to_string(&stored).unwrap();
    let back: SessionPayload = serde_json::from_str(&frame).unwrap();
    assert_eq!(back.session_id(), "rt");
    assert_eq!(back.session_type(), "review");
}

#[test]
fn serialized_frame_is_deterministic() {
    let reg = empty_registry();
    reg.upsert(review("det"));
    let a = serde_json::to_string(&reg.get("det").unwrap()).unwrap();
    let b = serde_json::to_string(&reg.get("det").unwrap()).unwrap();
    assert_eq!(a, b);
}

#[test]
fn every_variant_roundtrips() {
    for make in [
        review as fn(&str) -> SessionPayload,
        question,
        direction,
        picker,
        view,
    ] {
        let payload = make("rt");
        let frame = serde_json::to_string(&payload).unwrap();
        let back: SessionPayload = serde_json::from_str(&frame).unwrap();
        assert_eq!(back.session_id(), payload.session_id());
        assert_eq!(back.session_type(), payload.session_type());
    }
}

#[test]
fn frame_includes_session_type_discriminator() {
    for (make, ty) in [
        (review as fn(&str) -> SessionPayload, "review"),
        (question, "question"),
        (direction, "direction"),
        (picker, "picker"),
        (view, "view"),
    ] {
        let frame = serde_json::to_string(&make("x")).unwrap();
        let json: serde_json::Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(json["session_type"], ty);
    }
}

#[test]
fn status_serializes_snake_case() {
    for (status, wire) in [
        (SessionStatus::Pending, "pending"),
        (SessionStatus::Decided, "decided"),
        (SessionStatus::Answered, "answered"),
        (SessionStatus::Approved, "approved"),
        (SessionStatus::ChangesRequested, "changes_requested"),
    ] {
        let frame = serde_json::to_string(&review_with_status("s", status)).unwrap();
        let json: serde_json::Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(json["status"], wire, "status {status:?}");
    }
}

// ════════════════════════════════════════════════════════════════════════════
// AppState wiring — registry shared through AppState clones
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn app_state_clone_shares_registry() {
    let (state, _tmp) = app_state();
    let clone = state.clone();
    state.sessions.upsert(review("shared"));
    assert!(clone.sessions.contains("shared"));
}

#[test]
fn app_state_registry_independent_of_limits() {
    let tmp = tempfile::tempdir().unwrap();
    let store = StateStore::new(tmp.path());
    let a = AppState::new(store, Limits::default());
    let tmp2 = tempfile::tempdir().unwrap();
    let store2 = StateStore::new(tmp2.path());
    let b = AppState::new(store2, Limits { remote: true, ..Limits::default() });
    a.sessions.upsert(review("only-a"));
    // Two separately-constructed AppStates do not share a registry.
    assert!(a.sessions.contains("only-a"));
    assert!(!b.sessions.contains("only-a"));
}

#[test]
fn app_state_carries_configured_limits() {
    let tmp = tempfile::tempdir().unwrap();
    let store = StateStore::new(tmp.path());
    let limits = Limits {
        max_ws_sessions: 7,
        remote: true,
        ..Limits::default()
    };
    let state = AppState::new(store, limits);
    assert_eq!(state.limits.max_ws_sessions, 7);
    assert!(state.limits.remote);
}

// ── next_session_id (the in-memory replacement for on-disk id minting) ────────

#[test]
fn next_session_id_starts_at_one_per_prefix() {
    let reg = SessionRegistry::new();
    assert_eq!(reg.next_session_id("q"), "q-01");
    assert_eq!(reg.next_session_id("d"), "d-01");
    assert_eq!(reg.next_session_id("p"), "p-01");
}

#[test]
fn next_session_id_advances_past_registered_sessions() {
    let reg = SessionRegistry::new();
    reg.upsert(question("q-01"));
    reg.upsert(question("q-02"));
    // Minting scans the live registry, so the next id is monotonic.
    assert_eq!(reg.next_session_id("q"), "q-03");
    // A different prefix is independent.
    assert_eq!(reg.next_session_id("d"), "d-01");
}

#[test]
fn next_session_id_ignores_unparseable_suffixes() {
    let reg = SessionRegistry::new();
    reg.upsert(review("rev-frame")); // not a `q-NN` id
    assert_eq!(reg.next_session_id("q"), "q-01");
}
