//! Comprehensive in-process route tests for darkrun-http — area "runs_browse".
//!
//! Drives the public `build_router` surface via `tower::ServiceExt::oneshot`
//! (no socket bind) to exercise the runs browse REST routes end to end:
//! - `GET /api/runs` (empty / populated / sorted / archived-omitted / summary
//!   fields / progress / CORS / method edges)
//! - `GET /api/runs/:slug` (present / 404 / stations / units-on-active / phase /
//!   serde round-trip)
//!
//! Runs are seeded directly through [`darkrun_core::StateStore`] so the tests
//! own the on-disk shape and never depend on the engine crate.

use std::collections::BTreeMap;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use darkrun_api::{RunDetailPayload, RunListPayload};
use darkrun_core::domain::{
    Run, RunFrontmatter, Station, StationPhase, Status, Unit, UnitFrontmatter,
};
use darkrun_core::state::RunState;
use darkrun_core::StateStore;
use darkrun_http::{build_router, AppState, Limits};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

// ── Fixtures ────────────────────────────────────────────────────────────────

/// A state plus the store behind it, sharing one leaked tempdir so the on-disk
/// runs survive for the whole test.
fn state_with_store() -> (AppState, StateStore) {
    let tmp = tempfile::tempdir().expect("tmp");
    let path = tmp.path().to_path_buf();
    std::mem::forget(tmp);
    let store = StateStore::new(&path);
    let app_store = StateStore::new(&path);
    (AppState::new(app_store, Limits::default()), store)
}

fn remote_state_with_store() -> (AppState, StateStore) {
    let tmp = tempfile::tempdir().expect("tmp");
    let path = tmp.path().to_path_buf();
    std::mem::forget(tmp);
    let store = StateStore::new(&path);
    let limits = Limits {
        remote: true,
        rate_limit_per_min: 100_000,
        ..Limits::default()
    };
    (AppState::new(StateStore::new(&path), limits), store)
}

/// Seed a run document with the given slug/title/factory/status/active station.
#[allow(clippy::too_many_arguments)]
fn seed_run(
    store: &StateStore,
    slug: &str,
    title: Option<&str>,
    factory: &str,
    active_station: &str,
    status: Status,
    started_at: Option<&str>,
    archived: bool,
) {
    let frontmatter = RunFrontmatter {
        title: title.map(str::to_string),
        factory: factory.to_string(),
        mode: "continuous".to_string(),
        active_station: active_station.to_string(),
        status,
        archived: if archived { Some(true) } else { None },
        started_at: started_at.map(str::to_string),
        ..Default::default()
    };
    let resolved = title.unwrap_or(slug).to_string();
    let run = Run {
        slug: slug.to_string(),
        frontmatter,
        title: resolved.clone(),
        body: format!("# {resolved}\n"),
    };
    store.write_run(&run).expect("write run");
}

fn station(name: &str, status: Status, phase: StationPhase, started_at: Option<&str>) -> Station {
    Station {
        station: name.to_string(),
        status,
        phase,
        elaborated: false,
        checkpoint: None,
        chosen_checkpoint: None,
        branch: None,
        pr_ref: None,
        pr_status: None,
        pr_ready_at: None,
        pr_merged_at: None,
        verifier_nonce: None,
        started_at: started_at.map(str::to_string),
        completed_at: None,
    }
}

/// Seed a run's derived state with an ordered set of stations.
fn seed_state(store: &StateStore, slug: &str, factory: &str, active: &str, stations: Vec<Station>) {
    let mut map: BTreeMap<String, Station> = BTreeMap::new();
    for s in stations {
        map.insert(s.station.clone(), s);
    }
    let state = RunState {
        factory: factory.to_string(),
        active_station: active.to_string(),
        stations: map,
        ..Default::default()
    };
    store.write_state(slug, &state).expect("write state");
}

fn seed_unit(store: &StateStore, run: &str, slug: &str, title: &str, status: Status, station: &str) {
    let unit = Unit {
        slug: slug.to_string(),
        frontmatter: UnitFrontmatter {
            status,
            station: Some(station.to_string()),
            ..Default::default()
        },
        title: title.to_string(),
        body: format!("# {title}\n"),
    };
    store.write_unit(run, &unit).expect("write unit");
}

// ── Request helpers ───────────────────────────────────────────────────────────

async fn send(app: axum::Router, req: Request<Body>) -> axum::response::Response {
    app.oneshot(req).await.unwrap()
}

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
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
// GET /api/runs — empty
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn runs_empty_is_200() {
    let (app, _store) = state_with_store();
    let resp = send(build_router(app), get("/api/runs")).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn runs_empty_body_shape() {
    let (app, _store) = state_with_store();
    let resp = send(build_router(app), get("/api/runs")).await;
    let json = body_json(resp).await;
    assert_eq!(json["count"], 0);
    assert!(json["runs"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn runs_empty_content_type_is_json() {
    let (app, _store) = state_with_store();
    let resp = send(build_router(app), get("/api/runs")).await;
    assert!(content_type(&resp).starts_with("application/json"));
}

#[tokio::test]
async fn runs_empty_roundtrips_into_typed() {
    let (app, _store) = state_with_store();
    let resp = send(build_router(app), get("/api/runs")).await;
    let bytes = body_bytes(resp).await;
    let parsed: RunListPayload = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(parsed.count, 0);
    assert!(parsed.runs.is_empty());
}

// ════════════════════════════════════════════════════════════════════════════
// GET /api/runs — populated
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn runs_populated_count_matches() {
    let (app, store) = state_with_store();
    seed_run(&store, "alpha", Some("Alpha"), "software", "frame", Status::Active, None, false);
    seed_run(&store, "beta", None, "software", "build", Status::Completed, None, false);
    let resp = send(build_router(app), get("/api/runs")).await;
    let json = body_json(resp).await;
    assert_eq!(json["count"], 2);
    assert_eq!(json["runs"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn runs_summary_carries_all_fields() {
    let (app, store) = state_with_store();
    seed_run(
        &store,
        "alpha",
        Some("Alpha"),
        "software",
        "frame",
        Status::Active,
        Some("2026-05-30T00:00:00Z"),
        false,
    );
    seed_state(
        &store,
        "alpha",
        "software",
        "frame",
        vec![
            station("frame", Status::Active, StationPhase::Manufacture, Some("2026-05-30T00:00:00Z")),
            station("build", Status::Pending, StationPhase::Spec, None),
        ],
    );
    let resp = send(build_router(app), get("/api/runs")).await;
    let json = body_json(resp).await;
    let run = &json["runs"][0];
    assert_eq!(run["slug"], "alpha");
    assert_eq!(run["title"], "Alpha");
    assert_eq!(run["factory"], "software");
    assert_eq!(run["active_station"], "frame");
    assert_eq!(run["phase"], "manufacture");
    assert_eq!(run["status"], "active");
    assert_eq!(run["started_at"], "2026-05-30T00:00:00Z");
    assert_eq!(run["progress"]["completed"], 0);
    assert_eq!(run["progress"]["total"], 2);
}

#[tokio::test]
async fn runs_title_falls_back_to_slug() {
    let (app, store) = state_with_store();
    seed_run(&store, "no-title", None, "software", "frame", Status::Active, None, false);
    let resp = send(build_router(app), get("/api/runs")).await;
    let json = body_json(resp).await;
    assert_eq!(json["runs"][0]["title"], "no-title");
}

#[tokio::test]
async fn runs_progress_counts_completed_stations() {
    let (app, store) = state_with_store();
    seed_run(&store, "alpha", Some("Alpha"), "software", "build", Status::Active, None, false);
    seed_state(
        &store,
        "alpha",
        "software",
        "build",
        vec![
            station("frame", Status::Completed, StationPhase::Checkpoint, Some("2026-05-30T00:00:00Z")),
            station("specify", Status::Completed, StationPhase::Checkpoint, Some("2026-05-30T01:00:00Z")),
            station("build", Status::Active, StationPhase::Manufacture, Some("2026-05-30T02:00:00Z")),
        ],
    );
    let resp = send(build_router(app), get("/api/runs")).await;
    let json = body_json(resp).await;
    assert_eq!(json["runs"][0]["progress"]["completed"], 2);
    assert_eq!(json["runs"][0]["progress"]["total"], 3);
}

#[tokio::test]
async fn runs_without_state_have_zero_progress() {
    let (app, store) = state_with_store();
    seed_run(&store, "alpha", Some("Alpha"), "software", "frame", Status::Active, None, false);
    let resp = send(build_router(app), get("/api/runs")).await;
    let json = body_json(resp).await;
    assert_eq!(json["runs"][0]["progress"]["completed"], 0);
    assert_eq!(json["runs"][0]["progress"]["total"], 0);
}

#[tokio::test]
async fn runs_phase_absent_without_state() {
    let (app, store) = state_with_store();
    seed_run(&store, "alpha", Some("Alpha"), "software", "frame", Status::Active, None, false);
    let resp = send(build_router(app), get("/api/runs")).await;
    let json = body_json(resp).await;
    // No state → no active phase → key omitted.
    assert!(json["runs"][0].get("phase").is_none());
}

#[tokio::test]
async fn runs_started_at_omitted_when_absent() {
    let (app, store) = state_with_store();
    seed_run(&store, "alpha", Some("Alpha"), "software", "frame", Status::Active, None, false);
    let resp = send(build_router(app), get("/api/runs")).await;
    let json = body_json(resp).await;
    assert!(json["runs"][0].get("started_at").is_none());
}

#[tokio::test]
async fn runs_status_serializes_as_wire_string() {
    let (app, store) = state_with_store();
    seed_run(&store, "a", None, "software", "frame", Status::Completed, None, false);
    seed_run(&store, "b", None, "software", "frame", Status::InProgress, None, false);
    let resp = send(build_router(app), get("/api/runs")).await;
    let json = body_json(resp).await;
    assert_eq!(json["runs"][0]["status"], "completed");
    assert_eq!(json["runs"][1]["status"], "in_progress");
}

#[tokio::test]
async fn runs_populated_roundtrips_into_typed() {
    let (app, store) = state_with_store();
    seed_run(&store, "alpha", Some("Alpha"), "software", "frame", Status::Active, None, false);
    let resp = send(build_router(app), get("/api/runs")).await;
    let bytes = body_bytes(resp).await;
    let parsed: RunListPayload = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(parsed.count, 1);
    assert_eq!(parsed.runs[0].slug, "alpha");
    assert_eq!(parsed.runs[0].title, "Alpha");
}

// ════════════════════════════════════════════════════════════════════════════
// GET /api/runs — sorting
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn runs_are_sorted_by_slug() {
    let (app, store) = state_with_store();
    for slug in ["zeta", "alpha", "mike", "bravo"] {
        seed_run(&store, slug, None, "software", "frame", Status::Active, None, false);
    }
    let resp = send(build_router(app), get("/api/runs")).await;
    let json = body_json(resp).await;
    let slugs: Vec<&str> = json["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["slug"].as_str().unwrap())
        .collect();
    assert_eq!(slugs, ["alpha", "bravo", "mike", "zeta"]);
}

#[tokio::test]
async fn runs_listing_is_deterministic() {
    let (app, store) = state_with_store();
    for slug in ["c", "a", "b"] {
        seed_run(&store, slug, None, "software", "frame", Status::Active, None, false);
    }
    let app = build_router(app);
    let a = body_bytes(send(app.clone(), get("/api/runs")).await).await;
    let b = body_bytes(send(app, get("/api/runs")).await).await;
    assert_eq!(a, b);
}

// ════════════════════════════════════════════════════════════════════════════
// GET /api/runs — archived omission
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn runs_omit_archived() {
    let (app, store) = state_with_store();
    seed_run(&store, "live", None, "software", "frame", Status::Active, None, false);
    seed_run(&store, "gone", None, "software", "frame", Status::Completed, None, true);
    let resp = send(build_router(app), get("/api/runs")).await;
    let json = body_json(resp).await;
    assert_eq!(json["count"], 1);
    assert_eq!(json["runs"][0]["slug"], "live");
}

#[tokio::test]
async fn runs_all_archived_is_empty() {
    let (app, store) = state_with_store();
    seed_run(&store, "gone", None, "software", "frame", Status::Completed, None, true);
    let resp = send(build_router(app), get("/api/runs")).await;
    let json = body_json(resp).await;
    assert_eq!(json["count"], 0);
}

// ════════════════════════════════════════════════════════════════════════════
// GET /api/runs — method / CORS edges
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn runs_post_is_405() {
    let (app, _store) = state_with_store();
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/runs")
        .body(Body::empty())
        .unwrap();
    let resp = send(build_router(app), req).await;
    assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn runs_remote_get_carries_acao_star() {
    let (app, store) = remote_state_with_store();
    seed_run(&store, "alpha", None, "software", "frame", Status::Active, None, false);
    let req = Request::builder()
        .uri("/api/runs")
        .header(header::ORIGIN, "https://app.example.com")
        .body(Body::empty())
        .unwrap();
    let resp = send(build_router(app), req).await;
    let allow = resp
        .headers()
        .get("access-control-allow-origin")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert_eq!(allow, "*");
}

#[tokio::test]
async fn runs_get_is_nonmutating() {
    let (app, store) = state_with_store();
    seed_run(&store, "alpha", None, "software", "frame", Status::Active, None, false);
    let app = build_router(app);
    for _ in 0..3 {
        let resp = send(app.clone(), get("/api/runs")).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }
    // Still exactly one run on disk.
    let resp = send(app, get("/api/runs")).await;
    let json = body_json(resp).await;
    assert_eq!(json["count"], 1);
}

// ════════════════════════════════════════════════════════════════════════════
// GET /api/runs/:slug — present
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn run_detail_present_is_200() {
    let (app, store) = state_with_store();
    seed_run(&store, "alpha", Some("Alpha"), "software", "frame", Status::Active, None, false);
    let resp = send(build_router(app), get("/api/runs/alpha")).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn run_detail_top_level_fields() {
    let (app, store) = state_with_store();
    seed_run(
        &store,
        "alpha",
        Some("Alpha"),
        "software",
        "build",
        Status::Active,
        Some("2026-05-30T00:00:00Z"),
        false,
    );
    seed_state(
        &store,
        "alpha",
        "software",
        "build",
        vec![
            station("frame", Status::Completed, StationPhase::Checkpoint, Some("2026-05-30T00:00:00Z")),
            station("build", Status::Active, StationPhase::Manufacture, Some("2026-05-30T01:00:00Z")),
        ],
    );
    let resp = send(build_router(app), get("/api/runs/alpha")).await;
    let json = body_json(resp).await;
    assert_eq!(json["slug"], "alpha");
    assert_eq!(json["title"], "Alpha");
    assert_eq!(json["factory"], "software");
    assert_eq!(json["active_station"], "build");
    assert_eq!(json["phase"], "manufacture");
    assert_eq!(json["status"], "active");
    assert_eq!(json["progress"]["completed"], 1);
    assert_eq!(json["progress"]["total"], 2);
    assert_eq!(json["started_at"], "2026-05-30T00:00:00Z");
}

#[tokio::test]
async fn run_detail_stations_in_walk_order() {
    let (app, store) = state_with_store();
    seed_run(&store, "alpha", Some("Alpha"), "software", "build", Status::Active, None, false);
    seed_state(
        &store,
        "alpha",
        "software",
        "build",
        vec![
            // Insertion order here is not the walk order; started_at decides.
            station("build", Status::Active, StationPhase::Manufacture, Some("2026-05-30T02:00:00Z")),
            station("frame", Status::Completed, StationPhase::Checkpoint, Some("2026-05-30T00:00:00Z")),
            station("specify", Status::Completed, StationPhase::Checkpoint, Some("2026-05-30T01:00:00Z")),
        ],
    );
    let resp = send(build_router(app), get("/api/runs/alpha")).await;
    let json = body_json(resp).await;
    let names: Vec<&str> = json["stations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["frame", "specify", "build"]);
}

#[tokio::test]
async fn run_detail_orders_unstamped_stations_after_stamped_ones() {
    // Mixed started_at exercises every arm of the walk-order comparator: a
    // stamped station sorts before an unstamped one, and two unstamped ones fall
    // back to name order.
    let (app, store) = state_with_store();
    seed_run(&store, "alpha", None, "software", "build", Status::Active, None, false);
    seed_state(
        &store,
        "alpha",
        "software",
        "build",
        vec![
            station("specify", Status::Pending, StationPhase::Spec, None),
            station("build", Status::Active, StationPhase::Manufacture, Some("2026-05-30T00:00:00Z")),
            station("frame", Status::Pending, StationPhase::Spec, None),
        ],
    );
    let resp = send(build_router(app), get("/api/runs/alpha")).await;
    let json = body_json(resp).await;
    let names: Vec<&str> = json["stations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    // build (stamped) first; then the two unstamped by name (frame, specify).
    assert_eq!(names, ["build", "frame", "specify"]);
}

#[tokio::test]
async fn run_detail_station_fields() {
    let (app, store) = state_with_store();
    seed_run(&store, "alpha", None, "software", "frame", Status::Active, None, false);
    let mut frame = station("frame", Status::Active, StationPhase::Spec, Some("2026-05-30T00:00:00Z"));
    frame.completed_at = None;
    seed_state(&store, "alpha", "software", "frame", vec![frame]);
    let resp = send(build_router(app), get("/api/runs/alpha")).await;
    let json = body_json(resp).await;
    let s = &json["stations"][0];
    assert_eq!(s["name"], "frame");
    assert_eq!(s["status"], "active");
    assert_eq!(s["phase"], "spec");
    assert_eq!(s["started_at"], "2026-05-30T00:00:00Z");
    assert!(s.get("completed_at").is_none());
}

#[tokio::test]
async fn run_detail_units_only_on_active_station() {
    let (app, store) = state_with_store();
    seed_run(&store, "alpha", None, "software", "build", Status::Active, None, false);
    seed_unit(&store, "alpha", "u-frame", "Frame Unit", Status::Completed, "frame");
    seed_unit(&store, "alpha", "u-build-1", "Build One", Status::Active, "build");
    seed_unit(&store, "alpha", "u-build-2", "Build Two", Status::Pending, "build");
    let resp = send(build_router(app), get("/api/runs/alpha")).await;
    let json = body_json(resp).await;
    let units = json["units"].as_array().unwrap();
    assert_eq!(units.len(), 2);
    let slugs: Vec<&str> = units.iter().map(|u| u["slug"].as_str().unwrap()).collect();
    assert_eq!(slugs, ["u-build-1", "u-build-2"]);
}

#[tokio::test]
async fn run_detail_unit_fields() {
    let (app, store) = state_with_store();
    seed_run(&store, "alpha", None, "software", "build", Status::Active, None, false);
    seed_unit(&store, "alpha", "u-1", "Build One", Status::Active, "build");
    let resp = send(build_router(app), get("/api/runs/alpha")).await;
    let json = body_json(resp).await;
    let u = &json["units"][0];
    assert_eq!(u["slug"], "u-1");
    assert_eq!(u["title"], "Build One");
    assert_eq!(u["status"], "active");
    assert_eq!(u["station"], "build");
}

#[tokio::test]
async fn run_detail_no_units_is_empty_array() {
    let (app, store) = state_with_store();
    seed_run(&store, "alpha", None, "software", "frame", Status::Active, None, false);
    let resp = send(build_router(app), get("/api/runs/alpha")).await;
    let json = body_json(resp).await;
    assert!(json["units"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn run_detail_no_state_has_empty_stations() {
    let (app, store) = state_with_store();
    seed_run(&store, "alpha", None, "software", "frame", Status::Active, None, false);
    let resp = send(build_router(app), get("/api/runs/alpha")).await;
    let json = body_json(resp).await;
    assert!(json["stations"].as_array().unwrap().is_empty());
    assert!(json.get("phase").is_none());
}

#[tokio::test]
async fn run_detail_roundtrips_into_typed() {
    let (app, store) = state_with_store();
    seed_run(&store, "alpha", Some("Alpha"), "software", "frame", Status::Active, None, false);
    seed_state(
        &store,
        "alpha",
        "software",
        "frame",
        vec![station("frame", Status::Active, StationPhase::Spec, Some("2026-05-30T00:00:00Z"))],
    );
    seed_unit(&store, "alpha", "u-1", "Unit One", Status::Active, "frame");
    let resp = send(build_router(app), get("/api/runs/alpha")).await;
    let bytes = body_bytes(resp).await;
    let parsed: RunDetailPayload = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(parsed.slug, "alpha");
    assert_eq!(parsed.stations.len(), 1);
    assert_eq!(parsed.stations[0].name, "frame");
    assert_eq!(parsed.units.len(), 1);
    assert_eq!(parsed.units[0].slug, "u-1");
}

#[tokio::test]
async fn run_detail_content_type_is_json() {
    let (app, store) = state_with_store();
    seed_run(&store, "alpha", None, "software", "frame", Status::Active, None, false);
    let resp = send(build_router(app), get("/api/runs/alpha")).await;
    assert!(content_type(&resp).starts_with("application/json"));
}

#[tokio::test]
async fn run_detail_archived_run_is_still_fetchable_by_slug() {
    // The list omits archived runs, but a direct detail fetch still resolves —
    // the browse surface can deep-link an archived run.
    let (app, store) = state_with_store();
    seed_run(&store, "gone", Some("Gone"), "software", "frame", Status::Completed, None, true);
    let resp = send(build_router(app), get("/api/runs/gone")).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["slug"], "gone");
}

// ════════════════════════════════════════════════════════════════════════════
// GET /api/runs/:slug — 404 / edges
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn run_detail_absent_is_404() {
    let (app, _store) = state_with_store();
    let resp = send(build_router(app), get("/api/runs/ghost")).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn run_detail_absent_error_envelope() {
    let (app, _store) = state_with_store();
    let resp = send(build_router(app), get("/api/runs/ghost")).await;
    let json = body_json(resp).await;
    assert_eq!(json["error"], "run not found");
    assert_eq!(json["id"], "ghost");
}

#[tokio::test]
async fn run_detail_absent_content_type_is_json() {
    let (app, _store) = state_with_store();
    let resp = send(build_router(app), get("/api/runs/ghost")).await;
    assert!(content_type(&resp).starts_with("application/json"));
}

#[tokio::test]
async fn run_detail_post_is_405() {
    let (app, store) = state_with_store();
    seed_run(&store, "alpha", None, "software", "frame", Status::Active, None, false);
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/runs/alpha")
        .body(Body::empty())
        .unwrap();
    let resp = send(build_router(app), req).await;
    assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn run_detail_each_seeded_run_resolvable() {
    let (app, store) = state_with_store();
    for slug in ["alpha", "beta", "gamma"] {
        seed_run(&store, slug, None, "software", "frame", Status::Active, None, false);
    }
    let app = build_router(app);
    for slug in ["alpha", "beta", "gamma"] {
        let resp = send(app.clone(), get(&format!("/api/runs/{slug}"))).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["slug"], slug);
    }
    // An unseeded slug 404s.
    let resp = send(app, get("/api/runs/delta")).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ════════════════════════════════════════════════════════════════════════════
// List ↔ detail consistency
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn list_summary_matches_detail_for_same_run() {
    let (app, store) = state_with_store();
    seed_run(
        &store,
        "alpha",
        Some("Alpha"),
        "software",
        "build",
        Status::Active,
        Some("2026-05-30T00:00:00Z"),
        false,
    );
    seed_state(
        &store,
        "alpha",
        "software",
        "build",
        vec![
            station("frame", Status::Completed, StationPhase::Checkpoint, Some("2026-05-30T00:00:00Z")),
            station("build", Status::Active, StationPhase::Audit, Some("2026-05-30T01:00:00Z")),
        ],
    );
    let app = build_router(app);

    let list = body_json(send(app.clone(), get("/api/runs")).await).await;
    let detail = body_json(send(app, get("/api/runs/alpha")).await).await;
    let summary = &list["runs"][0];

    for field in ["slug", "title", "factory", "active_station", "phase", "status", "started_at"] {
        assert_eq!(summary[field], detail[field], "field {field} diverged");
    }
    assert_eq!(summary["progress"], detail["progress"]);
}
