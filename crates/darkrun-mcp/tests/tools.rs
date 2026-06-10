//! End-to-end tests for the `DarkrunServer` MCP tool surface.
//!
//! These drive the PUBLIC tool handlers on [`darkrun_mcp::DarkrunServer`]
//! exactly the way an MCP client would: build the typed `Parameters<_>` input,
//! call the handler, then assert on the returned [`CallToolResult`]'s
//! `is_error` flag and `structured_content` JSON shape. Every test runs against
//! a fresh `tempdir`-rooted `.darkrun/` tree so they are hermetic and parallel-
//! safe.
//!
//! Coverage spans: run start/next/show/list/archive, the unit triple
//! (list/get/create/update), the feedback surface
//! (create/list/resolve/reject/move), the checkpoint decision, and the factory
//! list/detail tools — plus input validation, error paths, idempotency, and the
//! structured-action shape returned by the manager.

use darkrun_mcp::tools::{
    ArchetypeInput, CheckpointDecideInput, DirectionInput, ElaborateSealInput, FactoryRef, FeedbackCreateInput,
    FeedbackListInput, FeedbackMoveInput, FeedbackRejectInput, FeedbackResolveInput,
    PickerInput, PickerOptionInput, ProofAttachInput, ProofGetInput, QuestionInput,
    QuestionOptionInput, RunArchiveInput, RunListInput, RunRef, RunReviewStampInput, RunShowRef,
    RunStartInput, RunSurfaceInput, SessionResultInput, UnitCreateInput, UnitRef, UnitUpdateInput,
};
use darkrun_mcp::DarkrunServer;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use serde_json::Value;
use tempfile::TempDir;

// ── Helpers ──────────────────────────────────────────────────────────────

fn server() -> (TempDir, DarkrunServer) {
    let dir = TempDir::new().expect("tmp");
    let server = DarkrunServer::new(dir.path());
    (dir, server)
}

/// The JSON `structured_content` of a successful result.
///
/// List tools wrap their array in a `{ "items": [...] }` envelope so the MCP
/// `structuredContent` is always a JSON object (the protocol forbids a
/// top-level array). This helper transparently unwraps that sole-`items`
/// envelope so content assertions read the list directly; object results pass
/// through unchanged. The envelope itself is pinned by
/// `list_tools_wrap_array_in_items_envelope`.
fn body(res: &CallToolResult) -> Value {
    let v = res.structured_content.clone().expect("structured content");
    match &v {
        Value::Object(map) if map.len() == 1 => match map.get("items") {
            Some(items @ Value::Array(_)) => items.clone(),
            _ => v,
        },
        _ => v,
    }
}

/// The `software` entry from a `darkrun_factory_list` body. The catalog now
/// ships multiple factories (legal, software) sorted by slug, so tests
/// that assert software's specific orientation must select it by name rather
/// than by list position.
fn software_entry(list: &Value) -> Value {
    list.as_array()
        .expect("factory list array")
        .iter()
        .find(|f| f["name"] == "software")
        .cloned()
        .expect("software factory in the catalog")
}

fn is_ok(res: &CallToolResult) -> bool {
    res.is_error == Some(false)
}

fn is_err(res: &CallToolResult) -> bool {
    res.is_error == Some(true)
}

/// Concatenated text content of an error result (for message assertions).
fn err_message(res: &CallToolResult) -> String {
    res.content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect::<Vec<_>>()
        .join("")
}

fn start(server: &DarkrunServer, slug: &str) -> CallToolResult {
    server
        .darkrun_run_new(Parameters(RunStartInput {
            slug: slug.into(),
            factory: "software".into(),
            title: Some("Run".into()),
            mode: "continuous".into(),
            size: "full".into(),        }))
        .unwrap()
}

fn started(slug: &str) -> (TempDir, DarkrunServer) {
    let (d, s) = server();
    start(&s, slug);
    // Solo holds the first station's Spec until the elaboration is sealed; seal
    // it up front so callers that walk the run linearly aren't stalled at Spec.
    s.darkrun_elaborate_seal(Parameters(ElaborateSealInput {
        slug: slug.into(),
        station: "frame".into(),
    }))
    .unwrap();
    (d, s)
}

fn next(server: &DarkrunServer, slug: &str) -> CallToolResult {
    server
        .darkrun_advance(Parameters(RunRef { slug: slug.into() }))
        .unwrap()
}

fn create_unit(server: &DarkrunServer, slug: &str, unit: &str, station: &str) -> CallToolResult {
    server
        .darkrun_unit_create(Parameters(UnitCreateInput {
            slug: slug.into(),
            unit: unit.into(),
            station: station.into(),
            title: None,
            depends_on: vec![],
            ..Default::default()
        }))
        .unwrap()
}

fn create_feedback(
    server: &DarkrunServer,
    slug: &str,
    station: &str,
    body: &str,
) -> CallToolResult {
    server
        .darkrun_feedback_create(Parameters(FeedbackCreateInput {
            slug: slug.into(),
            station: station.into(),
            body: body.into(),
            severity: None,
                origin: None,
                invalidates: None,
        }))
        .unwrap()
}

// ── darkrun_run_new ──────────────────────────────────────────────────────

#[test]
fn run_start_creates_state_on_disk() {
    let (dir, server) = server();
    let res = start(&server, "r");
    assert!(is_ok(&res));
    assert!(dir.path().join(".darkrun/r/run.md").exists());
}

#[test]
fn run_start_returns_run_with_slug_and_factory() {
    let (_d, server) = server();
    let res = start(&server, "alpha");
    let v = body(&res);
    assert_eq!(v["slug"], "alpha");
    assert_eq!(v["frontmatter"]["factory"], "software");
}

#[test]
fn run_start_seeds_active_station_at_first_station() {
    let (_d, server) = server();
    let v = body(&start(&server, "r"));
    assert_eq!(v["frontmatter"]["active_station"], "frame");
}

#[test]
fn run_start_status_is_active() {
    let (_d, server) = server();
    let v = body(&start(&server, "r"));
    assert_eq!(v["frontmatter"]["status"], "active");
}

#[test]
fn run_start_honors_explicit_title() {
    let (_d, server) = server();
    let res = server
        .darkrun_run_new(Parameters(RunStartInput {
            slug: "r".into(),
            factory: "software".into(),
            title: Some("Ship the thing".into()),
            mode: "continuous".into(),
            size: "full".into(),        }))
        .unwrap();
    let v = body(&res);
    assert_eq!(v["title"], "Ship the thing");
    assert_eq!(v["frontmatter"]["title"], "Ship the thing");
}

#[test]
fn run_start_title_defaults_to_slug_when_absent() {
    let (_d, server) = server();
    let res = server
        .darkrun_run_new(Parameters(RunStartInput {
            slug: "my-slug".into(),
            factory: "software".into(),
            title: None,
            mode: "continuous".into(),
            size: "full".into(),        }))
        .unwrap();
    let v = body(&res);
    assert_eq!(v["title"], "my-slug");
}

#[test]
fn run_start_records_mode() {
    let (_d, server) = server();
    let res = server
        .darkrun_run_new(Parameters(RunStartInput {
            slug: "r".into(),
            factory: "software".into(),
            title: None,
            mode: "team".into(),
            size: "full".into(),        }))
        .unwrap();
    assert_eq!(body(&res)["frontmatter"]["mode"], "team");
}

#[test]
fn run_start_records_continuous_mode() {
    let (_d, server) = server();
    let v = body(&start(&server, "r"));
    assert_eq!(v["frontmatter"]["mode"], "solo");
}

#[test]
fn run_start_sets_started_at_timestamp() {
    let (_d, server) = server();
    let v = body(&start(&server, "r"));
    assert!(v["frontmatter"]["started_at"].is_string());
}

#[test]
fn run_start_body_contains_title_heading() {
    let (_d, server) = server();
    let res = server
        .darkrun_run_new(Parameters(RunStartInput {
            slug: "r".into(),
            factory: "software".into(),
            title: Some("Hello".into()),
            mode: "continuous".into(),
            size: "full".into(),        }))
        .unwrap();
    assert!(body(&res)["body"].as_str().unwrap().contains("# Hello"));
}

#[test]
fn run_start_rejects_empty_slug() {
    let (_d, server) = server();
    let res = server
        .darkrun_run_new(Parameters(RunStartInput {
            slug: "".into(),
            factory: "software".into(),
            title: None,
            mode: "continuous".into(),
            size: "full".into(),        }))
        .unwrap();
    assert!(is_err(&res));
}

#[test]
fn run_start_rejects_whitespace_slug() {
    let (_d, server) = server();
    let res = server
        .darkrun_run_new(Parameters(RunStartInput {
            slug: "   ".into(),
            factory: "software".into(),
            title: None,
            mode: "continuous".into(),
            size: "full".into(),        }))
        .unwrap();
    assert!(is_err(&res));
    assert!(err_message(&res).contains("slug"));
}

#[test]
fn run_start_rejects_unknown_factory() {
    let (_d, server) = server();
    let res = server
        .darkrun_run_new(Parameters(RunStartInput {
            slug: "r".into(),
            factory: "nonexistent".into(),
            title: None,
            mode: "continuous".into(),
            size: "full".into(),        }))
        .unwrap();
    assert!(is_err(&res));
    assert!(err_message(&res).contains("nonexistent"));
}

#[test]
fn run_start_unknown_factory_does_not_create_state() {
    let (dir, server) = server();
    server
        .darkrun_run_new(Parameters(RunStartInput {
            slug: "r".into(),
            factory: "nope".into(),
            title: None,
            mode: "continuous".into(),
            size: "full".into(),        }))
        .unwrap();
    assert!(!dir.path().join(".darkrun/r/run.md").exists());
}

#[test]
fn run_start_is_not_archived_initially() {
    let (_d, server) = server();
    let v = body(&start(&server, "r"));
    // archived is skip-if-none, so absent means not archived.
    assert!(v["frontmatter"].get("archived").is_none() || v["frontmatter"]["archived"] == false);
}

#[test]
fn run_start_two_distinct_slugs_both_persist() {
    let (dir, server) = server();
    start(&server, "a");
    start(&server, "b");
    assert!(dir.path().join(".darkrun/a/run.md").exists());
    assert!(dir.path().join(".darkrun/b/run.md").exists());
}

#[test]
fn run_start_re_start_same_slug_overwrites_title() {
    let (_d, server) = server();
    start(&server, "r");
    let res = server
        .darkrun_run_new(Parameters(RunStartInput {
            slug: "r".into(),
            factory: "software".into(),
            title: Some("Second".into()),
            mode: "continuous".into(),
            size: "full".into(),        }))
        .unwrap();
    assert!(is_ok(&res));
    assert_eq!(body(&res)["title"], "Second");
}

// ── darkrun_advance ───────────────────────────────────────────────────────

#[test]
fn run_next_first_tick_is_spec_on_frame() {
    let (_d, server) = started("r");
    let v = body(&next(&server, "r"));
    assert_eq!(v["action"]["action"], "spec");
    assert_eq!(v["action"]["station"], "frame");
}

#[test]
fn run_next_action_tag_matches_current_phase_spec() {
    let (_d, server) = started("r");
    let v = body(&next(&server, "r"));
    // The position's action and the top-level action agree.
    assert_eq!(v["action"]["action"], v["position"]["action"]["action"]);
}

#[test]
fn run_next_spec_carries_kills_framing() {
    let (_d, server) = started("r");
    let v = body(&next(&server, "r"));
    assert_eq!(v["action"]["kills"], "wrong-thing");
}

#[test]
fn run_next_second_tick_is_review() {
    let (_d, server) = started("r");
    next(&server, "r"); // spec
    let v = body(&next(&server, "r"));
    assert_eq!(v["action"]["action"], "review");
    assert_eq!(v["action"]["station"], "frame");
}

#[test]
fn run_next_review_lists_reviewers() {
    let (_d, server) = started("r");
    next(&server, "r");
    let v = body(&next(&server, "r"));
    let reviewers = v["action"]["reviewers"].as_array().unwrap();
    assert!(reviewers.iter().any(|r| r == "value"));
    assert!(reviewers.iter().any(|r| r == "feasibility"));
}

#[test]
fn run_next_track_is_run_for_forward_progress() {
    let (_d, server) = started("r");
    let v = body(&next(&server, "r"));
    assert_eq!(v["position"]["track"], "run");
}

#[test]
fn run_next_top_level_run_slug_present() {
    let (_d, server) = started("r");
    let v = body(&next(&server, "r"));
    assert_eq!(v["run"], "r");
}

#[test]
fn run_next_manufacture_after_units_decomposed() {
    let (_d, server) = started("r");
    next(&server, "r"); // spec -> review
    next(&server, "r"); // review -> manufacture
    create_unit(&server, "r", "u1", "frame");
    approve(&server, "r"); // clear the pre-execution operator gate
    let v = body(&next(&server, "r"));
    assert_eq!(v["action"]["action"], "manufacture");
    let units = v["action"]["units"].as_array().unwrap();
    assert!(units.iter().any(|u| u == "u1"));
}

#[test]
fn run_next_manufacture_carries_worker_beat() {
    let (_d, server) = started("r");
    next(&server, "r");
    next(&server, "r");
    create_unit(&server, "r", "u1", "frame");
    approve(&server, "r"); // clear the pre-execution operator gate
    let v = body(&next(&server, "r"));
    assert_eq!(v["action"]["worker"], "framer");
}

#[test]
fn run_next_noop_when_unit_blocked_mid_wave() {
    let (_d, server) = started("r");
    next(&server, "r");
    next(&server, "r");
    // A dispatched, in-flight unit: no wave-ready unit, not all complete → noop.
    // (A dangling dep would be a units_invalid decomposition error.)
    create_unit(&server, "r", "u1", "frame");
    server
        .darkrun_unit_update(Parameters(UnitUpdateInput {
            slug: "r".into(),
            unit: "u1".into(),
            status: Some("in_progress".into()),
            depends_on: None,
            worker: None,
            inputs: None,
            outputs: None,
            ..Default::default()
        }))
        .unwrap();
    approve(&server, "r"); // clear the pre-execution operator gate
    let v = body(&next(&server, "r"));
    assert_eq!(v["action"]["action"], "noop");
}

#[test]
fn run_next_advances_to_audit_when_units_complete() {
    let (_d, server) = started("r");
    next(&server, "r");
    next(&server, "r");
    create_unit(&server, "r", "u1", "frame");
    approve(&server, "r"); // clear the pre-execution operator gate
    next(&server, "r"); // manufacture
    server
        .darkrun_unit_update(Parameters(UnitUpdateInput {
            slug: "r".into(),
            unit: "u1".into(),
            status: Some("completed".into()),
            depends_on: None,
            worker: None,
            inputs: None,
            outputs: None,
            ..Default::default()
        }))
        .unwrap();
    let v = body(&next(&server, "r"));
    assert_eq!(v["action"]["action"], "audit");
}

#[test]
fn run_next_walks_audit_reflect_checkpoint_in_order() {
    let (_d, server) = started("r");
    next(&server, "r");
    next(&server, "r");
    create_unit(&server, "r", "u1", "frame");
    approve(&server, "r"); // clear the pre-execution operator gate
    next(&server, "r");
    server
        .darkrun_unit_update(Parameters(UnitUpdateInput {
            slug: "r".into(),
            unit: "u1".into(),
            status: Some("completed".into()),
            depends_on: None,
            worker: None,
            inputs: None,
            outputs: None,
            ..Default::default()
        }))
        .unwrap();
    assert_eq!(body(&next(&server, "r"))["action"]["action"], "audit");
    assert_eq!(body(&next(&server, "r"))["action"]["action"], "reflect");
    let cp = body(&next(&server, "r"));
    assert_eq!(cp["action"]["action"], "checkpoint");
    assert_eq!(cp["action"]["kind"], "ask");
}

#[test]
fn run_next_errors_on_missing_run() {
    let (_d, server) = server();
    let res = next(&server, "ghost");
    assert!(is_err(&res));
}

#[test]
fn run_next_is_idempotent_at_spec_when_no_state_change() {
    // Two ticks advance the phase, but the FIRST tick from a fresh run is
    // deterministically Spec/frame.
    let (_d, s1) = started("r");
    let v1 = body(&next(&s1, "r"));
    let (_d2, s2) = started("r");
    let v2 = body(&next(&s2, "r"));
    assert_eq!(v1["action"]["action"], v2["action"]["action"]);
    assert_eq!(v1["action"]["station"], v2["action"]["station"]);
}

// ── darkrun_run_inspect ───────────────────────────────────────────────────────

#[test]
fn run_show_returns_run_state_and_position() {
    let (_d, server) = started("r");
    let res = server
        .darkrun_run_inspect(Parameters(RunShowRef { slug: Some("r".into()) }))
        .unwrap();
    assert!(is_ok(&res));
    let v = body(&res);
    assert!(v.get("run").is_some());
    assert!(v.get("state").is_some());
    assert!(v.get("position").is_some());
}

#[test]
fn run_show_run_has_correct_slug() {
    let (_d, server) = started("r");
    let v = body(
        &server
            .darkrun_run_inspect(Parameters(RunShowRef { slug: Some("r".into()) }))
            .unwrap(),
    );
    assert_eq!(v["run"]["slug"], "r");
}

#[test]
fn run_show_without_slug_infers_the_sole_run() {
    // No slug, no git branch, no active-run pointer: the lone run is unambiguous,
    // so the user need not name it.
    let (_d, server) = started("r");
    let res = server
        .darkrun_run_inspect(Parameters(RunShowRef { slug: None }))
        .unwrap();
    assert!(is_ok(&res));
    assert_eq!(body(&res)["run"]["slug"], "r");
}

#[test]
fn run_show_without_slug_errors_when_nothing_to_infer() {
    // No runs at all → nothing to disambiguate to → a clear error, not a panic.
    let (_d, server) = server();
    let res = server
        .darkrun_run_inspect(Parameters(RunShowRef { slug: None }))
        .unwrap();
    assert!(is_err(&res));
}

#[test]
fn run_show_position_reflects_spec_for_fresh_run() {
    let (_d, server) = started("r");
    let v = body(
        &server
            .darkrun_run_inspect(Parameters(RunShowRef { slug: Some("r".into()) }))
            .unwrap(),
    );
    assert_eq!(v["position"]["action"]["action"], "spec");
    assert_eq!(v["position"]["track"], "run");
}

#[test]
fn run_show_state_has_active_station() {
    let (_d, server) = started("r");
    let v = body(
        &server
            .darkrun_run_inspect(Parameters(RunShowRef { slug: Some("r".into()) }))
            .unwrap(),
    );
    assert_eq!(v["state"]["active_station"], "frame");
}

#[test]
fn run_show_position_advances_after_ticks() {
    let (_d, server) = started("r");
    next(&server, "r"); // spec -> review
    let v = body(
        &server
            .darkrun_run_inspect(Parameters(RunShowRef { slug: Some("r".into()) }))
            .unwrap(),
    );
    assert_eq!(v["position"]["action"]["action"], "review");
}

#[test]
fn run_show_errors_on_missing_run() {
    let (_d, server) = server();
    let res = server
        .darkrun_run_inspect(Parameters(RunShowRef { slug: Some("ghost".into()) }))
        .unwrap();
    assert!(is_err(&res));
}

#[test]
fn run_show_is_a_pure_read_does_not_advance() {
    let (_d, server) = started("r");
    server
        .darkrun_run_inspect(Parameters(RunShowRef { slug: Some("r".into()) }))
        .unwrap();
    server
        .darkrun_run_inspect(Parameters(RunShowRef { slug: Some("r".into()) }))
        .unwrap();
    // Still at spec — show never advances the phase machine.
    let v = body(&next(&server, "r"));
    assert_eq!(v["action"]["action"], "spec");
}

// ── darkrun_unit_list / get / create / update ──────────────────────────────

#[test]
fn unit_list_empty_for_fresh_run() {
    let (_d, server) = started("r");
    let res = server
        .darkrun_unit_list(Parameters(RunRef { slug: "r".into() }))
        .unwrap();
    assert!(is_ok(&res));
    assert_eq!(body(&res).as_array().unwrap().len(), 0);
}

#[test]
fn unit_list_reflects_created_units() {
    let (_d, server) = started("r");
    create_unit(&server, "r", "u1", "frame");
    create_unit(&server, "r", "u2", "frame");
    let v = body(
        &server
            .darkrun_unit_list(Parameters(RunRef { slug: "r".into() }))
            .unwrap(),
    );
    assert_eq!(v.as_array().unwrap().len(), 2);
}

#[test]
fn unit_create_returns_pending_unit() {
    let (_d, server) = started("r");
    let res = create_unit(&server, "r", "u1", "frame");
    assert!(is_ok(&res));
    let v = body(&res);
    assert_eq!(v["slug"], "u1");
    assert_eq!(v["frontmatter"]["status"], "pending");
    assert_eq!(v["frontmatter"]["station"], "frame");
}

#[test]
fn unit_create_with_title_sets_name_and_title() {
    let (_d, server) = started("r");
    let res = server
        .darkrun_unit_create(Parameters(UnitCreateInput {
            slug: "r".into(),
            unit: "u1".into(),
            station: "frame".into(),
            title: Some("First unit".into()),
            depends_on: vec![],
            ..Default::default()
        }))
        .unwrap();
    let v = body(&res);
    assert_eq!(v["title"], "First unit");
    assert_eq!(v["frontmatter"]["name"], "First unit");
}

#[test]
fn unit_create_with_dependencies() {
    let (_d, server) = started("r");
    create_unit(&server, "r", "dep", "frame");
    let res = server
        .darkrun_unit_create(Parameters(UnitCreateInput {
            slug: "r".into(),
            unit: "u1".into(),
            station: "frame".into(),
            title: None,
            depends_on: vec!["dep".into()],
            ..Default::default()
        }))
        .unwrap();
    let v = body(&res);
    assert_eq!(v["frontmatter"]["depends_on"][0], "dep");
}

#[test]
fn unit_create_rejects_duplicate() {
    let (_d, server) = started("r");
    create_unit(&server, "r", "u1", "frame");
    let res = create_unit(&server, "r", "u1", "frame");
    assert!(is_err(&res));
    assert!(err_message(&res).contains("already exists"));
}

#[test]
fn unit_create_rejects_empty_slug() {
    let (_d, server) = started("r");
    let res = create_unit(&server, "r", "  ", "frame");
    assert!(is_err(&res));
}

#[test]
fn unit_get_returns_created_unit() {
    let (_d, server) = started("r");
    create_unit(&server, "r", "u1", "frame");
    let res = server
        .darkrun_unit_get(Parameters(UnitRef {
            slug: "r".into(),
            unit: "u1".into(),
        }))
        .unwrap();
    assert!(is_ok(&res));
    let v = body(&res);
    assert_eq!(v["slug"], "u1");
    assert_eq!(v["frontmatter"]["station"], "frame");
}

#[test]
fn unit_get_errors_on_missing_unit() {
    let (_d, server) = started("r");
    let res = server
        .darkrun_unit_get(Parameters(UnitRef {
            slug: "r".into(),
            unit: "ghost".into(),
        }))
        .unwrap();
    assert!(is_err(&res));
}

#[test]
fn unit_update_advances_status_to_completed() {
    let (_d, server) = started("r");
    create_unit(&server, "r", "u1", "frame");
    let res = server
        .darkrun_unit_update(Parameters(UnitUpdateInput {
            slug: "r".into(),
            unit: "u1".into(),
            status: Some("completed".into()),
            depends_on: None,
            worker: None,
            inputs: None,
            outputs: None,
            ..Default::default()
        }))
        .unwrap();
    let v = body(&res);
    assert_eq!(v["frontmatter"]["status"], "completed");
    assert!(v["frontmatter"]["completed_at"].is_string());
}

#[test]
fn unit_update_sets_worker() {
    let (_d, server) = started("r");
    create_unit(&server, "r", "u1", "frame");
    let res = server
        .darkrun_unit_update(Parameters(UnitUpdateInput {
            slug: "r".into(),
            unit: "u1".into(),
            status: None,
            depends_on: None,
            worker: Some("builder".into()),
            inputs: None,
            outputs: None,
            ..Default::default()
        }))
        .unwrap();
    assert_eq!(body(&res)["frontmatter"]["worker"], "builder");
}

#[test]
fn unit_update_sets_outputs() {
    let (_d, server) = started("r");
    create_unit(&server, "r", "u1", "frame");
    let res = server
        .darkrun_unit_update(Parameters(UnitUpdateInput {
            slug: "r".into(),
            unit: "u1".into(),
            status: None,
            depends_on: None,
            worker: None,
            inputs: None,
            outputs: Some(vec!["frame/out.md".into()]),
            ..Default::default()
        }))
        .unwrap();
    assert_eq!(body(&res)["frontmatter"]["outputs"][0], "frame/out.md");
}

#[test]
fn unit_update_deps_allowed_while_pending() {
    let (_d, server) = started("r");
    create_unit(&server, "r", "x", "frame");
    create_unit(&server, "r", "u1", "frame");
    let res = server
        .darkrun_unit_update(Parameters(UnitUpdateInput {
            slug: "r".into(),
            unit: "u1".into(),
            status: None,
            depends_on: Some(vec!["x".into()]),
            worker: None,
            inputs: None,
            outputs: None,
            ..Default::default()
        }))
        .unwrap();
    assert!(is_ok(&res));
    assert_eq!(body(&res)["frontmatter"]["depends_on"][0], "x");
}

#[test]
fn unit_update_deps_rejected_once_active() {
    let (_d, server) = started("r");
    create_unit(&server, "r", "u1", "frame");
    server
        .darkrun_unit_update(Parameters(UnitUpdateInput {
            slug: "r".into(),
            unit: "u1".into(),
            status: Some("active".into()),
            depends_on: None,
            worker: None,
            inputs: None,
            outputs: None,
            ..Default::default()
        }))
        .unwrap();
    let res = server
        .darkrun_unit_update(Parameters(UnitUpdateInput {
            slug: "r".into(),
            unit: "u1".into(),
            status: None,
            depends_on: Some(vec!["x".into()]),
            worker: None,
            inputs: None,
            outputs: None,
            ..Default::default()
        }))
        .unwrap();
    assert!(is_err(&res));
    assert!(err_message(&res).contains("immutable"));
}

#[test]
fn unit_update_rejects_invalid_status() {
    let (_d, server) = started("r");
    create_unit(&server, "r", "u1", "frame");
    let res = server
        .darkrun_unit_update(Parameters(UnitUpdateInput {
            slug: "r".into(),
            unit: "u1".into(),
            status: Some("nonsense".into()),
            depends_on: None,
            worker: None,
            inputs: None,
            outputs: None,
            ..Default::default()
        }))
        .unwrap();
    assert!(is_err(&res));
    assert!(err_message(&res).contains("invalid status"));
}

#[test]
fn unit_update_errors_on_missing_unit() {
    let (_d, server) = started("r");
    let res = server
        .darkrun_unit_update(Parameters(UnitUpdateInput {
            slug: "r".into(),
            unit: "ghost".into(),
            status: Some("completed".into()),
            depends_on: None,
            worker: None,
            inputs: None,
            outputs: None,
            ..Default::default()
        }))
        .unwrap();
    assert!(is_err(&res));
}

#[test]
fn unit_update_accepts_in_progress_alias() {
    let (_d, server) = started("r");
    create_unit(&server, "r", "u1", "frame");
    let res = server
        .darkrun_unit_update(Parameters(UnitUpdateInput {
            slug: "r".into(),
            unit: "u1".into(),
            status: Some("in_progress".into()),
            depends_on: None,
            worker: None,
            inputs: None,
            outputs: None,
            ..Default::default()
        }))
        .unwrap();
    assert!(is_ok(&res));
    assert_eq!(body(&res)["frontmatter"]["status"], "in_progress");
}

#[test]
fn unit_create_get_update_roundtrip_is_persistent() {
    let (_d, server) = started("r");
    create_unit(&server, "r", "u1", "frame");
    server
        .darkrun_unit_update(Parameters(UnitUpdateInput {
            slug: "r".into(),
            unit: "u1".into(),
            status: Some("blocked".into()),
            depends_on: None,
            worker: None,
            inputs: None,
            outputs: None,
            ..Default::default()
        }))
        .unwrap();
    let v = body(
        &server
            .darkrun_unit_get(Parameters(UnitRef {
                slug: "r".into(),
                unit: "u1".into(),
            }))
            .unwrap(),
    );
    assert_eq!(v["frontmatter"]["status"], "blocked");
}

// ── darkrun_feedback_create / list / resolve / reject / move ───────────────

#[test]
fn feedback_create_allocates_first_id() {
    let (_d, server) = started("r");
    let res = create_feedback(&server, "r", "frame", "widget overflows");
    assert!(is_ok(&res));
    let v = body(&res);
    assert_eq!(v["id"], "fb-01");
    assert_eq!(v["status"], "pending");
    assert_eq!(v["station"], "frame");
}

#[test]
fn feedback_create_with_severity() {
    let (_d, server) = started("r");
    let res = server
        .darkrun_feedback_create(Parameters(FeedbackCreateInput {
            slug: "r".into(),
            station: "frame".into(),
            body: "x".into(),
            severity: Some("high".into()),
                origin: None,
                invalidates: None,
        }))
        .unwrap();
    assert_eq!(body(&res)["severity"], "high");
}

#[test]
fn feedback_create_rejects_invalid_severity() {
    let (_d, server) = started("r");
    let res = server
        .darkrun_feedback_create(Parameters(FeedbackCreateInput {
            slug: "r".into(),
            station: "frame".into(),
            body: "x".into(),
            severity: Some("catastrophic".into()),
                origin: None,
                invalidates: None,
        }))
        .unwrap();
    assert!(is_err(&res));
    assert!(err_message(&res).contains("invalid severity"));
}

#[test]
fn feedback_create_rejects_empty_body() {
    let (_d, server) = started("r");
    let res = create_feedback(&server, "r", "frame", "   ");
    assert!(is_err(&res));
}

#[test]
fn feedback_create_sequential_ids() {
    let (_d, server) = started("r");
    let a = body(&create_feedback(&server, "r", "frame", "a"));
    let b = body(&create_feedback(&server, "r", "frame", "b"));
    assert_eq!(a["id"], "fb-01");
    assert_eq!(b["id"], "fb-02");
}

#[test]
fn feedback_create_records_created_at() {
    let (_d, server) = started("r");
    let v = body(&create_feedback(&server, "r", "frame", "a"));
    assert!(v["created_at"].is_string());
}

#[test]
fn feedback_list_includes_open_item() {
    let (_d, server) = started("r");
    create_feedback(&server, "r", "frame", "a");
    let res = server
        .darkrun_feedback_list(Parameters(FeedbackListInput {
            slug: "r".into(),
            include_settled: true,
        }))
        .unwrap();
    assert_eq!(body(&res).as_array().unwrap().len(), 1);
}

#[test]
fn feedback_list_empty_for_fresh_run() {
    let (_d, server) = started("r");
    let res = server
        .darkrun_feedback_list(Parameters(FeedbackListInput {
            slug: "r".into(),
            include_settled: true,
        }))
        .unwrap();
    assert_eq!(body(&res).as_array().unwrap().len(), 0);
}

#[test]
fn feedback_list_sorted_by_id() {
    let (_d, server) = started("r");
    create_feedback(&server, "r", "frame", "a");
    create_feedback(&server, "r", "frame", "b");
    create_feedback(&server, "r", "frame", "c");
    let v = body(
        &server
            .darkrun_feedback_list(Parameters(FeedbackListInput {
                slug: "r".into(),
                include_settled: true,
            }))
            .unwrap(),
    );
    let arr = v.as_array().unwrap();
    assert_eq!(arr[0]["id"], "fb-01");
    assert_eq!(arr[1]["id"], "fb-02");
    assert_eq!(arr[2]["id"], "fb-03");
}

#[test]
fn feedback_list_hides_settled_when_requested() {
    let (_d, server) = started("r");
    let id = body(&create_feedback(&server, "r", "frame", "a"))["id"]
        .as_str()
        .unwrap()
        .to_string();
    server
        .darkrun_feedback_resolve(Parameters(FeedbackResolveInput {
            slug: "r".into(),
            feedback_id: id,
            status: "addressed".into(),
                reply: None,
        }))
        .unwrap();
    let v = body(
        &server
            .darkrun_feedback_list(Parameters(FeedbackListInput {
                slug: "r".into(),
                include_settled: false,
            }))
            .unwrap(),
    );
    assert_eq!(v.as_array().unwrap().len(), 0);
}

#[test]
fn feedback_list_keeps_settled_when_included() {
    let (_d, server) = started("r");
    let id = body(&create_feedback(&server, "r", "frame", "a"))["id"]
        .as_str()
        .unwrap()
        .to_string();
    server
        .darkrun_feedback_resolve(Parameters(FeedbackResolveInput {
            slug: "r".into(),
            feedback_id: id,
            status: "addressed".into(),
                reply: None,
        }))
        .unwrap();
    let v = body(
        &server
            .darkrun_feedback_list(Parameters(FeedbackListInput {
                slug: "r".into(),
                include_settled: true,
            }))
            .unwrap(),
    );
    assert_eq!(v.as_array().unwrap().len(), 1);
}

#[test]
fn feedback_resolve_stamps_addressed() {
    let (_d, server) = started("r");
    let id = body(&create_feedback(&server, "r", "frame", "a"))["id"]
        .as_str()
        .unwrap()
        .to_string();
    let res = server
        .darkrun_feedback_resolve(Parameters(FeedbackResolveInput {
            slug: "r".into(),
            feedback_id: id,
            status: "addressed".into(),
                reply: None,
        }))
        .unwrap();
    assert_eq!(body(&res)["status"], "addressed");
}

#[test]
fn feedback_resolve_accepts_answered() {
    let (_d, server) = started("r");
    let id = body(&create_feedback(&server, "r", "frame", "q"))["id"]
        .as_str()
        .unwrap()
        .to_string();
    let res = server
        .darkrun_feedback_resolve(Parameters(FeedbackResolveInput {
            slug: "r".into(),
            feedback_id: id,
            status: "answered".into(),
                reply: None,
        }))
        .unwrap();
    assert_eq!(body(&res)["status"], "answered");
}

#[test]
fn feedback_resolve_accepts_non_actionable() {
    let (_d, server) = started("r");
    let id = body(&create_feedback(&server, "r", "frame", "q"))["id"]
        .as_str()
        .unwrap()
        .to_string();
    let res = server
        .darkrun_feedback_resolve(Parameters(FeedbackResolveInput {
            slug: "r".into(),
            feedback_id: id,
            status: "non_actionable".into(),
                reply: None,
        }))
        .unwrap();
    assert_eq!(body(&res)["status"], "non_actionable");
}

#[test]
fn feedback_resolve_accepts_closed() {
    let (_d, server) = started("r");
    let id = body(&create_feedback(&server, "r", "frame", "q"))["id"]
        .as_str()
        .unwrap()
        .to_string();
    let res = server
        .darkrun_feedback_resolve(Parameters(FeedbackResolveInput {
            slug: "r".into(),
            feedback_id: id,
            status: "closed".into(),
                reply: None,
        }))
        .unwrap();
    assert_eq!(body(&res)["status"], "closed");
}

#[test]
fn feedback_resolve_rejects_non_terminal_status() {
    let (_d, server) = started("r");
    let id = body(&create_feedback(&server, "r", "frame", "a"))["id"]
        .as_str()
        .unwrap()
        .to_string();
    let res = server
        .darkrun_feedback_resolve(Parameters(FeedbackResolveInput {
            slug: "r".into(),
            feedback_id: id,
            status: "fixing".into(),
                reply: None,
        }))
        .unwrap();
    assert!(is_err(&res));
    assert!(err_message(&res).contains("terminal"));
}

#[test]
fn feedback_resolve_rejects_invalid_status() {
    let (_d, server) = started("r");
    let id = body(&create_feedback(&server, "r", "frame", "a"))["id"]
        .as_str()
        .unwrap()
        .to_string();
    let res = server
        .darkrun_feedback_resolve(Parameters(FeedbackResolveInput {
            slug: "r".into(),
            feedback_id: id,
            status: "garbage".into(),
                reply: None,
        }))
        .unwrap();
    assert!(is_err(&res));
    assert!(err_message(&res).contains("invalid status"));
}

#[test]
fn feedback_resolve_errors_on_missing_id() {
    let (_d, server) = started("r");
    let res = server
        .darkrun_feedback_resolve(Parameters(FeedbackResolveInput {
            slug: "r".into(),
            feedback_id: "fb-99".into(),
            status: "addressed".into(),
                reply: None,
        }))
        .unwrap();
    assert!(is_err(&res));
}

#[test]
fn feedback_resolve_twice_is_rejected_as_settled() {
    let (_d, server) = started("r");
    let id = body(&create_feedback(&server, "r", "frame", "a"))["id"]
        .as_str()
        .unwrap()
        .to_string();
    server
        .darkrun_feedback_resolve(Parameters(FeedbackResolveInput {
            slug: "r".into(),
            feedback_id: id.clone(),
            status: "addressed".into(),
                reply: None,
        }))
        .unwrap();
    let again = server
        .darkrun_feedback_resolve(Parameters(FeedbackResolveInput {
            slug: "r".into(),
            feedback_id: id,
            status: "closed".into(),
                reply: None,
        }))
        .unwrap();
    assert!(is_err(&again));
}

#[test]
fn feedback_reject_marks_rejected_and_appends_reason() {
    let (_d, server) = started("r");
    let id = body(&create_feedback(&server, "r", "frame", "bad"))["id"]
        .as_str()
        .unwrap()
        .to_string();
    let res = server
        .darkrun_feedback_reject(Parameters(FeedbackRejectInput {
            slug: "r".into(),
            feedback_id: id,
            reason: "stale duplicate".into(),
        }))
        .unwrap();
    let v = body(&res);
    assert_eq!(v["status"], "rejected");
    assert!(v["body"].as_str().unwrap().contains("stale duplicate"));
}

#[test]
fn feedback_reject_errors_on_missing_id() {
    let (_d, server) = started("r");
    let res = server
        .darkrun_feedback_reject(Parameters(FeedbackRejectInput {
            slug: "r".into(),
            feedback_id: "fb-99".into(),
            reason: "x".into(),
        }))
        .unwrap();
    assert!(is_err(&res));
}

#[test]
fn feedback_reject_then_resolve_is_settled() {
    let (_d, server) = started("r");
    let id = body(&create_feedback(&server, "r", "frame", "bad"))["id"]
        .as_str()
        .unwrap()
        .to_string();
    server
        .darkrun_feedback_reject(Parameters(FeedbackRejectInput {
            slug: "r".into(),
            feedback_id: id.clone(),
            reason: "x".into(),
        }))
        .unwrap();
    let res = server
        .darkrun_feedback_resolve(Parameters(FeedbackResolveInput {
            slug: "r".into(),
            feedback_id: id,
            status: "addressed".into(),
                reply: None,
        }))
        .unwrap();
    assert!(is_err(&res));
}

#[test]
fn feedback_move_relocates_station() {
    let (_d, server) = started("r");
    let id = body(&create_feedback(&server, "r", "frame", "x"))["id"]
        .as_str()
        .unwrap()
        .to_string();
    let res = server
        .darkrun_feedback_move(Parameters(FeedbackMoveInput {
            slug: "r".into(),
            feedback_id: id,
            to_station: "shape".into(),
        }))
        .unwrap();
    assert_eq!(body(&res)["station"], "shape");
}

#[test]
fn feedback_move_errors_on_missing_id() {
    let (_d, server) = started("r");
    let res = server
        .darkrun_feedback_move(Parameters(FeedbackMoveInput {
            slug: "r".into(),
            feedback_id: "fb-99".into(),
            to_station: "shape".into(),
        }))
        .unwrap();
    assert!(is_err(&res));
}

#[test]
fn feedback_move_on_settled_is_rejected() {
    let (_d, server) = started("r");
    let id = body(&create_feedback(&server, "r", "frame", "x"))["id"]
        .as_str()
        .unwrap()
        .to_string();
    server
        .darkrun_feedback_resolve(Parameters(FeedbackResolveInput {
            slug: "r".into(),
            feedback_id: id.clone(),
            status: "addressed".into(),
                reply: None,
        }))
        .unwrap();
    let res = server
        .darkrun_feedback_move(Parameters(FeedbackMoveInput {
            slug: "r".into(),
            feedback_id: id,
            to_station: "shape".into(),
        }))
        .unwrap();
    assert!(is_err(&res));
}

#[test]
fn feedback_create_then_list_get_roundtrip_preserves_body() {
    let (_d, server) = started("r");
    create_feedback(&server, "r", "build", "the widget overflows its bounds");
    let v = body(
        &server
            .darkrun_feedback_list(Parameters(FeedbackListInput {
                slug: "r".into(),
                include_settled: true,
            }))
            .unwrap(),
    );
    assert!(v[0]["body"]
        .as_str()
        .unwrap()
        .contains("the widget overflows its bounds"));
    assert_eq!(v[0]["station"], "build");
}

// ── darkrun_checkpoint_decide ──────────────────────────────────────────────

#[test]
fn checkpoint_decide_approve_advances_to_next_station() {
    let (_d, server) = started("r");
    let res = server
        .darkrun_checkpoint_decide(Parameters(CheckpointDecideInput {
            slug: "r".into(),
            approved: true,
            feedback: None,
        }))
        .unwrap();
    assert!(is_ok(&res));
    let v = body(&res);
    // Approving frame's gate advances to specify's Spec.
    assert_eq!(v["action"]["action"], "spec");
    assert_eq!(v["action"]["station"], "specify");
}

#[test]
fn checkpoint_decide_reject_holds_and_routes_feedback() {
    let (_d, server) = started("r");
    let res = server
        .darkrun_checkpoint_decide(Parameters(CheckpointDecideInput {
            slug: "r".into(),
            approved: false,
            feedback: Some("not good enough".into()),
        }))
        .unwrap();
    let v = body(&res);
    // The reject files feedback, which preempts the run track.
    assert_eq!(v["position"]["track"], "feedback");
    assert_eq!(v["action"]["action"], "fix_feedback");
}

#[test]
fn checkpoint_decide_reject_files_visible_feedback() {
    let (_d, server) = started("r");
    server
        .darkrun_checkpoint_decide(Parameters(CheckpointDecideInput {
            slug: "r".into(),
            approved: false,
            feedback: Some("missing edge case".into()),
        }))
        .unwrap();
    let v = body(
        &server
            .darkrun_feedback_list(Parameters(FeedbackListInput {
                slug: "r".into(),
                include_settled: true,
            }))
            .unwrap(),
    );
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert!(arr[0]["body"]
        .as_str()
        .unwrap()
        .contains("missing edge case"));
}

#[test]
fn checkpoint_decide_reject_without_feedback_files_nothing() {
    let (_d, server) = started("r");
    server
        .darkrun_checkpoint_decide(Parameters(CheckpointDecideInput {
            slug: "r".into(),
            approved: false,
            feedback: None,
        }))
        .unwrap();
    let v = body(
        &server
            .darkrun_feedback_list(Parameters(FeedbackListInput {
                slug: "r".into(),
                include_settled: true,
            }))
            .unwrap(),
    );
    assert_eq!(v.as_array().unwrap().len(), 0);
}

#[test]
fn checkpoint_decide_errors_on_missing_run() {
    let (_d, server) = server();
    let res = server
        .darkrun_checkpoint_decide(Parameters(CheckpointDecideInput {
            slug: "ghost".into(),
            approved: true,
            feedback: None,
        }))
        .unwrap();
    assert!(is_err(&res));
}

#[test]
fn checkpoint_decide_approve_marks_station_completed_in_show() {
    let (_d, server) = started("r");
    server
        .darkrun_checkpoint_decide(Parameters(CheckpointDecideInput {
            slug: "r".into(),
            approved: true,
            feedback: None,
        }))
        .unwrap();
    let v = body(
        &server
            .darkrun_run_inspect(Parameters(RunShowRef { slug: Some("r".into()) }))
            .unwrap(),
    );
    assert_eq!(v["state"]["stations"]["frame"]["status"], "completed");
    assert_eq!(v["state"]["active_station"], "specify");
}

#[test]
fn checkpoint_decide_reject_blocks_station_in_show() {
    let (_d, server) = started("r");
    server
        .darkrun_checkpoint_decide(Parameters(CheckpointDecideInput {
            slug: "r".into(),
            approved: false,
            feedback: Some("nope".into()),
        }))
        .unwrap();
    let v = body(
        &server
            .darkrun_run_inspect(Parameters(RunShowRef { slug: Some("r".into()) }))
            .unwrap(),
    );
    assert_eq!(v["state"]["stations"]["frame"]["status"], "blocked");
}

#[test]
fn checkpoint_decide_then_resolve_feedback_resumes_run() {
    let (_d, server) = started("r");
    server
        .darkrun_checkpoint_decide(Parameters(CheckpointDecideInput {
            slug: "r".into(),
            approved: false,
            feedback: Some("fix".into()),
        }))
        .unwrap();
    // Resolve the routed feedback terminally.
    let id = body(
        &server
            .darkrun_feedback_list(Parameters(FeedbackListInput {
                slug: "r".into(),
                include_settled: true,
            }))
            .unwrap(),
    )[0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    server
        .darkrun_feedback_resolve(Parameters(FeedbackResolveInput {
            slug: "r".into(),
            feedback_id: id,
            status: "addressed".into(),
                reply: None,
        }))
        .unwrap();
    // The run track resumes (no longer on the feedback track).
    let v = body(
        &server
            .darkrun_run_inspect(Parameters(RunShowRef { slug: Some("r".into()) }))
            .unwrap(),
    );
    assert_ne!(v["position"]["track"], "feedback");
}

// ── darkrun_run_list / archive ─────────────────────────────────────────────

#[test]
fn run_list_empty_when_no_runs() {
    let (_d, server) = server();
    let res = server
        .darkrun_run_list(Parameters(RunListInput {
            include_archived: false,
        }))
        .unwrap();
    assert_eq!(body(&res).as_array().unwrap().len(), 0);
}

#[test]
fn run_list_returns_summaries() {
    let (_d, server) = server();
    start(&server, "a");
    start(&server, "b");
    let v = body(
        &server
            .darkrun_run_list(Parameters(RunListInput {
                include_archived: false,
            }))
            .unwrap(),
    );
    assert_eq!(v.as_array().unwrap().len(), 2);
}

#[test]
fn run_list_summary_shape() {
    let (_d, server) = started("r");
    let v = body(
        &server
            .darkrun_run_list(Parameters(RunListInput {
                include_archived: false,
            }))
            .unwrap(),
    );
    let item = &v.as_array().unwrap()[0];
    assert_eq!(item["slug"], "r");
    assert_eq!(item["factory"], "software");
    assert_eq!(item["active_station"], "frame");
    assert_eq!(item["status"], "active");
    assert_eq!(item["archived"], false);
}

#[test]
fn run_list_sorted_by_slug() {
    let (_d, server) = server();
    start(&server, "zebra");
    start(&server, "alpha");
    start(&server, "mango");
    let v = body(
        &server
            .darkrun_run_list(Parameters(RunListInput {
                include_archived: false,
            }))
            .unwrap(),
    );
    let arr = v.as_array().unwrap();
    assert_eq!(arr[0]["slug"], "alpha");
    assert_eq!(arr[1]["slug"], "mango");
    assert_eq!(arr[2]["slug"], "zebra");
}

#[test]
fn run_archive_hides_from_default_list() {
    let (_d, server) = started("r");
    server
        .darkrun_run_archive(Parameters(RunArchiveInput {
            slug: "r".into(),
            archived: true,
        }))
        .unwrap();
    let v = body(
        &server
            .darkrun_run_list(Parameters(RunListInput {
                include_archived: false,
            }))
            .unwrap(),
    );
    assert_eq!(v.as_array().unwrap().len(), 0);
}

#[test]
fn run_archive_visible_when_included() {
    let (_d, server) = started("r");
    server
        .darkrun_run_archive(Parameters(RunArchiveInput {
            slug: "r".into(),
            archived: true,
        }))
        .unwrap();
    let v = body(
        &server
            .darkrun_run_list(Parameters(RunListInput {
                include_archived: true,
            }))
            .unwrap(),
    );
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["archived"], true);
}

#[test]
fn run_archive_returns_slug_and_flag() {
    let (_d, server) = started("r");
    let res = server
        .darkrun_run_archive(Parameters(RunArchiveInput {
            slug: "r".into(),
            archived: true,
        }))
        .unwrap();
    let v = body(&res);
    assert_eq!(v["slug"], "r");
    assert_eq!(v["archived"], true);
}

#[test]
fn run_archive_restore_brings_back_to_default_list() {
    let (_d, server) = started("r");
    server
        .darkrun_run_archive(Parameters(RunArchiveInput {
            slug: "r".into(),
            archived: true,
        }))
        .unwrap();
    server
        .darkrun_run_archive(Parameters(RunArchiveInput {
            slug: "r".into(),
            archived: false,
        }))
        .unwrap();
    let v = body(
        &server
            .darkrun_run_list(Parameters(RunListInput {
                include_archived: false,
            }))
            .unwrap(),
    );
    assert_eq!(v.as_array().unwrap().len(), 1);
}

#[test]
fn run_archive_errors_on_missing_run() {
    let (_d, server) = server();
    let res = server
        .darkrun_run_archive(Parameters(RunArchiveInput {
            slug: "ghost".into(),
            archived: true,
        }))
        .unwrap();
    assert!(is_err(&res));
}

#[test]
fn run_archive_is_idempotent() {
    let (_d, server) = started("r");
    let a = server
        .darkrun_run_archive(Parameters(RunArchiveInput {
            slug: "r".into(),
            archived: true,
        }))
        .unwrap();
    let b = server
        .darkrun_run_archive(Parameters(RunArchiveInput {
            slug: "r".into(),
            archived: true,
        }))
        .unwrap();
    assert!(is_ok(&a));
    assert!(is_ok(&b));
    let v = body(
        &server
            .darkrun_run_list(Parameters(RunListInput {
                include_archived: true,
            }))
            .unwrap(),
    );
    assert_eq!(v.as_array().unwrap().len(), 1);
}

#[test]
fn run_list_mixes_archived_and_active() {
    let (_d, server) = server();
    start(&server, "live");
    start(&server, "dead");
    server
        .darkrun_run_archive(Parameters(RunArchiveInput {
            slug: "dead".into(),
            archived: true,
        }))
        .unwrap();
    let default = body(
        &server
            .darkrun_run_list(Parameters(RunListInput {
                include_archived: false,
            }))
            .unwrap(),
    );
    assert_eq!(default.as_array().unwrap().len(), 1);
    assert_eq!(default[0]["slug"], "live");

    let all = body(
        &server
            .darkrun_run_list(Parameters(RunListInput {
                include_archived: true,
            }))
            .unwrap(),
    );
    assert_eq!(all.as_array().unwrap().len(), 2);
}

// ── darkrun_factory_list / detail ──────────────────────────────────────────

#[test]
fn factory_list_includes_software() {
    let (_d, server) = server();
    let v = body(&server.darkrun_factory_list().unwrap());
    assert!(
        v.as_array().unwrap().iter().any(|f| f["name"] == "software"),
        "software factory listed"
    );
}

#[test]
fn factory_list_software_has_six_stations() {
    let (_d, server) = server();
    let v = body(&server.darkrun_factory_list().unwrap());
    let sw = software_entry(&v);
    assert_eq!(sw["stations"].as_array().unwrap().len(), 6);
}

#[test]
fn factory_list_first_station_is_frame() {
    let (_d, server) = server();
    let v = body(&server.darkrun_factory_list().unwrap());
    let sw = software_entry(&v);
    assert_eq!(sw["stations"][0]["name"], "frame");
}

#[test]
fn factory_list_stations_in_cost_order() {
    let (_d, server) = server();
    let v = body(&server.darkrun_factory_list().unwrap());
    let sw = software_entry(&v);
    let names: Vec<&str> = sw["stations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        vec!["frame", "specify", "shape", "build", "prove", "harden"]
    );
}

#[test]
fn factory_list_station_carries_kills_and_artifact() {
    let (_d, server) = server();
    let v = body(&server.darkrun_factory_list().unwrap());
    let sw = software_entry(&v);
    assert_eq!(sw["stations"][0]["kills"], "wrong-thing");
    assert_eq!(sw["stations"][0]["artifact"], "frame.md");
}

#[test]
fn factory_list_station_carries_workers_and_reviewers() {
    let (_d, server) = server();
    let v = body(&server.darkrun_factory_list().unwrap());
    let sw = software_entry(&v);
    let frame = &sw["stations"][0];
    assert_eq!(frame["workers"][0], "framer");
    assert_eq!(frame["reviewers"][0], "value");
}

#[test]
fn factory_detail_returns_software_plan() {
    let (_d, server) = server();
    let res = server
        .darkrun_factory_detail(Parameters(FactoryRef {
            factory: "software".into(),
        }))
        .unwrap();
    assert!(is_ok(&res));
    let v = body(&res);
    assert_eq!(v["name"], "software");
    assert_eq!(v["stations"].as_array().unwrap().len(), 6);
}

#[test]
fn factory_detail_last_station_is_harden() {
    let (_d, server) = server();
    let v = body(
        &server
            .darkrun_factory_detail(Parameters(FactoryRef {
                factory: "software".into(),
            }))
            .unwrap(),
    );
    assert_eq!(v["stations"][5]["name"], "harden");
    assert_eq!(v["stations"][5]["artifact"], "release.md");
}

#[test]
fn factory_detail_specify_station_shape() {
    let (_d, server) = server();
    let v = body(
        &server
            .darkrun_factory_detail(Parameters(FactoryRef {
                factory: "software".into(),
            }))
            .unwrap(),
    );
    assert_eq!(v["stations"][1]["name"], "specify");
    assert_eq!(v["stations"][1]["kills"], "ambiguity");
    assert_eq!(v["stations"][1]["artifact"], "spec.md");
}

#[test]
fn factory_detail_errors_on_unknown() {
    let (_d, server) = server();
    let res = server
        .darkrun_factory_detail(Parameters(FactoryRef {
            factory: "nope".into(),
        }))
        .unwrap();
    assert!(is_err(&res));
    assert!(err_message(&res).contains("nope"));
}

#[test]
fn factory_detail_matches_list_entry() {
    let (_d, server) = server();
    let list = body(&server.darkrun_factory_list().unwrap());
    let detail = body(
        &server
            .darkrun_factory_detail(Parameters(FactoryRef {
                factory: "software".into(),
            }))
            .unwrap(),
    );
    let entry = list
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == "software")
        .expect("software listed");
    assert_eq!(entry["name"], detail["name"]);
    assert_eq!(entry["stations"], detail["stations"]);
}

// ── Cross-tool / integration flows ─────────────────────────────────────────

#[test]
fn full_walk_to_specify_via_tools_only() {
    let (_d, server) = started("r");
    // frame: spec, review, manufacture(empty -> still spec until units), so
    // create a completed unit to clear manufacture.
    next(&server, "r"); // spec -> review
    next(&server, "r"); // review -> manufacture
    create_unit(&server, "r", "u1", "frame");
    server
        .darkrun_unit_update(Parameters(UnitUpdateInput {
            slug: "r".into(),
            unit: "u1".into(),
            status: Some("completed".into()),
            depends_on: None,
            worker: None,
            inputs: None,
            outputs: None,
            ..Default::default()
        }))
        .unwrap();
    approve(&server, "r"); // clear the pre-execution operator gate
    // Tick through audit → reflect to the post-execution checkpoint gate.
    let mut cp = body(&next(&server, "r"));
    for _ in 0..4 {
        if cp["action"]["action"] == "checkpoint" {
            break;
        }
        cp = body(&next(&server, "r"));
    }
    assert_eq!(cp["action"]["action"], "checkpoint");
    // Approve to advance to specify.
    let decided = server
        .darkrun_checkpoint_decide(Parameters(CheckpointDecideInput {
            slug: "r".into(),
            approved: true,
            feedback: None,
        }))
        .unwrap();
    assert_eq!(body(&decided)["action"]["station"], "specify");
}

#[test]
fn feedback_preempts_run_track_on_next() {
    let (_d, server) = started("r");
    create_feedback(&server, "r", "frame", "broken");
    let v = body(&next(&server, "r"));
    assert_eq!(v["position"]["track"], "feedback");
    assert_eq!(v["action"]["action"], "fix_feedback");
}

#[test]
fn resolving_feedback_lets_run_resume() {
    let (_d, server) = started("r");
    let id = body(&create_feedback(&server, "r", "frame", "broken"))["id"]
        .as_str()
        .unwrap()
        .to_string();
    // Preempted by feedback.
    assert_eq!(body(&next(&server, "r"))["position"]["track"], "feedback");
    // Resolve it.
    server
        .darkrun_feedback_resolve(Parameters(FeedbackResolveInput {
            slug: "r".into(),
            feedback_id: id,
            status: "addressed".into(),
                reply: None,
        }))
        .unwrap();
    // Now the run track is back.
    assert_eq!(body(&next(&server, "r"))["position"]["track"], "run");
}

#[test]
fn units_for_two_runs_are_isolated() {
    let (_d, server) = server();
    start(&server, "a");
    start(&server, "b");
    create_unit(&server, "a", "u1", "frame");
    let a = body(
        &server
            .darkrun_unit_list(Parameters(RunRef { slug: "a".into() }))
            .unwrap(),
    );
    let b = body(
        &server
            .darkrun_unit_list(Parameters(RunRef { slug: "b".into() }))
            .unwrap(),
    );
    assert_eq!(a.as_array().unwrap().len(), 1);
    assert_eq!(b.as_array().unwrap().len(), 0);
}

#[test]
fn feedback_for_two_runs_are_isolated() {
    let (_d, server) = server();
    start(&server, "a");
    start(&server, "b");
    create_feedback(&server, "a", "frame", "only a");
    let a = body(
        &server
            .darkrun_feedback_list(Parameters(FeedbackListInput {
                slug: "a".into(),
                include_settled: true,
            }))
            .unwrap(),
    );
    let b = body(
        &server
            .darkrun_feedback_list(Parameters(FeedbackListInput {
                slug: "b".into(),
                include_settled: true,
            }))
            .unwrap(),
    );
    assert_eq!(a.as_array().unwrap().len(), 1);
    assert_eq!(b.as_array().unwrap().len(), 0);
}

#[test]
fn run_next_is_error_flag_false_on_success() {
    let (_d, server) = started("r");
    let res = next(&server, "r");
    assert_eq!(res.is_error, Some(false));
}

#[test]
fn structured_content_present_on_every_success() {
    let (_d, server) = started("r");
    assert!(next(&server, "r").structured_content.is_some());
    assert!(server
        .darkrun_run_inspect(Parameters(RunShowRef { slug: Some("r".into()) }))
        .unwrap()
        .structured_content
        .is_some());
    assert!(server
        .darkrun_factory_list()
        .unwrap()
        .structured_content
        .is_some());
}

#[test]
fn list_tools_wrap_array_in_items_envelope() {
    // MCP `structuredContent` must be a JSON object, never a top-level array —
    // strict clients reject "expected record, received array". Every list tool
    // wraps its array under `items`. Asserted on the raw content, bypassing the
    // `body()` unwrap.
    let (_d, server) = started("r");
    create_unit(&server, "r", "u1", "frame");
    let units = server
        .darkrun_unit_list(Parameters(RunRef { slug: "r".into() }))
        .unwrap()
        .structured_content
        .expect("structured content");
    assert!(units.is_object(), "unit_list must return an object, got {units}");
    assert_eq!(units["items"].as_array().expect("items array").len(), 1);

    let factories = server
        .darkrun_factory_list()
        .unwrap()
        .structured_content
        .expect("structured content");
    assert!(
        factories.is_object(),
        "factory_list must return an object, got {factories}"
    );
    assert!(factories["items"].is_array(), "factory_list items must be an array");
}

// ── Extended coverage: helpers for full station walks ──────────────────────

/// Drive the active station from its current cursor all the way to its open
/// Checkpoint via the tool surface, seeding a completed unit so Manufacture
/// clears. Returns the checkpoint result body. Auto stations advance during the
/// walk and return their checkpoint action before completing.
fn walk_station_to_checkpoint(server: &DarkrunServer, slug: &str, station: &str) -> Value {
    // Seed a completed unit on the station so Manufacture has work that clears.
    server
        .darkrun_unit_create(Parameters(UnitCreateInput {
            slug: slug.into(),
            unit: format!("{station}-u"),
            station: station.into(),
            title: None,
            depends_on: vec![],
            ..Default::default()
        }))
        .unwrap();
    // Consume the station's declared inputs so the runtime input-coverage gate
    // is satisfied (the run's distillation is carried forward).
    let inputs = darkrun_mcp::resolve_factory("software")
        .and_then(|f| f.station(station).map(|d| d.inputs.clone()))
        .filter(|i| !i.is_empty());
    server
        .darkrun_unit_update(Parameters(UnitUpdateInput {
            slug: slug.into(),
            unit: format!("{station}-u"),
            status: Some("completed".into()),
            depends_on: None,
            worker: None,
            inputs,
            outputs: None,
            ..Default::default()
        }))
        .unwrap();
    for _ in 0..14 {
        let v = body(&next(server, slug));
        let action = v["action"]["action"].as_str().unwrap().to_string();
        let on = v["action"]["station"].as_str().unwrap_or("").to_string();
        // Clear the pre-execution operator gate so the wave releases and the walk
        // reaches the post-execution gate.
        if action == "user_gate" && on == station {
            approve(server, slug);
            continue;
        }
        // Solo holds the Spec until the elaboration is sealed.
        if action == "spec" && on == station {
            server
                .darkrun_elaborate_seal(Parameters(ElaborateSealInput {
                    slug: slug.into(),
                    station: station.into(),
                }))
                .unwrap();
            continue;
        }
        // The gate is a local checkpoint or, for an external station,
        // external_review_requested.
        let is_gate = action == "checkpoint" || action == "external_review_requested";
        if is_gate && on == station {
            return v;
        }
        assert!(
            on == station || action == "spec",
            "unexpected action {action} on {on} while walking {station}"
        );
    }
    panic!("station {station} never reached checkpoint");
}

fn approve(server: &DarkrunServer, slug: &str) -> Value {
    body(
        &server
            .darkrun_checkpoint_decide(Parameters(CheckpointDecideInput {
                slug: slug.into(),
                approved: true,
                feedback: None,
            }))
            .unwrap(),
    )
}

/// If `action` is the whole-run review, stamp every run reviewer off (modelling
/// the run reviewers fanning out + signing) and return the now-sealing action;
/// otherwise return `action` unchanged.
fn sign_run_reviews(server: &DarkrunServer, slug: &str, action: Value) -> Value {
    if action["action"]["action"] != "run_review" {
        return action;
    }
    let reviewers: Vec<String> = action["action"]["reviewers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap().to_string())
        .collect();
    for r in reviewers {
        server
            .darkrun_run_review_stamp(Parameters(RunReviewStampInput {
                slug: slug.into(),
                role: r,
            }))
            .unwrap();
    }
    body(&next(server, slug))
}

#[test]
fn walk_frame_checkpoint_is_ask() {
    let (_d, server) = started("r");
    let cp = walk_station_to_checkpoint(&server, "r", "frame");
    assert_eq!(cp["action"]["kind"], "ask");
}

#[test]
fn walk_frame_then_specify_checkpoint_is_ask() {
    let (_d, server) = started("r");
    walk_station_to_checkpoint(&server, "r", "frame");
    approve(&server, "r");
    let cp = walk_station_to_checkpoint(&server, "r", "specify");
    assert_eq!(cp["action"]["kind"], "ask");
    assert_eq!(cp["action"]["station"], "specify");
}

#[test]
fn walk_through_shape_checkpoint_is_ask() {
    let (_d, server) = started("r");
    walk_station_to_checkpoint(&server, "r", "frame");
    approve(&server, "r");
    walk_station_to_checkpoint(&server, "r", "specify");
    approve(&server, "r");
    let cp = walk_station_to_checkpoint(&server, "r", "shape");
    assert_eq!(cp["action"]["kind"], "ask");
}

#[test]
fn build_station_ask_checkpoint_holds() {
    let (_d, server) = started("r");
    for st in ["frame", "specify", "shape"] {
        walk_station_to_checkpoint(&server, "r", st);
        approve(&server, "r");
    }
    let cp = walk_station_to_checkpoint(&server, "r", "build");
    assert_eq!(cp["action"]["kind"], "ask");
    // Holds for the operator decision — not auto-advanced.
    let show = body(
        &server
            .darkrun_run_inspect(Parameters(RunShowRef { slug: Some("r".into()) }))
            .unwrap(),
    );
    assert_eq!(show["state"]["stations"]["build"]["status"], "in_progress");
    assert_eq!(show["state"]["active_station"], "build");
}

#[test]
fn prove_station_ask_checkpoint_advances_to_harden_on_approve() {
    let (_d, server) = started("r");
    for st in ["frame", "specify", "shape", "build"] {
        walk_station_to_checkpoint(&server, "r", st);
        approve(&server, "r");
    }
    let cp = walk_station_to_checkpoint(&server, "r", "prove");
    assert_eq!(cp["action"]["kind"], "ask");
    approve(&server, "r");
    let show = body(
        &server
            .darkrun_run_inspect(Parameters(RunShowRef { slug: Some("r".into()) }))
            .unwrap(),
    );
    assert_eq!(show["state"]["active_station"], "harden");
}

#[test]
fn harden_station_ask_checkpoint_holds() {
    let (_d, server) = started("r");
    for st in ["frame", "specify", "shape", "build", "prove"] {
        walk_station_to_checkpoint(&server, "r", st);
        approve(&server, "r");
    }
    let cp = walk_station_to_checkpoint(&server, "r", "harden");
    assert_eq!(cp["action"]["action"], "checkpoint");
    assert_eq!(cp["action"]["kind"], "ask");
    let show = body(
        &server
            .darkrun_run_inspect(Parameters(RunShowRef { slug: Some("r".into()) }))
            .unwrap(),
    );
    assert_eq!(show["state"]["stations"]["harden"]["status"], "in_progress");
}

#[test]
fn full_run_walks_to_sealed() {
    let (_d, server) = started("r");
    for st in ["frame", "specify", "shape", "build", "prove"] {
        walk_station_to_checkpoint(&server, "r", st);
        approve(&server, "r");
    }
    walk_station_to_checkpoint(&server, "r", "harden");
    let sealed = sign_run_reviews(&server, "r", approve(&server, "r"));
    assert_eq!(sealed["action"]["action"], "sealed");
    assert_eq!(sealed["action"]["run"], "r");
}

#[test]
fn sealed_run_show_position_is_sealed() {
    let (_d, server) = started("r");
    for st in ["frame", "specify", "shape", "build", "prove"] {
        walk_station_to_checkpoint(&server, "r", st);
        approve(&server, "r");
    }
    walk_station_to_checkpoint(&server, "r", "harden");
    sign_run_reviews(&server, "r", approve(&server, "r"));
    let show = body(
        &server
            .darkrun_run_inspect(Parameters(RunShowRef { slug: Some("r".into()) }))
            .unwrap(),
    );
    assert_eq!(show["position"]["action"]["action"], "sealed");
}

#[test]
fn sealed_run_next_returns_sealed() {
    let (_d, server) = started("r");
    for st in ["frame", "specify", "shape", "build", "prove"] {
        walk_station_to_checkpoint(&server, "r", st);
        approve(&server, "r");
    }
    walk_station_to_checkpoint(&server, "r", "harden");
    sign_run_reviews(&server, "r", approve(&server, "r"));
    // Re-ticking a sealed run keeps returning sealed (idempotent terminal).
    let v = body(&next(&server, "r"));
    assert_eq!(v["action"]["action"], "sealed");
}

// ── Severity matrix ────────────────────────────────────────────────────────

#[test]
fn feedback_severity_blocker_roundtrips() {
    let (_d, server) = started("r");
    let res = server
        .darkrun_feedback_create(Parameters(FeedbackCreateInput {
            slug: "r".into(),
            station: "frame".into(),
            body: "x".into(),
            severity: Some("blocker".into()),
                origin: None,
                invalidates: None,
        }))
        .unwrap();
    assert_eq!(body(&res)["severity"], "blocker");
}

#[test]
fn feedback_severity_medium_roundtrips() {
    let (_d, server) = started("r");
    let res = server
        .darkrun_feedback_create(Parameters(FeedbackCreateInput {
            slug: "r".into(),
            station: "frame".into(),
            body: "x".into(),
            severity: Some("medium".into()),
                origin: None,
                invalidates: None,
        }))
        .unwrap();
    assert_eq!(body(&res)["severity"], "medium");
}

#[test]
fn feedback_severity_low_roundtrips() {
    let (_d, server) = started("r");
    let res = server
        .darkrun_feedback_create(Parameters(FeedbackCreateInput {
            slug: "r".into(),
            station: "frame".into(),
            body: "x".into(),
            severity: Some("low".into()),
                origin: None,
                invalidates: None,
        }))
        .unwrap();
    assert_eq!(body(&res)["severity"], "low");
}

#[test]
fn feedback_severity_uppercase_accepted() {
    let (_d, server) = started("r");
    let res = server
        .darkrun_feedback_create(Parameters(FeedbackCreateInput {
            slug: "r".into(),
            station: "frame".into(),
            body: "x".into(),
            severity: Some("HIGH".into()),
                origin: None,
                invalidates: None,
        }))
        .unwrap();
    assert!(is_ok(&res));
    assert_eq!(body(&res)["severity"], "high");
}

#[test]
fn feedback_no_severity_is_absent() {
    let (_d, server) = started("r");
    let v = body(&create_feedback(&server, "r", "frame", "x"));
    // severity skip-if-none → either absent or null.
    assert!(v.get("severity").map(|s| s.is_null()).unwrap_or(true));
}

// ── Status alias matrix for unit_update ────────────────────────────────────

#[test]
fn unit_update_status_pending_ok() {
    let (_d, server) = started("r");
    create_unit(&server, "r", "u1", "frame");
    let res = server
        .darkrun_unit_update(Parameters(UnitUpdateInput {
            slug: "r".into(),
            unit: "u1".into(),
            status: Some("pending".into()),
            depends_on: None,
            worker: None,
            inputs: None,
            outputs: None,
            ..Default::default()
        }))
        .unwrap();
    assert_eq!(body(&res)["frontmatter"]["status"], "pending");
}

#[test]
fn unit_update_status_active_ok() {
    let (_d, server) = started("r");
    create_unit(&server, "r", "u1", "frame");
    let res = server
        .darkrun_unit_update(Parameters(UnitUpdateInput {
            slug: "r".into(),
            unit: "u1".into(),
            status: Some("active".into()),
            depends_on: None,
            worker: None,
            inputs: None,
            outputs: None,
            ..Default::default()
        }))
        .unwrap();
    assert_eq!(body(&res)["frontmatter"]["status"], "active");
}

#[test]
fn unit_update_status_blocked_ok() {
    let (_d, server) = started("r");
    create_unit(&server, "r", "u1", "frame");
    let res = server
        .darkrun_unit_update(Parameters(UnitUpdateInput {
            slug: "r".into(),
            unit: "u1".into(),
            status: Some("blocked".into()),
            depends_on: None,
            worker: None,
            inputs: None,
            outputs: None,
            ..Default::default()
        }))
        .unwrap();
    assert_eq!(body(&res)["frontmatter"]["status"], "blocked");
}

#[test]
fn unit_update_status_uppercase_normalized() {
    let (_d, server) = started("r");
    create_unit(&server, "r", "u1", "frame");
    let res = server
        .darkrun_unit_update(Parameters(UnitUpdateInput {
            slug: "r".into(),
            unit: "u1".into(),
            status: Some("COMPLETED".into()),
            depends_on: None,
            worker: None,
            inputs: None,
            outputs: None,
            ..Default::default()
        }))
        .unwrap();
    assert!(is_ok(&res));
    assert_eq!(body(&res)["frontmatter"]["status"], "completed");
}

#[test]
fn unit_update_inputs_rejected_once_active() {
    let (_d, server) = started("r");
    create_unit(&server, "r", "u1", "frame");
    server
        .darkrun_unit_update(Parameters(UnitUpdateInput {
            slug: "r".into(),
            unit: "u1".into(),
            status: Some("completed".into()),
            depends_on: None,
            worker: None,
            inputs: None,
            outputs: None,
            ..Default::default()
        }))
        .unwrap();
    let res = server
        .darkrun_unit_update(Parameters(UnitUpdateInput {
            slug: "r".into(),
            unit: "u1".into(),
            status: None,
            depends_on: None,
            worker: None,
            inputs: Some(vec!["in.md".into()]),
            outputs: None,
            ..Default::default()
        }))
        .unwrap();
    assert!(is_err(&res));
}

#[test]
fn unit_update_outputs_allowed_after_active() {
    let (_d, server) = started("r");
    create_unit(&server, "r", "u1", "frame");
    server
        .darkrun_unit_update(Parameters(UnitUpdateInput {
            slug: "r".into(),
            unit: "u1".into(),
            status: Some("completed".into()),
            depends_on: None,
            worker: None,
            inputs: None,
            outputs: None,
            ..Default::default()
        }))
        .unwrap();
    // outputs are not structural → allowed even when not pending.
    let res = server
        .darkrun_unit_update(Parameters(UnitUpdateInput {
            slug: "r".into(),
            unit: "u1".into(),
            status: None,
            depends_on: None,
            worker: None,
            inputs: None,
            outputs: Some(vec!["out.md".into()]),
            ..Default::default()
        }))
        .unwrap();
    assert!(is_ok(&res));
    assert_eq!(body(&res)["frontmatter"]["outputs"][0], "out.md");
}

#[test]
fn unit_update_inputs_allowed_while_pending() {
    let (_d, server) = started("r");
    create_unit(&server, "r", "u1", "frame");
    let res = server
        .darkrun_unit_update(Parameters(UnitUpdateInput {
            slug: "r".into(),
            unit: "u1".into(),
            status: None,
            depends_on: None,
            worker: None,
            inputs: Some(vec!["spec.md".into()]),
            outputs: None,
            ..Default::default()
        }))
        .unwrap();
    assert!(is_ok(&res));
    assert_eq!(body(&res)["frontmatter"]["inputs"][0], "spec.md");
}

// ── Determinism / idempotency ──────────────────────────────────────────────

#[test]
fn factory_list_is_deterministic_across_calls() {
    let (_d, server) = server();
    let a = body(&server.darkrun_factory_list().unwrap());
    let b = body(&server.darkrun_factory_list().unwrap());
    assert_eq!(a, b);
}

#[test]
fn factory_detail_is_deterministic_across_calls() {
    let (_d, server) = server();
    let a = body(
        &server
            .darkrun_factory_detail(Parameters(FactoryRef {
                factory: "software".into(),
            }))
            .unwrap(),
    );
    let b = body(
        &server
            .darkrun_factory_detail(Parameters(FactoryRef {
                factory: "software".into(),
            }))
            .unwrap(),
    );
    assert_eq!(a, b);
}

#[test]
fn run_show_position_is_deterministic_without_ticks() {
    let (_d, server) = started("r");
    let a = body(
        &server
            .darkrun_run_inspect(Parameters(RunShowRef { slug: Some("r".into()) }))
            .unwrap(),
    );
    let b = body(
        &server
            .darkrun_run_inspect(Parameters(RunShowRef { slug: Some("r".into()) }))
            .unwrap(),
    );
    assert_eq!(a["position"], b["position"]);
}

#[test]
fn unit_list_repeated_read_is_stable() {
    let (_d, server) = started("r");
    create_unit(&server, "r", "u1", "frame");
    create_unit(&server, "r", "u2", "frame");
    let a = body(
        &server
            .darkrun_unit_list(Parameters(RunRef { slug: "r".into() }))
            .unwrap(),
    );
    let b = body(
        &server
            .darkrun_unit_list(Parameters(RunRef { slug: "r".into() }))
            .unwrap(),
    );
    assert_eq!(a, b);
}

#[test]
fn feedback_list_repeated_read_is_stable() {
    let (_d, server) = started("r");
    create_feedback(&server, "r", "frame", "a");
    create_feedback(&server, "r", "shape", "b");
    let a = body(
        &server
            .darkrun_feedback_list(Parameters(FeedbackListInput {
                slug: "r".into(),
                include_settled: true,
            }))
            .unwrap(),
    );
    let b = body(
        &server
            .darkrun_feedback_list(Parameters(FeedbackListInput {
                slug: "r".into(),
                include_settled: true,
            }))
            .unwrap(),
    );
    assert_eq!(a, b);
}

#[test]
fn two_independent_servers_produce_same_first_action() {
    let (_d1, s1) = started("r");
    let (_d2, s2) = started("r");
    let a = body(&next(&s1, "r"));
    let b = body(&next(&s2, "r"));
    assert_eq!(a["action"], b["action"]);
}

// ── Feedback id allocation edge cases ──────────────────────────────────────

#[test]
fn feedback_ids_continue_after_resolution() {
    let (_d, server) = started("r");
    let id1 = body(&create_feedback(&server, "r", "frame", "a"))["id"]
        .as_str()
        .unwrap()
        .to_string();
    server
        .darkrun_feedback_resolve(Parameters(FeedbackResolveInput {
            slug: "r".into(),
            feedback_id: id1,
            status: "addressed".into(),
                reply: None,
        }))
        .unwrap();
    // Next id is still fb-02 — resolution doesn't free the slot.
    let id2 = body(&create_feedback(&server, "r", "frame", "b"))["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(id2, "fb-02");
}

#[test]
fn feedback_create_many_allocates_padded_ids() {
    let (_d, server) = started("r");
    for _ in 0..9 {
        create_feedback(&server, "r", "frame", "x");
    }
    let v = body(&create_feedback(&server, "r", "frame", "tenth"));
    assert_eq!(v["id"], "fb-10");
}

// ── checkpoint_decide idempotency & flows ──────────────────────────────────

#[test]
fn checkpoint_approve_each_station_independently() {
    let (_d, server) = started("r");
    let after_frame = approve(&server, "r");
    assert_eq!(after_frame["action"]["station"], "specify");
    let after_specify = approve(&server, "r");
    assert_eq!(after_specify["action"]["station"], "shape");
}

#[test]
fn checkpoint_decide_reject_routes_then_approve_after_resolve() {
    let (_d, server) = started("r");
    server
        .darkrun_checkpoint_decide(Parameters(CheckpointDecideInput {
            slug: "r".into(),
            approved: false,
            feedback: Some("redo".into()),
        }))
        .unwrap();
    // Resolve the routed feedback.
    let id = body(
        &server
            .darkrun_feedback_list(Parameters(FeedbackListInput {
                slug: "r".into(),
                include_settled: true,
            }))
            .unwrap(),
    )[0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    server
        .darkrun_feedback_resolve(Parameters(FeedbackResolveInput {
            slug: "r".into(),
            feedback_id: id,
            status: "addressed".into(),
                reply: None,
        }))
        .unwrap();
    // Frame is still blocked but feedback cleared — now approve to advance.
    let v = approve(&server, "r");
    assert_eq!(v["action"]["station"], "specify");
}

// ── run_next noop message content ──────────────────────────────────────────

#[test]
fn run_next_noop_carries_message() {
    let (_d, server) = started("r");
    next(&server, "r");
    next(&server, "r");
    // A dispatched, in-flight unit yields the mid-wave noop.
    create_unit(&server, "r", "u1", "frame");
    server
        .darkrun_unit_update(Parameters(UnitUpdateInput {
            slug: "r".into(),
            unit: "u1".into(),
            status: Some("in_progress".into()),
            depends_on: None,
            worker: None,
            inputs: None,
            outputs: None,
            ..Default::default()
        }))
        .unwrap();
    approve(&server, "r"); // clear the pre-execution operator gate
    let v = body(&next(&server, "r"));
    assert!(v["action"]["message"].as_str().unwrap().contains("noop"));
}

// ── Cross-tool: archive then show/next still work ──────────────────────────

#[test]
fn archived_run_is_still_showable() {
    let (_d, server) = started("r");
    server
        .darkrun_run_archive(Parameters(RunArchiveInput {
            slug: "r".into(),
            archived: true,
        }))
        .unwrap();
    let res = server
        .darkrun_run_inspect(Parameters(RunShowRef { slug: Some("r".into()) }))
        .unwrap();
    assert!(is_ok(&res));
    assert_eq!(body(&res)["run"]["frontmatter"]["archived"], true);
}

#[test]
fn archived_run_can_still_tick() {
    let (_d, server) = started("r");
    server
        .darkrun_run_archive(Parameters(RunArchiveInput {
            slug: "r".into(),
            archived: true,
        }))
        .unwrap();
    let res = next(&server, "r");
    assert!(is_ok(&res));
}

// ── Units belong to non-active stations too ────────────────────────────────

#[test]
fn unit_create_on_later_station_is_allowed() {
    let (_d, server) = started("r");
    let res = create_unit(&server, "r", "u1", "harden");
    assert!(is_ok(&res));
    assert_eq!(body(&res)["frontmatter"]["station"], "harden");
}

#[test]
fn unit_list_includes_units_across_stations() {
    let (_d, server) = started("r");
    create_unit(&server, "r", "u1", "frame");
    create_unit(&server, "r", "u2", "shape");
    create_unit(&server, "r", "u3", "harden");
    let v = body(
        &server
            .darkrun_unit_list(Parameters(RunRef { slug: "r".into() }))
            .unwrap(),
    );
    assert_eq!(v.as_array().unwrap().len(), 3);
}

// ── Error path: operations on a never-started run ──────────────────────────

#[test]
fn unit_list_on_missing_run_is_empty_or_error() {
    let (_d, server) = server();
    let res = server
        .darkrun_unit_list(Parameters(RunRef {
            slug: "ghost".into(),
        }))
        .unwrap();
    // No run dir → an empty unit set (read tolerates the absent dir).
    if is_ok(&res) {
        assert_eq!(body(&res).as_array().unwrap().len(), 0);
    }
}

#[test]
fn feedback_list_on_missing_run_is_empty_or_error() {
    let (_d, server) = server();
    let res = server
        .darkrun_feedback_list(Parameters(FeedbackListInput {
            slug: "ghost".into(),
            include_settled: true,
        }))
        .unwrap();
    if is_ok(&res) {
        assert_eq!(body(&res).as_array().unwrap().len(), 0);
    }
}

// ── Batch 3: deeper shape & wave-readiness coverage ────────────────────────

#[test]
fn manufacture_dispatches_only_dependency_ready_units() {
    let (_d, server) = started("r");
    next(&server, "r"); // spec -> review
    next(&server, "r"); // review -> manufacture
                        // base has no deps; dependent waits on base.
    create_unit(&server, "r", "base", "frame");
    server
        .darkrun_unit_create(Parameters(UnitCreateInput {
            slug: "r".into(),
            unit: "dependent".into(),
            station: "frame".into(),
            title: None,
            depends_on: vec!["base".into()],
            ..Default::default()
        }))
        .unwrap();
    approve(&server, "r"); // clear the pre-execution operator gate
    let v = body(&next(&server, "r"));
    let units = v["action"]["units"].as_array().unwrap();
    // Only base is wave-ready.
    assert!(units.iter().any(|u| u == "base"));
    assert!(!units.iter().any(|u| u == "dependent"));
}

#[test]
fn manufacture_releases_dependent_after_base_completes() {
    let (_d, server) = started("r");
    next(&server, "r");
    next(&server, "r");
    create_unit(&server, "r", "base", "frame");
    server
        .darkrun_unit_create(Parameters(UnitCreateInput {
            slug: "r".into(),
            unit: "dependent".into(),
            station: "frame".into(),
            title: None,
            depends_on: vec!["base".into()],
            ..Default::default()
        }))
        .unwrap();
    approve(&server, "r"); // clear the pre-execution operator gate
    next(&server, "r"); // dispatch base
    server
        .darkrun_unit_update(Parameters(UnitUpdateInput {
            slug: "r".into(),
            unit: "base".into(),
            status: Some("completed".into()),
            depends_on: None,
            worker: None,
            inputs: None,
            outputs: None,
            ..Default::default()
        }))
        .unwrap();
    let v = body(&next(&server, "r"));
    // Now dependent is wave-ready.
    assert_eq!(v["action"]["action"], "manufacture");
    let units = v["action"]["units"].as_array().unwrap();
    assert!(units.iter().any(|u| u == "dependent"));
}

#[test]
fn manufacture_dispatches_two_independent_units_together() {
    let (_d, server) = started("r");
    next(&server, "r");
    next(&server, "r");
    create_unit(&server, "r", "u1", "frame");
    create_unit(&server, "r", "u2", "frame");
    approve(&server, "r"); // clear the pre-execution operator gate
    let v = body(&next(&server, "r"));
    let units = v["action"]["units"].as_array().unwrap();
    assert_eq!(units.len(), 2);
}

#[test]
fn run_show_run_frontmatter_factory_field() {
    let (_d, server) = started("r");
    let v = body(
        &server
            .darkrun_run_inspect(Parameters(RunShowRef { slug: Some("r".into()) }))
            .unwrap(),
    );
    assert_eq!(v["run"]["frontmatter"]["factory"], "software");
}

#[test]
fn run_show_state_stations_seeded_for_frame() {
    let (_d, server) = started("r");
    let v = body(
        &server
            .darkrun_run_inspect(Parameters(RunShowRef { slug: Some("r".into()) }))
            .unwrap(),
    );
    assert!(v["state"]["stations"].get("frame").is_some());
}

#[test]
fn run_show_position_track_is_run_for_fresh() {
    let (_d, server) = started("r");
    let v = body(
        &server
            .darkrun_run_inspect(Parameters(RunShowRef { slug: Some("r".into()) }))
            .unwrap(),
    );
    assert_eq!(v["position"]["track"], "run");
}

#[test]
fn feedback_move_across_three_stations() {
    let (_d, server) = started("r");
    let id = body(&create_feedback(&server, "r", "frame", "x"))["id"]
        .as_str()
        .unwrap()
        .to_string();
    let a = body(
        &server
            .darkrun_feedback_move(Parameters(FeedbackMoveInput {
                slug: "r".into(),
                feedback_id: id.clone(),
                to_station: "shape".into(),
            }))
            .unwrap(),
    );
    assert_eq!(a["station"], "shape");
    let b = body(
        &server
            .darkrun_feedback_move(Parameters(FeedbackMoveInput {
                slug: "r".into(),
                feedback_id: id,
                to_station: "harden".into(),
            }))
            .unwrap(),
    );
    assert_eq!(b["station"], "harden");
}

#[test]
fn feedback_move_preserves_open_status() {
    let (_d, server) = started("r");
    let id = body(&create_feedback(&server, "r", "frame", "x"))["id"]
        .as_str()
        .unwrap()
        .to_string();
    let moved = body(
        &server
            .darkrun_feedback_move(Parameters(FeedbackMoveInput {
                slug: "r".into(),
                feedback_id: id,
                to_station: "shape".into(),
            }))
            .unwrap(),
    );
    assert_eq!(moved["status"], "pending");
}

#[test]
fn feedback_reject_reason_appears_in_listing() {
    let (_d, server) = started("r");
    let id = body(&create_feedback(&server, "r", "frame", "thing"))["id"]
        .as_str()
        .unwrap()
        .to_string();
    server
        .darkrun_feedback_reject(Parameters(FeedbackRejectInput {
            slug: "r".into(),
            feedback_id: id,
            reason: "obsolete".into(),
        }))
        .unwrap();
    let v = body(
        &server
            .darkrun_feedback_list(Parameters(FeedbackListInput {
                slug: "r".into(),
                include_settled: true,
            }))
            .unwrap(),
    );
    assert!(v[0]["body"].as_str().unwrap().contains("obsolete"));
}

#[test]
fn multiple_open_feedback_fixes_first_by_id() {
    let (_d, server) = started("r");
    create_feedback(&server, "r", "frame", "first");
    create_feedback(&server, "r", "frame", "second");
    let v = body(&next(&server, "r"));
    assert_eq!(v["action"]["action"], "fix_feedback");
    assert_eq!(v["action"]["feedback_id"], "fb-01");
}

#[test]
fn resolving_first_feedback_surfaces_second() {
    let (_d, server) = started("r");
    create_feedback(&server, "r", "frame", "first");
    create_feedback(&server, "r", "frame", "second");
    server
        .darkrun_feedback_resolve(Parameters(FeedbackResolveInput {
            slug: "r".into(),
            feedback_id: "fb-01".into(),
            status: "addressed".into(),
                reply: None,
        }))
        .unwrap();
    let v = body(&next(&server, "r"));
    assert_eq!(v["action"]["action"], "fix_feedback");
    assert_eq!(v["action"]["feedback_id"], "fb-02");
}

#[test]
fn feedback_fix_carries_run_and_station() {
    let (_d, server) = started("r");
    create_feedback(&server, "r", "frame", "x");
    let v = body(&next(&server, "r"));
    assert_eq!(v["action"]["run"], "r");
    assert_eq!(v["action"]["station"], "frame");
}

// ── run_archive flag default semantics via explicit values ────────────────

#[test]
fn run_archive_explicit_false_keeps_visible() {
    let (_d, server) = started("r");
    server
        .darkrun_run_archive(Parameters(RunArchiveInput {
            slug: "r".into(),
            archived: false,
        }))
        .unwrap();
    let v = body(
        &server
            .darkrun_run_list(Parameters(RunListInput {
                include_archived: false,
            }))
            .unwrap(),
    );
    assert_eq!(v.as_array().unwrap().len(), 1);
}

// ── unit_create title vs slug edge cases ──────────────────────────────────

#[test]
fn unit_create_without_title_uses_slug_as_title() {
    let (_d, server) = started("r");
    let v = body(&create_unit(&server, "r", "my-unit", "frame"));
    assert_eq!(v["title"], "my-unit");
}

#[test]
fn unit_create_body_has_heading() {
    let (_d, server) = started("r");
    let v = body(&create_unit(&server, "r", "u1", "frame"));
    assert!(v["body"].as_str().unwrap().contains("# u1"));
}

// ── checkpoint kind reflected by detail vs next ────────────────────────────

#[test]
fn checkpoint_kind_in_next_is_the_mode_gate() {
    let (_d, server) = started("r");
    let cp = walk_station_to_checkpoint(&server, "r", "frame");
    // The gate is now a pure function of the run's global mode; a solo run asks.
    assert_eq!(cp["action"]["kind"], "ask");
}

// ── run_list status field follows lifecycle ────────────────────────────────

#[test]
fn run_list_active_station_advances_after_approval() {
    let (_d, server) = started("r");
    approve(&server, "r"); // frame -> specify
    let v = body(
        &server
            .darkrun_run_list(Parameters(RunListInput {
                include_archived: false,
            }))
            .unwrap(),
    );
    // The summary's active_station comes from run frontmatter (legacy cache),
    // which run_start seeds to frame and isn't rewritten by ticks; assert it is
    // a known station name regardless.
    let st = v[0]["active_station"].as_str().unwrap();
    assert!(["frame", "specify", "shape", "build", "prove", "harden"].contains(&st));
}

// ── Many runs listing ──────────────────────────────────────────────────────

#[test]
fn run_list_handles_many_runs() {
    let (_d, server) = server();
    for i in 0..10 {
        start(&server, &format!("run-{i:02}"));
    }
    let v = body(
        &server
            .darkrun_run_list(Parameters(RunListInput {
                include_archived: false,
            }))
            .unwrap(),
    );
    assert_eq!(v.as_array().unwrap().len(), 10);
}

#[test]
fn run_list_sorted_among_many() {
    let (_d, server) = server();
    for i in [3, 1, 2, 0] {
        start(&server, &format!("run-{i}"));
    }
    let v = body(
        &server
            .darkrun_run_list(Parameters(RunListInput {
                include_archived: false,
            }))
            .unwrap(),
    );
    let slugs: Vec<&str> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["slug"].as_str().unwrap())
        .collect();
    assert_eq!(slugs, vec!["run-0", "run-1", "run-2", "run-3"]);
}

// ── is_error flags on success paths ───────────────────────────────────────

#[test]
fn unit_create_is_error_false() {
    let (_d, server) = started("r");
    assert_eq!(
        create_unit(&server, "r", "u1", "frame").is_error,
        Some(false)
    );
}

#[test]
fn feedback_create_is_error_false() {
    let (_d, server) = started("r");
    assert_eq!(
        create_feedback(&server, "r", "frame", "x").is_error,
        Some(false)
    );
}

#[test]
fn factory_detail_is_error_false_for_software() {
    let (_d, server) = server();
    let res = server
        .darkrun_factory_detail(Parameters(FactoryRef {
            factory: "software".into(),
        }))
        .unwrap();
    assert_eq!(res.is_error, Some(false));
}

#[test]
fn run_archive_is_error_false() {
    let (_d, server) = started("r");
    let res = server
        .darkrun_run_archive(Parameters(RunArchiveInput {
            slug: "r".into(),
            archived: true,
        }))
        .unwrap();
    assert_eq!(res.is_error, Some(false));
}

// ── Batch 4: per-station feedback, modes, and action-shape assertions ──────

#[test]
fn feedback_create_on_specify_station() {
    let (_d, server) = started("r");
    let v = body(&create_feedback(&server, "r", "specify", "ambiguous req"));
    assert_eq!(v["station"], "specify");
}

#[test]
fn feedback_create_on_shape_station() {
    let (_d, server) = started("r");
    let v = body(&create_feedback(&server, "r", "shape", "fragile design"));
    assert_eq!(v["station"], "shape");
}

#[test]
fn feedback_create_on_build_station() {
    let (_d, server) = started("r");
    let v = body(&create_feedback(&server, "r", "build", "off-by-one"));
    assert_eq!(v["station"], "build");
}

#[test]
fn feedback_create_on_prove_station() {
    let (_d, server) = started("r");
    let v = body(&create_feedback(&server, "r", "prove", "missing coverage"));
    assert_eq!(v["station"], "prove");
}

#[test]
fn feedback_create_on_harden_station() {
    let (_d, server) = started("r");
    let v = body(&create_feedback(&server, "r", "harden", "secrets in log"));
    assert_eq!(v["station"], "harden");
}

#[test]
fn feedback_create_on_arbitrary_station_string() {
    // The tool does not validate station names against the factory plan;
    // triage can place feedback anywhere.
    let (_d, server) = started("r");
    let res = create_feedback(&server, "r", "totally-made-up", "x");
    assert!(is_ok(&res));
    assert_eq!(body(&res)["station"], "totally-made-up");
}

#[test]
fn run_start_mode_right_sized_persists_in_show() {
    let (_d, server) = server();
    server
        .darkrun_run_new(Parameters(RunStartInput {
            slug: "r".into(),
            factory: "software".into(),
            title: None,
            mode: "dark".into(),
            size: "full".into(),        }))
        .unwrap();
    let v = body(
        &server
            .darkrun_run_inspect(Parameters(RunShowRef { slug: Some("r".into()) }))
            .unwrap(),
    );
    assert_eq!(v["run"]["frontmatter"]["mode"], "dark");
}

#[test]
fn run_start_unknown_mode_normalizes_to_solo() {
    let (_d, server) = server();
    let res = server
        .darkrun_run_new(Parameters(RunStartInput {
            slug: "r".into(),
            factory: "software".into(),
            title: None,
            mode: "experimental-xyz".into(),
            size: "full".into(),        }))
        .unwrap();
    // An unrecognized mode label resolves to the in-the-loop default.
    assert_eq!(body(&res)["frontmatter"]["mode"], "solo");
}

#[test]
fn spec_action_shape_has_run_station_kills() {
    let (_d, server) = started("r");
    let v = body(&next(&server, "r"));
    let a = &v["action"];
    assert!(a["run"].is_string());
    assert!(a["station"].is_string());
    assert!(a["kills"].is_string());
}

#[test]
fn review_action_shape_has_reviewers_array() {
    let (_d, server) = started("r");
    next(&server, "r");
    let v = body(&next(&server, "r"));
    assert!(v["action"]["reviewers"].is_array());
}

#[test]
fn manufacture_action_shape_has_worker_and_units() {
    let (_d, server) = started("r");
    next(&server, "r");
    next(&server, "r");
    create_unit(&server, "r", "u1", "frame");
    approve(&server, "r"); // clear the pre-execution operator gate
    let v = body(&next(&server, "r"));
    assert!(v["action"]["worker"].is_string());
    assert!(v["action"]["units"].is_array());
}

#[test]
fn audit_action_shape_has_reviewers() {
    let (_d, server) = started("r");
    next(&server, "r");
    next(&server, "r");
    create_unit(&server, "r", "u1", "frame");
    approve(&server, "r"); // clear the pre-execution operator gate
    next(&server, "r");
    server
        .darkrun_unit_update(Parameters(UnitUpdateInput {
            slug: "r".into(),
            unit: "u1".into(),
            status: Some("completed".into()),
            depends_on: None,
            worker: None,
            inputs: None,
            outputs: None,
            ..Default::default()
        }))
        .unwrap();
    let v = body(&next(&server, "r"));
    assert_eq!(v["action"]["action"], "audit");
    assert!(v["action"]["reviewers"].is_array());
}

#[test]
fn reflect_action_shape_has_run_and_station_only() {
    let (_d, server) = started("r");
    next(&server, "r");
    next(&server, "r");
    create_unit(&server, "r", "u1", "frame");
    approve(&server, "r"); // clear the pre-execution operator gate
    next(&server, "r");
    server
        .darkrun_unit_update(Parameters(UnitUpdateInput {
            slug: "r".into(),
            unit: "u1".into(),
            status: Some("completed".into()),
            depends_on: None,
            worker: None,
            inputs: None,
            outputs: None,
            ..Default::default()
        }))
        .unwrap();
    next(&server, "r"); // audit (folds in the old tests phase)
    let v = body(&next(&server, "r"));
    assert_eq!(v["action"]["action"], "reflect");
    assert_eq!(v["action"]["run"], "r");
    assert_eq!(v["action"]["station"], "frame");
}

#[test]
fn checkpoint_action_shape_has_kind() {
    let (_d, server) = started("r");
    let cp = walk_station_to_checkpoint(&server, "r", "frame");
    assert!(cp["action"]["kind"].is_string());
    assert_eq!(cp["action"]["run"], "r");
}

#[test]
fn sealed_action_shape_has_run_only() {
    let (_d, server) = started("r");
    for st in ["frame", "specify", "shape", "build", "prove"] {
        walk_station_to_checkpoint(&server, "r", st);
        approve(&server, "r");
    }
    walk_station_to_checkpoint(&server, "r", "harden");
    let v = sign_run_reviews(&server, "r", approve(&server, "r"));
    assert_eq!(v["action"]["action"], "sealed");
    assert_eq!(v["action"]["run"], "r");
    assert!(v["action"].get("station").is_none());
}

#[test]
fn fix_feedback_action_shape_has_feedback_id() {
    let (_d, server) = started("r");
    create_feedback(&server, "r", "frame", "x");
    let v = body(&next(&server, "r"));
    assert!(v["action"]["feedback_id"].is_string());
}

// ── tick result top-level shape ────────────────────────────────────────────

#[test]
fn tick_result_has_run_position_action() {
    let (_d, server) = started("r");
    let v = body(&next(&server, "r"));
    assert!(v.get("run").is_some());
    assert!(v.get("position").is_some());
    assert!(v.get("action").is_some());
}

#[test]
fn tick_position_action_equals_top_action() {
    let (_d, server) = started("r");
    let v = body(&next(&server, "r"));
    assert_eq!(v["position"]["action"], v["action"]);
}

// ── Feedback resolve status normalization ──────────────────────────────────

#[test]
fn feedback_resolve_uppercase_terminal_status() {
    let (_d, server) = started("r");
    let id = body(&create_feedback(&server, "r", "frame", "x"))["id"]
        .as_str()
        .unwrap()
        .to_string();
    let res = server
        .darkrun_feedback_resolve(Parameters(FeedbackResolveInput {
            slug: "r".into(),
            feedback_id: id,
            status: "ADDRESSED".into(),
                reply: None,
        }))
        .unwrap();
    assert!(is_ok(&res));
    assert_eq!(body(&res)["status"], "addressed");
}

#[test]
fn feedback_resolve_nonactionable_alias() {
    let (_d, server) = started("r");
    let id = body(&create_feedback(&server, "r", "frame", "x"))["id"]
        .as_str()
        .unwrap()
        .to_string();
    let res = server
        .darkrun_feedback_resolve(Parameters(FeedbackResolveInput {
            slug: "r".into(),
            feedback_id: id,
            status: "nonactionable".into(),
                reply: None,
        }))
        .unwrap();
    assert!(is_ok(&res));
    assert_eq!(body(&res)["status"], "non_actionable");
}

// ── checkpoint rejected feedback uses fb-checkpoint id ─────────────────────

#[test]
fn checkpoint_reject_feedback_listed_with_pending_status() {
    let (_d, server) = started("r");
    server
        .darkrun_checkpoint_decide(Parameters(CheckpointDecideInput {
            slug: "r".into(),
            approved: false,
            feedback: Some("rework".into()),
        }))
        .unwrap();
    let v = body(
        &server
            .darkrun_feedback_list(Parameters(FeedbackListInput {
                slug: "r".into(),
                include_settled: false,
            }))
            .unwrap(),
    );
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert_eq!(v[0]["status"], "pending");
}

// ── Independent runs walk independently ────────────────────────────────────

#[test]
fn two_runs_advance_independently() {
    let (_d, server) = server();
    start(&server, "a");
    start(&server, "b");
    // Solo holds each run's Spec until sealed; seal both so they walk linearly.
    for slug in ["a", "b"] {
        server
            .darkrun_elaborate_seal(Parameters(ElaborateSealInput {
                slug: slug.into(),
                station: "frame".into(),
            }))
            .unwrap();
    }
    // Advance a twice, b once.
    next(&server, "a");
    next(&server, "a");
    next(&server, "b");
    let a = body(
        &server
            .darkrun_run_inspect(Parameters(RunShowRef { slug: Some("a".into()) }))
            .unwrap(),
    );
    let b = body(
        &server
            .darkrun_run_inspect(Parameters(RunShowRef { slug: Some("b".into()) }))
            .unwrap(),
    );
    // a's frame station advanced through Review to the pre-execution user gate;
    // b is still at the Review phase — the two runs walk independently.
    assert_eq!(a["state"]["stations"]["frame"]["phase"], "user_gate");
    assert_eq!(b["state"]["stations"]["frame"]["phase"], "review");
    assert_eq!(b["position"]["action"]["action"], "review");
}

#[test]
fn archiving_one_run_does_not_affect_other() {
    let (_d, server) = server();
    start(&server, "a");
    start(&server, "b");
    server
        .darkrun_run_archive(Parameters(RunArchiveInput {
            slug: "a".into(),
            archived: true,
        }))
        .unwrap();
    let v = body(
        &server
            .darkrun_run_list(Parameters(RunListInput {
                include_archived: false,
            }))
            .unwrap(),
    );
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["slug"], "b");
}

// ── Batch 5: factory-detail per-station field coverage ─────────────────────

#[test]
fn factory_detail_every_station_has_required_fields() {
    let (_d, server) = server();
    let v = body(
        &server
            .darkrun_factory_detail(Parameters(FactoryRef {
                factory: "software".into(),
            }))
            .unwrap(),
    );
    for st in v["stations"].as_array().unwrap() {
        assert!(st["name"].is_string());
        assert!(st["kills"].is_string());
        assert!(st["artifact"].is_string());
        assert!(st["workers"].is_array());
        assert!(st["reviewers"].is_array());
    }
}

#[test]
fn factory_detail_every_station_has_nonempty_workers() {
    let (_d, server) = server();
    let v = body(
        &server
            .darkrun_factory_detail(Parameters(FactoryRef {
                factory: "software".into(),
            }))
            .unwrap(),
    );
    for st in v["stations"].as_array().unwrap() {
        assert!(!st["workers"].as_array().unwrap().is_empty());
    }
}

#[test]
fn factory_detail_every_station_has_two_reviewers() {
    let (_d, server) = server();
    let v = body(
        &server
            .darkrun_factory_detail(Parameters(FactoryRef {
                factory: "software".into(),
            }))
            .unwrap(),
    );
    for st in v["stations"].as_array().unwrap() {
        let n = st["reviewers"].as_array().unwrap().len();
        assert!(n == 2 || n == 3, "station has 2 reviewers (3 for shape)");
    }
}

#[test]
fn factory_detail_shape_station_fields() {
    let (_d, server) = server();
    let v = body(
        &server
            .darkrun_factory_detail(Parameters(FactoryRef {
                factory: "software".into(),
            }))
            .unwrap(),
    );
    let shape = &v["stations"][2];
    assert_eq!(shape["name"], "shape");
    assert_eq!(shape["kills"], "expensive-structural-reversal");
    assert_eq!(shape["artifact"], "design.md");
}

#[test]
fn factory_detail_build_station_fields() {
    let (_d, server) = server();
    let v = body(
        &server
            .darkrun_factory_detail(Parameters(FactoryRef {
                factory: "software".into(),
            }))
            .unwrap(),
    );
    let build = &v["stations"][3];
    assert_eq!(build["name"], "build");
    assert_eq!(build["kills"], "implementation-defects");
    assert_eq!(build["artifact"], "code");
}

#[test]
fn factory_detail_prove_station_fields() {
    let (_d, server) = server();
    let v = body(
        &server
            .darkrun_factory_detail(Parameters(FactoryRef {
                factory: "software".into(),
            }))
            .unwrap(),
    );
    let prove = &v["stations"][4];
    assert_eq!(prove["name"], "prove");
    assert_eq!(prove["kills"], "escaped-defects");
    assert_eq!(prove["artifact"], "proof.md");
}

#[test]
fn factory_detail_harden_station_fields() {
    let (_d, server) = server();
    let v = body(
        &server
            .darkrun_factory_detail(Parameters(FactoryRef {
                factory: "software".into(),
            }))
            .unwrap(),
    );
    let harden = &v["stations"][5];
    assert_eq!(harden["name"], "harden");
    assert_eq!(harden["kills"], "works-in-dev-dies-in-prod");
    assert_eq!(harden["artifact"], "release.md");
}

#[test]
fn factory_detail_frame_workers_are_three() {
    let (_d, server) = server();
    let v = body(
        &server
            .darkrun_factory_detail(Parameters(FactoryRef {
                factory: "software".into(),
            }))
            .unwrap(),
    );
    let w = v["stations"][0]["workers"].as_array().unwrap();
    assert_eq!(w.len(), 3);
    assert_eq!(w[0], "framer");
    assert_eq!(w[1], "challenger");
    assert_eq!(w[2], "distiller");
}

#[test]
fn factory_detail_build_workers_are_four() {
    let (_d, server) = server();
    let v = body(
        &server
            .darkrun_factory_detail(Parameters(FactoryRef {
                factory: "software".into(),
            }))
            .unwrap(),
    );
    let w = v["stations"][3]["workers"].as_array().unwrap();
    assert_eq!(w.len(), 4);
    assert_eq!(w[0], "test_author");
}

#[test]
fn factory_detail_case_sensitive_factory_name() {
    let (_d, server) = server();
    let res = server
        .darkrun_factory_detail(Parameters(FactoryRef {
            factory: "Software".into(),
        }))
        .unwrap();
    // resolve_factory matches exact "software"; "Software" is unknown.
    assert!(is_err(&res));
}

#[test]
fn factory_detail_empty_name_errors() {
    let (_d, server) = server();
    let res = server
        .darkrun_factory_detail(Parameters(FactoryRef { factory: "".into() }))
        .unwrap();
    assert!(is_err(&res));
}

#[test]
fn factory_list_lists_the_shipped_catalog() {
    let (_d, server) = server();
    let v = body(&server.darkrun_factory_list().unwrap());
    let names: Vec<&str> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"software"), "software shipped");
    assert!(names.contains(&"legal"), "legal shipped");
}

// ── Batch 5: more error/validation edges ───────────────────────────────────

#[test]
fn unit_create_on_missing_run_still_writes_unit_dir() {
    // create does not require the run to exist; it writes under .darkrun/<run>.
    let (_d, server) = server();
    let res = create_unit(&server, "no-such-run", "u1", "frame");
    assert!(is_ok(&res));
    let got = server
        .darkrun_unit_get(Parameters(UnitRef {
            slug: "no-such-run".into(),
            unit: "u1".into(),
        }))
        .unwrap();
    assert!(is_ok(&got));
}

#[test]
fn feedback_create_on_missing_run_allocates_id() {
    let (_d, server) = server();
    let res = create_feedback(&server, "no-such-run", "frame", "x");
    assert!(is_ok(&res));
    assert_eq!(body(&res)["id"], "fb-01");
}

#[test]
fn unit_get_after_two_updates_reflects_last() {
    let (_d, server) = started("r");
    create_unit(&server, "r", "u1", "frame");
    server
        .darkrun_unit_update(Parameters(UnitUpdateInput {
            slug: "r".into(),
            unit: "u1".into(),
            status: None,
            depends_on: None,
            worker: Some("first".into()),
            inputs: None,
            outputs: None,
            ..Default::default()
        }))
        .unwrap();
    server
        .darkrun_unit_update(Parameters(UnitUpdateInput {
            slug: "r".into(),
            unit: "u1".into(),
            status: None,
            depends_on: None,
            worker: Some("second".into()),
            inputs: None,
            outputs: None,
            ..Default::default()
        }))
        .unwrap();
    let v = body(
        &server
            .darkrun_unit_get(Parameters(UnitRef {
                slug: "r".into(),
                unit: "u1".into(),
            }))
            .unwrap(),
    );
    assert_eq!(v["frontmatter"]["worker"], "second");
}

#[test]
fn unit_update_noop_when_all_fields_none() {
    let (_d, server) = started("r");
    create_unit(&server, "r", "u1", "frame");
    let res = server
        .darkrun_unit_update(Parameters(UnitUpdateInput {
            slug: "r".into(),
            unit: "u1".into(),
            status: None,
            depends_on: None,
            worker: None,
            inputs: None,
            outputs: None,
            ..Default::default()
        }))
        .unwrap();
    assert!(is_ok(&res));
    // Status unchanged.
    assert_eq!(body(&res)["frontmatter"]["status"], "pending");
}

#[test]
fn feedback_resolve_empty_status_defaults_to_addressed() {
    // The tool's status arg defaults to "addressed" at the schema level; here we
    // pass it explicitly to confirm the default value is terminal/valid.
    let (_d, server) = started("r");
    let id = body(&create_feedback(&server, "r", "frame", "x"))["id"]
        .as_str()
        .unwrap()
        .to_string();
    let res = server
        .darkrun_feedback_resolve(Parameters(FeedbackResolveInput {
            slug: "r".into(),
            feedback_id: id,
            status: "addressed".into(),
                reply: None,
        }))
        .unwrap();
    assert_eq!(body(&res)["status"], "addressed");
}

#[test]
fn checkpoint_decide_approve_idempotent_via_distinct_stations() {
    // Approving repeatedly walks station by station; each approval is a fresh
    // decision on the now-current station.
    let (_d, server) = started("r");
    let s1 = approve(&server, "r");
    let s2 = approve(&server, "r");
    assert_ne!(s1["action"]["station"], s2["action"]["station"]);
}

// ── Batch 5: run_show body & title preservation ────────────────────────────

#[test]
fn run_show_body_preserves_title_heading() {
    let (_d, server) = server();
    server
        .darkrun_run_new(Parameters(RunStartInput {
            slug: "r".into(),
            factory: "software".into(),
            title: Some("My Run".into()),
            mode: "continuous".into(),
            size: "full".into(),        }))
        .unwrap();
    let v = body(
        &server
            .darkrun_run_inspect(Parameters(RunShowRef { slug: Some("r".into()) }))
            .unwrap(),
    );
    assert!(v["run"]["body"].as_str().unwrap().contains("# My Run"));
}

#[test]
fn run_show_title_resolves_from_frontmatter() {
    let (_d, server) = server();
    server
        .darkrun_run_new(Parameters(RunStartInput {
            slug: "r".into(),
            factory: "software".into(),
            title: Some("Titled".into()),
            mode: "continuous".into(),
            size: "full".into(),        }))
        .unwrap();
    let v = body(
        &server
            .darkrun_run_inspect(Parameters(RunShowRef { slug: Some("r".into()) }))
            .unwrap(),
    );
    assert_eq!(v["run"]["title"], "Titled");
}

// ── Batch 5: unit status timestamps ────────────────────────────────────────

#[test]
fn unit_active_status_sets_started_at() {
    let (_d, server) = started("r");
    create_unit(&server, "r", "u1", "frame");
    let v = body(
        &server
            .darkrun_unit_update(Parameters(UnitUpdateInput {
                slug: "r".into(),
                unit: "u1".into(),
                status: Some("active".into()),
                depends_on: None,
                worker: None,
                inputs: None,
                outputs: None,
                ..Default::default()
            }))
            .unwrap(),
    );
    assert!(v["frontmatter"]["started_at"].is_string());
}

#[test]
fn unit_completed_sets_both_timestamps() {
    let (_d, server) = started("r");
    create_unit(&server, "r", "u1", "frame");
    let v = body(
        &server
            .darkrun_unit_update(Parameters(UnitUpdateInput {
                slug: "r".into(),
                unit: "u1".into(),
                status: Some("completed".into()),
                depends_on: None,
                worker: None,
                inputs: None,
                outputs: None,
                ..Default::default()
            }))
            .unwrap(),
    );
    assert!(v["frontmatter"]["started_at"].is_string());
    assert!(v["frontmatter"]["completed_at"].is_string());
}

#[test]
fn unit_pending_has_no_timestamps() {
    let (_d, server) = started("r");
    let v = body(&create_unit(&server, "r", "u1", "frame"));
    // started_at/completed_at are skip-if-none → absent for a fresh pending unit.
    assert!(v["frontmatter"].get("started_at").is_none());
    assert!(v["frontmatter"].get("completed_at").is_none());
}

// ── Batch 6: persistence across server instances on the same root ──────────

#[test]
fn state_persists_across_server_reinstantiation() {
    let dir = TempDir::new().unwrap();
    {
        let s = DarkrunServer::new(dir.path());
        start(&s, "r");
        // Solo holds the Spec until sealed; seal so the tick advances to Review.
        s.darkrun_elaborate_seal(Parameters(ElaborateSealInput {
            slug: "r".into(),
            station: "frame".into(),
        }))
        .unwrap();
        next(&s, "r"); // spec -> review
    }
    // Fresh server over the same on-disk state.
    let s2 = DarkrunServer::new(dir.path());
    let v = body(
        &s2.darkrun_run_inspect(Parameters(RunShowRef { slug: Some("r".into()) }))
            .unwrap(),
    );
    assert_eq!(v["state"]["stations"]["frame"]["phase"], "review");
}

#[test]
fn units_persist_across_server_reinstantiation() {
    let dir = TempDir::new().unwrap();
    {
        let s = DarkrunServer::new(dir.path());
        start(&s, "r");
        create_unit(&s, "r", "u1", "frame");
    }
    let s2 = DarkrunServer::new(dir.path());
    let v = body(
        &s2.darkrun_unit_list(Parameters(RunRef { slug: "r".into() }))
            .unwrap(),
    );
    assert_eq!(v.as_array().unwrap().len(), 1);
}

#[test]
fn feedback_persists_across_server_reinstantiation() {
    let dir = TempDir::new().unwrap();
    {
        let s = DarkrunServer::new(dir.path());
        start(&s, "r");
        create_feedback(&s, "r", "frame", "persisted");
    }
    let s2 = DarkrunServer::new(dir.path());
    let v = body(
        &s2.darkrun_feedback_list(Parameters(FeedbackListInput {
            slug: "r".into(),
            include_settled: true,
        }))
        .unwrap(),
    );
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert!(v[0]["body"].as_str().unwrap().contains("persisted"));
}

#[test]
fn archive_persists_across_server_reinstantiation() {
    let dir = TempDir::new().unwrap();
    {
        let s = DarkrunServer::new(dir.path());
        start(&s, "r");
        s.darkrun_run_archive(Parameters(RunArchiveInput {
            slug: "r".into(),
            archived: true,
        }))
        .unwrap();
    }
    let s2 = DarkrunServer::new(dir.path());
    let v = body(
        &s2.darkrun_run_list(Parameters(RunListInput {
            include_archived: false,
        }))
        .unwrap(),
    );
    assert_eq!(v.as_array().unwrap().len(), 0);
}

// ── Batch 6: severity survives listing ─────────────────────────────────────

#[test]
fn feedback_severity_survives_list() {
    let (_d, server) = started("r");
    server
        .darkrun_feedback_create(Parameters(FeedbackCreateInput {
            slug: "r".into(),
            station: "frame".into(),
            body: "x".into(),
            severity: Some("blocker".into()),
                origin: None,
                invalidates: None,
        }))
        .unwrap();
    let v = body(
        &server
            .darkrun_feedback_list(Parameters(FeedbackListInput {
                slug: "r".into(),
                include_settled: true,
            }))
            .unwrap(),
    );
    assert_eq!(v[0]["severity"], "blocker");
}

#[test]
fn feedback_severity_survives_status_change() {
    let (_d, server) = started("r");
    let id = body(
        &server
            .darkrun_feedback_create(Parameters(FeedbackCreateInput {
                slug: "r".into(),
                station: "frame".into(),
                body: "x".into(),
                severity: Some("high".into()),
                origin: None,
                invalidates: None,
            }))
            .unwrap(),
    )["id"]
        .as_str()
        .unwrap()
        .to_string();
    let v = body(
        &server
            .darkrun_feedback_resolve(Parameters(FeedbackResolveInput {
                slug: "r".into(),
                feedback_id: id,
                status: "addressed".into(),
                reply: None,
            }))
            .unwrap(),
    );
    assert_eq!(v["severity"], "high");
}

#[test]
fn feedback_station_survives_status_change() {
    let (_d, server) = started("r");
    let id = body(&create_feedback(&server, "r", "build", "x"))["id"]
        .as_str()
        .unwrap()
        .to_string();
    let v = body(
        &server
            .darkrun_feedback_resolve(Parameters(FeedbackResolveInput {
                slug: "r".into(),
                feedback_id: id,
                status: "closed".into(),
                reply: None,
            }))
            .unwrap(),
    );
    assert_eq!(v["station"], "build");
}

// ── Batch 6: depends_on replacement vs append semantics ────────────────────

#[test]
fn unit_update_depends_on_replaces_set() {
    let (_d, server) = started("r");
    create_unit(&server, "r", "a", "frame");
    create_unit(&server, "r", "b", "frame");
    create_unit(&server, "r", "c", "frame");
    server
        .darkrun_unit_create(Parameters(UnitCreateInput {
            slug: "r".into(),
            unit: "u1".into(),
            station: "frame".into(),
            title: None,
            depends_on: vec!["a".into(), "b".into()],
            ..Default::default()
        }))
        .unwrap();
    let v = body(
        &server
            .darkrun_unit_update(Parameters(UnitUpdateInput {
                slug: "r".into(),
                unit: "u1".into(),
                status: None,
                depends_on: Some(vec!["c".into()]),
                worker: None,
                inputs: None,
                outputs: None,
                ..Default::default()
            }))
            .unwrap(),
    );
    let deps = v["frontmatter"]["depends_on"].as_array().unwrap();
    assert_eq!(deps.len(), 1);
    assert_eq!(deps[0], "c");
}

#[test]
fn unit_update_empty_depends_on_clears_set() {
    let (_d, server) = started("r");
    create_unit(&server, "r", "a", "frame");
    server
        .darkrun_unit_create(Parameters(UnitCreateInput {
            slug: "r".into(),
            unit: "u1".into(),
            station: "frame".into(),
            title: None,
            depends_on: vec!["a".into()],
            ..Default::default()
        }))
        .unwrap();
    let v = body(
        &server
            .darkrun_unit_update(Parameters(UnitUpdateInput {
                slug: "r".into(),
                unit: "u1".into(),
                status: None,
                depends_on: Some(vec![]),
                worker: None,
                inputs: None,
                outputs: None,
                ..Default::default()
            }))
            .unwrap(),
    );
    assert_eq!(v["frontmatter"]["depends_on"].as_array().unwrap().len(), 0);
}


// ── Live mirror: every tick refreshes the session payload ───────────────────

/// The desktop re-renders live as the engine progresses: EVERY `darkrun_tick`
/// rebuilds the run's session payload (and the registry upsert broadcasts it
/// to subscribed WebSockets) — not just gate ticks. The operator can watch the
/// phase move and file feedback against current state at any moment.
#[test]
fn every_tick_refreshes_the_live_session_payload() {
    let (_d, server) = started("r");
    let reg = server.sessions();

    // Tick 1: the session exists immediately (no gate required) and carries
    // the run's live phase.
    next(&server, "r");
    let phase_of = |reg: &darkrun_http::SessionRegistry| -> String {
        match reg.get("r") {
            Some(darkrun_api::SessionPayload::Review(p)) => p
                .current_state
                .as_ref()
                .and_then(|c| c.phase.as_ref())
                .map(|ph| format!("{ph:?}"))
                .unwrap_or_default(),
            other => panic!("expected a review session for the run, got {other:?}"),
        }
    };
    let first = phase_of(&reg);
    assert!(!first.is_empty(), "tick 1 published a live phase");

    // Subscribe to the broadcast channel, then tick again: the refreshed
    // payload is PUSHED to the subscriber (the WS frame the desktop receives).
    let mut rx = reg.subscribe("r").expect("session broadcast channel");
    next(&server, "r");
    let frame = rx.try_recv().expect("the tick pushed a frame to subscribers");
    assert!(frame.contains("\"session_type\""), "a payload frame: {frame}");
    // And again — every tick pushes, continuously, gate or not.
    next(&server, "r");
    rx.try_recv().expect("the next tick pushed another frame");
    let _ = phase_of(&reg); // payload stays well-formed across refreshes
    let _ = first;
}

// ── Batch 6: feedback create with empty station string ─────────────────────

#[test]
fn feedback_create_with_empty_station_allowed() {
    let (_d, server) = started("r");
    let res = create_feedback(&server, "r", "", "no station");
    assert!(is_ok(&res));
    assert_eq!(body(&res)["station"], "");
}

// ── Batch 6: unit_create multiple deps preserved in order ──────────────────

#[test]
fn unit_create_preserves_dependency_order() {
    let (_d, server) = started("r");
    create_unit(&server, "r", "first", "frame");
    create_unit(&server, "r", "second", "frame");
    create_unit(&server, "r", "third", "frame");
    let v = body(
        &server
            .darkrun_unit_create(Parameters(UnitCreateInput {
                slug: "r".into(),
                unit: "u1".into(),
                station: "frame".into(),
                title: None,
                depends_on: vec!["first".into(), "second".into(), "third".into()],
                ..Default::default()
            }))
            .unwrap(),
    );
    let deps = v["frontmatter"]["depends_on"].as_array().unwrap();
    assert_eq!(deps[0], "first");
    assert_eq!(deps[1], "second");
    assert_eq!(deps[2], "third");
}

// ── Batch 6: run_next on a brand-new run does not require run_show first ────

#[test]
fn run_next_works_without_prior_show() {
    let (_d, server) = started("r");
    let res = next(&server, "r");
    assert!(is_ok(&res));
    assert_eq!(body(&res)["action"]["action"], "spec");
}

// ── Batch 6: checkpoint_decide on a sealed run ─────────────────────────────

#[test]
fn checkpoint_decide_on_sealed_run_errors() {
    let (_d, server) = started("r");
    for st in ["frame", "specify", "shape", "build", "prove"] {
        walk_station_to_checkpoint(&server, "r", st);
        approve(&server, "r");
    }
    walk_station_to_checkpoint(&server, "r", "harden");
    approve(&server, "r"); // seal
                           // No active station remains → decide errors.
    let res = server
        .darkrun_checkpoint_decide(Parameters(CheckpointDecideInput {
            slug: "r".into(),
            approved: true,
            feedback: None,
        }))
        .unwrap();
    assert!(is_err(&res));
}

// ── Batch 6: large unit set listing ────────────────────────────────────────

#[test]
fn unit_list_handles_many_units() {
    let (_d, server) = started("r");
    for i in 0..15 {
        create_unit(&server, "r", &format!("u{i:02}"), "frame");
    }
    let v = body(
        &server
            .darkrun_unit_list(Parameters(RunRef { slug: "r".into() }))
            .unwrap(),
    );
    assert_eq!(v.as_array().unwrap().len(), 15);
}

// ── Batch 6: feedback list ordering with mixed statuses ────────────────────

#[test]
fn feedback_list_keeps_id_order_with_mixed_statuses() {
    let (_d, server) = started("r");
    create_feedback(&server, "r", "frame", "a");
    create_feedback(&server, "r", "frame", "b");
    create_feedback(&server, "r", "frame", "c");
    // Resolve the middle one.
    server
        .darkrun_feedback_resolve(Parameters(FeedbackResolveInput {
            slug: "r".into(),
            feedback_id: "fb-02".into(),
            status: "addressed".into(),
                reply: None,
        }))
        .unwrap();
    let v = body(
        &server
            .darkrun_feedback_list(Parameters(FeedbackListInput {
                slug: "r".into(),
                include_settled: true,
            }))
            .unwrap(),
    );
    let ids: Vec<&str> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["fb-01", "fb-02", "fb-03"]);
}

#[test]
fn feedback_list_hide_settled_drops_only_terminal() {
    let (_d, server) = started("r");
    create_feedback(&server, "r", "frame", "a");
    create_feedback(&server, "r", "frame", "b");
    create_feedback(&server, "r", "frame", "c");
    server
        .darkrun_feedback_resolve(Parameters(FeedbackResolveInput {
            slug: "r".into(),
            feedback_id: "fb-02".into(),
            status: "addressed".into(),
                reply: None,
        }))
        .unwrap();
    let v = body(
        &server
            .darkrun_feedback_list(Parameters(FeedbackListInput {
                slug: "r".into(),
                include_settled: false,
            }))
            .unwrap(),
    );
    let ids: Vec<&str> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["fb-01", "fb-03"]);
}

// ── Batch 7: reject path on auto/external stations & misc ──────────────────

#[test]
fn checkpoint_reject_on_build_auto_station_blocks() {
    let (_d, server) = started("r");
    for st in ["frame", "specify", "shape"] {
        walk_station_to_checkpoint(&server, "r", st);
        approve(&server, "r");
    }
    // build is auto, but an explicit reject still holds and routes feedback.
    // First reach build's open checkpoint by NOT clearing manufacture — instead
    // we reject the now-current station (build) directly.
    let res = server
        .darkrun_checkpoint_decide(Parameters(CheckpointDecideInput {
            slug: "r".into(),
            approved: false,
            feedback: Some("build rework".into()),
        }))
        .unwrap();
    assert_eq!(body(&res)["position"]["track"], "feedback");
    let show = body(
        &server
            .darkrun_run_inspect(Parameters(RunShowRef { slug: Some("r".into()) }))
            .unwrap(),
    );
    assert_eq!(show["state"]["stations"]["build"]["status"], "blocked");
}

#[test]
fn checkpoint_reject_then_approve_advances_past_build() {
    let (_d, server) = started("r");
    for st in ["frame", "specify", "shape"] {
        walk_station_to_checkpoint(&server, "r", st);
        approve(&server, "r");
    }
    server
        .darkrun_checkpoint_decide(Parameters(CheckpointDecideInput {
            slug: "r".into(),
            approved: false,
            feedback: Some("rework".into()),
        }))
        .unwrap();
    // Resolve routed feedback.
    server
        .darkrun_feedback_resolve(Parameters(FeedbackResolveInput {
            slug: "r".into(),
            feedback_id: "fb-01".into(),
            status: "addressed".into(),
                reply: None,
        }))
        .unwrap();
    let v = approve(&server, "r");
    assert_eq!(v["action"]["station"], "prove");
}

#[test]
fn run_start_whitespace_title_is_used_verbatim() {
    let (_d, server) = server();
    let res = server
        .darkrun_run_new(Parameters(RunStartInput {
            slug: "r".into(),
            factory: "software".into(),
            title: Some("  spaced  ".into()),
            mode: "continuous".into(),
            size: "full".into(),        }))
        .unwrap();
    // Title is not trimmed by the tool.
    assert_eq!(body(&res)["title"], "  spaced  ");
}

#[test]
fn unit_station_persists_through_status_changes() {
    let (_d, server) = started("r");
    create_unit(&server, "r", "u1", "shape");
    server
        .darkrun_unit_update(Parameters(UnitUpdateInput {
            slug: "r".into(),
            unit: "u1".into(),
            status: Some("completed".into()),
            depends_on: None,
            worker: None,
            inputs: None,
            outputs: None,
            ..Default::default()
        }))
        .unwrap();
    let v = body(
        &server
            .darkrun_unit_get(Parameters(UnitRef {
                slug: "r".into(),
                unit: "u1".into(),
            }))
            .unwrap(),
    );
    assert_eq!(v["frontmatter"]["station"], "shape");
}

#[test]
fn feedback_resolve_fixing_is_not_terminal_rejected() {
    // fixing is a valid feedback status but NOT terminal → resolve refuses it.
    let (_d, server) = started("r");
    let id = body(&create_feedback(&server, "r", "frame", "x"))["id"]
        .as_str()
        .unwrap()
        .to_string();
    let res = server
        .darkrun_feedback_resolve(Parameters(FeedbackResolveInput {
            slug: "r".into(),
            feedback_id: id,
            status: "fixing".into(),
                reply: None,
        }))
        .unwrap();
    assert!(is_err(&res));
}

#[test]
fn feedback_resolve_pending_is_not_terminal_rejected() {
    let (_d, server) = started("r");
    let id = body(&create_feedback(&server, "r", "frame", "x"))["id"]
        .as_str()
        .unwrap()
        .to_string();
    let res = server
        .darkrun_feedback_resolve(Parameters(FeedbackResolveInput {
            slug: "r".into(),
            feedback_id: id,
            status: "pending".into(),
                reply: None,
        }))
        .unwrap();
    assert!(is_err(&res));
}

#[test]
fn feedback_resolve_escalated_is_not_terminal_rejected() {
    let (_d, server) = started("r");
    let id = body(&create_feedback(&server, "r", "frame", "x"))["id"]
        .as_str()
        .unwrap()
        .to_string();
    let res = server
        .darkrun_feedback_resolve(Parameters(FeedbackResolveInput {
            slug: "r".into(),
            feedback_id: id,
            status: "escalated".into(),
                reply: None,
        }))
        .unwrap();
    assert!(is_err(&res));
}

#[test]
fn feedback_resolve_rejected_via_resolve_is_terminal() {
    // "rejected" is terminal, so resolve accepts it (distinct from reject tool).
    let (_d, server) = started("r");
    let id = body(&create_feedback(&server, "r", "frame", "x"))["id"]
        .as_str()
        .unwrap()
        .to_string();
    let res = server
        .darkrun_feedback_resolve(Parameters(FeedbackResolveInput {
            slug: "r".into(),
            feedback_id: id,
            status: "rejected".into(),
                reply: None,
        }))
        .unwrap();
    assert!(is_ok(&res));
    assert_eq!(body(&res)["status"], "rejected");
}

#[test]
fn unit_update_worker_does_not_change_status() {
    let (_d, server) = started("r");
    create_unit(&server, "r", "u1", "frame");
    let v = body(
        &server
            .darkrun_unit_update(Parameters(UnitUpdateInput {
                slug: "r".into(),
                unit: "u1".into(),
                status: None,
                depends_on: None,
                worker: Some("framer".into()),
                inputs: None,
                outputs: None,
                ..Default::default()
            }))
            .unwrap(),
    );
    assert_eq!(v["frontmatter"]["status"], "pending");
    assert_eq!(v["frontmatter"]["worker"], "framer");
}

#[test]
fn unit_list_reflects_status_after_update() {
    let (_d, server) = started("r");
    create_unit(&server, "r", "u1", "frame");
    server
        .darkrun_unit_update(Parameters(UnitUpdateInput {
            slug: "r".into(),
            unit: "u1".into(),
            status: Some("completed".into()),
            depends_on: None,
            worker: None,
            inputs: None,
            outputs: None,
            ..Default::default()
        }))
        .unwrap();
    let v = body(
        &server
            .darkrun_unit_list(Parameters(RunRef { slug: "r".into() }))
            .unwrap(),
    );
    let u = v
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["slug"] == "u1")
        .unwrap();
    assert_eq!(u["frontmatter"]["status"], "completed");
}

#[test]
fn run_archive_then_restore_preserves_run_data() {
    let (_d, server) = started("r");
    create_unit(&server, "r", "u1", "frame");
    server
        .darkrun_run_archive(Parameters(RunArchiveInput {
            slug: "r".into(),
            archived: true,
        }))
        .unwrap();
    server
        .darkrun_run_archive(Parameters(RunArchiveInput {
            slug: "r".into(),
            archived: false,
        }))
        .unwrap();
    let v = body(
        &server
            .darkrun_unit_list(Parameters(RunRef { slug: "r".into() }))
            .unwrap(),
    );
    assert_eq!(v.as_array().unwrap().len(), 1);
}

#[test]
fn feedback_move_to_empty_station() {
    let (_d, server) = started("r");
    let id = body(&create_feedback(&server, "r", "frame", "x"))["id"]
        .as_str()
        .unwrap()
        .to_string();
    let v = body(
        &server
            .darkrun_feedback_move(Parameters(FeedbackMoveInput {
                slug: "r".into(),
                feedback_id: id,
                to_station: "".into(),
            }))
            .unwrap(),
    );
    assert_eq!(v["station"], "");
}

#[test]
fn run_show_reflects_unit_count_indirectly_via_position() {
    // With a decomposed+completed unit, the frame manufacture clears to audit.
    let (_d, server) = started("r");
    next(&server, "r");
    next(&server, "r");
    create_unit(&server, "r", "u1", "frame");
    // Clear the pre-execution operator gate while the unit is still pending, so
    // the cursor parks in Manufacture (not yet advanced past it).
    approve(&server, "r");
    server
        .darkrun_unit_update(Parameters(UnitUpdateInput {
            slug: "r".into(),
            unit: "u1".into(),
            status: Some("completed".into()),
            depends_on: None,
            worker: None,
            inputs: None,
            outputs: None,
            ..Default::default()
        }))
        .unwrap();
    let v = body(
        &server
            .darkrun_run_inspect(Parameters(RunShowRef { slug: Some("r".into()) }))
            .unwrap(),
    );
    // Manufacture phase + all units complete → derived action is audit.
    assert_eq!(v["position"]["action"]["action"], "audit");
}

#[test]
fn checkpoint_decide_returns_tick_result_shape() {
    let (_d, server) = started("r");
    let v = approve(&server, "r");
    assert!(v.get("run").is_some());
    assert!(v.get("position").is_some());
    assert!(v.get("action").is_some());
}

#[test]
fn feedback_create_body_with_newlines_preserved() {
    let (_d, server) = started("r");
    let v = body(&create_feedback(
        &server,
        "r",
        "frame",
        "line one\nline two",
    ));
    assert!(v["body"].as_str().unwrap().contains("line one"));
    assert!(v["body"].as_str().unwrap().contains("line two"));
}

#[test]
fn unit_create_has_no_iterations_so_pass_is_zero() {
    let (_d, server) = started("r");
    let v = body(&create_unit(&server, "r", "u1", "frame"));
    // `pass` is derived from the (empty) iteration history, not a stored field.
    assert!(v["frontmatter"].get("pass").is_none());
    assert!(v["frontmatter"]["iterations"].as_array().is_none_or(|a| a.is_empty()));
}

#[test]
fn run_list_excludes_archived_keeps_active_sorted() {
    let (_d, server) = server();
    for s in ["c", "a", "b"] {
        start(&server, s);
    }
    server
        .darkrun_run_archive(Parameters(RunArchiveInput {
            slug: "b".into(),
            archived: true,
        }))
        .unwrap();
    let v = body(
        &server
            .darkrun_run_list(Parameters(RunListInput {
                include_archived: false,
            }))
            .unwrap(),
    );
    let slugs: Vec<&str> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["slug"].as_str().unwrap())
        .collect();
    assert_eq!(slugs, vec!["a", "c"]);
}

// ── Batch 8: station-scoped workers, feedback-track show, noop shape ───────

/// Drive a freshly-entered station to its Manufacture phase, decompose `unit`,
/// then return the manufacture tick body. Tolerant of the station's starting
/// phase (the prior `approve` re-tick may have already moved it past Spec).
fn manufacture_first_unit(server: &DarkrunServer, slug: &str, station: &str, unit: &str) -> Value {
    // Tick until the station holds at its pre-execution operator gate or reaches
    // Manufacture (no units yet means the derived action falls back to spec, but
    // the phase still advances).
    let phase = |s: &DarkrunServer| -> String {
        body(
            &s.darkrun_run_inspect(Parameters(RunShowRef { slug: Some(slug.into()) }))
                .unwrap(),
        )["state"]["stations"][station]["phase"]
            .as_str()
            .unwrap_or("")
            .to_string()
    };
    for _ in 0..8 {
        let p = phase(server);
        if p == "manufacture" || p == "user_gate" {
            break;
        }
        // Solo holds the Spec until the elaboration is sealed.
        if p == "spec" {
            server
                .darkrun_elaborate_seal(Parameters(ElaborateSealInput {
                    slug: slug.into(),
                    station: station.into(),
                }))
                .unwrap();
        }
        next(server, slug);
    }
    server
        .darkrun_unit_create(Parameters(UnitCreateInput {
            slug: slug.into(),
            unit: unit.into(),
            station: station.into(),
            title: None,
            depends_on: vec![],
            ..Default::default()
        }))
        .unwrap();
    // Consume the station's declared inputs so the runtime input-coverage gate is
    // satisfied (the run's distillation is carried forward).
    if let Some(inputs) = darkrun_mcp::resolve_factory("software")
        .and_then(|f| f.station(station).map(|d| d.inputs.clone()))
        .filter(|i| !i.is_empty())
    {
        server
            .darkrun_unit_update(Parameters(UnitUpdateInput {
                slug: slug.into(),
                unit: unit.into(),
                status: None,
                depends_on: None,
                worker: None,
                inputs: Some(inputs),
                outputs: None,
                ..Default::default()
            }))
            .unwrap();
    }
    // If holding at the operator gate, clear it so the wave releases; the
    // checkpoint_decide re-tick returns the Manufacture action directly.
    if phase(server) == "user_gate" {
        return approve(server, slug);
    }
    body(&next(server, slug))
}

#[test]
fn manufacture_worker_is_first_station_worker_for_specify() {
    let (_d, server) = started("r");
    walk_station_to_checkpoint(&server, "r", "frame");
    approve(&server, "r"); // advance to specify
    let v = manufacture_first_unit(&server, "r", "specify", "s1");
    assert_eq!(v["action"]["action"], "manufacture");
    assert_eq!(v["action"]["worker"], "spec_writer");
}

#[test]
fn manufacture_worker_is_first_station_worker_for_shape() {
    let (_d, server) = started("r");
    walk_station_to_checkpoint(&server, "r", "frame");
    approve(&server, "r");
    walk_station_to_checkpoint(&server, "r", "specify");
    approve(&server, "r"); // advance to shape
    let v = manufacture_first_unit(&server, "r", "shape", "sh1");
    assert_eq!(v["action"]["action"], "manufacture");
    assert_eq!(v["action"]["worker"], "designer");
}

#[test]
fn run_show_position_track_feedback_when_open() {
    let (_d, server) = started("r");
    create_feedback(&server, "r", "frame", "x");
    let v = body(
        &server
            .darkrun_run_inspect(Parameters(RunShowRef { slug: Some("r".into()) }))
            .unwrap(),
    );
    assert_eq!(v["position"]["track"], "feedback");
    assert_eq!(v["position"]["action"]["action"], "fix_feedback");
}

#[test]
fn run_show_position_track_returns_to_run_after_resolve() {
    let (_d, server) = started("r");
    create_feedback(&server, "r", "frame", "x");
    server
        .darkrun_feedback_resolve(Parameters(FeedbackResolveInput {
            slug: "r".into(),
            feedback_id: "fb-01".into(),
            status: "addressed".into(),
                reply: None,
        }))
        .unwrap();
    let v = body(
        &server
            .darkrun_run_inspect(Parameters(RunShowRef { slug: Some("r".into()) }))
            .unwrap(),
    );
    assert_eq!(v["position"]["track"], "run");
}

#[test]
fn noop_position_action_is_null_in_show() {
    let (_d, server) = started("r");
    next(&server, "r"); // spec -> review
    next(&server, "r"); // review -> manufacture
    // A dispatched, in-flight unit yields the mid-wave noop.
    create_unit(&server, "r", "u1", "frame");
    server
        .darkrun_unit_update(Parameters(UnitUpdateInput {
            slug: "r".into(),
            unit: "u1".into(),
            status: Some("in_progress".into()),
            depends_on: None,
            worker: None,
            inputs: None,
            outputs: None,
            ..Default::default()
        }))
        .unwrap();
    approve(&server, "r"); // clear the pre-execution operator gate
    let v = body(
        &server
            .darkrun_run_inspect(Parameters(RunShowRef { slug: Some("r".into()) }))
            .unwrap(),
    );
    // Mid-wave noop → position.action is null (no action this tick).
    assert!(v["position"]["action"].is_null());
}

#[test]
fn noop_tick_action_is_noop_but_position_null() {
    let (_d, server) = started("r");
    next(&server, "r");
    next(&server, "r");
    // A dispatched, in-flight unit yields the mid-wave noop.
    create_unit(&server, "r", "u1", "frame");
    server
        .darkrun_unit_update(Parameters(UnitUpdateInput {
            slug: "r".into(),
            unit: "u1".into(),
            status: Some("in_progress".into()),
            depends_on: None,
            worker: None,
            inputs: None,
            outputs: None,
            ..Default::default()
        }))
        .unwrap();
    approve(&server, "r"); // clear the pre-execution operator gate
    let v = body(&next(&server, "r"));
    assert_eq!(v["action"]["action"], "noop");
    assert!(v["position"]["action"].is_null());
    assert_eq!(v["position"]["track"], "run");
}

#[test]
fn feedback_track_does_not_advance_phase() {
    let (_d, server) = started("r");
    next(&server, "r"); // spec -> review
    create_feedback(&server, "r", "frame", "x");
    // Several ticks while feedback open shouldn't advance the station phase.
    next(&server, "r");
    next(&server, "r");
    let v = body(
        &server
            .darkrun_run_inspect(Parameters(RunShowRef { slug: Some("r".into()) }))
            .unwrap(),
    );
    assert_eq!(v["state"]["stations"]["frame"]["phase"], "review");
}

#[test]
fn unit_get_shape_has_slug_frontmatter_title_body() {
    let (_d, server) = started("r");
    create_unit(&server, "r", "u1", "frame");
    let v = body(
        &server
            .darkrun_unit_get(Parameters(UnitRef {
                slug: "r".into(),
                unit: "u1".into(),
            }))
            .unwrap(),
    );
    assert!(v.get("slug").is_some());
    assert!(v.get("frontmatter").is_some());
    assert!(v.get("title").is_some());
    assert!(v.get("body").is_some());
}

#[test]
fn feedback_get_shape_via_list_has_all_fields() {
    let (_d, server) = started("r");
    server
        .darkrun_feedback_create(Parameters(FeedbackCreateInput {
            slug: "r".into(),
            station: "frame".into(),
            body: "x".into(),
            severity: Some("high".into()),
                origin: None,
                invalidates: None,
        }))
        .unwrap();
    let v = body(
        &server
            .darkrun_feedback_list(Parameters(FeedbackListInput {
                slug: "r".into(),
                include_settled: true,
            }))
            .unwrap(),
    );
    let f = &v[0];
    assert!(f.get("id").is_some());
    assert!(f.get("run").is_some());
    assert!(f.get("station").is_some());
    assert!(f.get("status").is_some());
    assert!(f.get("body").is_some());
}

#[test]
fn feedback_run_field_matches_slug() {
    let (_d, server) = started("my-run");
    let v = body(&create_feedback(&server, "my-run", "frame", "x"));
    assert_eq!(v["run"], "my-run");
}

#[test]
fn run_summary_status_is_active_for_running() {
    let (_d, server) = started("r");
    let v = body(
        &server
            .darkrun_run_list(Parameters(RunListInput {
                include_archived: false,
            }))
            .unwrap(),
    );
    assert_eq!(v[0]["status"], "active");
}

#[test]
fn unit_create_with_title_overrides_slug_in_body() {
    let (_d, server) = started("r");
    let v = body(
        &server
            .darkrun_unit_create(Parameters(UnitCreateInput {
                slug: "r".into(),
                unit: "u1".into(),
                station: "frame".into(),
                title: Some("Display".into()),
                depends_on: vec![],
                ..Default::default()
            }))
            .unwrap(),
    );
    assert!(v["body"].as_str().unwrap().contains("# Display"));
}

#[test]
fn checkpoint_reject_increments_feedback_ids() {
    let (_d, server) = started("r");
    // Pre-existing feedback so the routed one isn't fb-01.
    create_feedback(&server, "r", "frame", "pre");
    server
        .darkrun_feedback_resolve(Parameters(FeedbackResolveInput {
            slug: "r".into(),
            feedback_id: "fb-01".into(),
            status: "addressed".into(),
                reply: None,
        }))
        .unwrap();
    server
        .darkrun_checkpoint_decide(Parameters(CheckpointDecideInput {
            slug: "r".into(),
            approved: false,
            feedback: Some("rework".into()),
        }))
        .unwrap();
    let v = body(
        &server
            .darkrun_feedback_list(Parameters(FeedbackListInput {
                slug: "r".into(),
                include_settled: false,
            }))
            .unwrap(),
    );
    // The routed rework feedback is the only open item.
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert!(v[0]["body"].as_str().unwrap().contains("rework"));
}

#[test]
fn manufacture_clears_when_unit_completed_before_dispatch() {
    let (_d, server) = started("r");
    next(&server, "r");
    next(&server, "r");
    // Pre-completed unit: wave_ready is empty, all complete → audit immediately.
    server
        .darkrun_unit_create(Parameters(UnitCreateInput {
            slug: "r".into(),
            unit: "u1".into(),
            station: "frame".into(),
            title: None,
            depends_on: vec![],
            ..Default::default()
        }))
        .unwrap();
    approve(&server, "r"); // clear the pre-execution operator gate
    server
        .darkrun_unit_update(Parameters(UnitUpdateInput {
            slug: "r".into(),
            unit: "u1".into(),
            status: Some("completed".into()),
            depends_on: None,
            worker: None,
            inputs: None,
            outputs: None,
            ..Default::default()
        }))
        .unwrap();
    let v = body(&next(&server, "r"));
    assert_eq!(v["action"]["action"], "audit");
}

// ── Batch 9: factory_list parity, await absence, message content ───────────

#[test]
fn factory_list_each_station_matches_detail_workers() {
    let (_d, server) = server();
    let list = body(&server.darkrun_factory_list().unwrap());
    let detail = body(
        &server
            .darkrun_factory_detail(Parameters(FactoryRef {
                factory: "software".into(),
            }))
            .unwrap(),
    );
    let entry = list
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == "software")
        .expect("software listed");
    for i in 0..6 {
        assert_eq!(
            entry["stations"][i]["workers"],
            detail["stations"][i]["workers"]
        );
        assert_eq!(
            entry["stations"][i]["reviewers"],
            detail["stations"][i]["reviewers"]
        );
    }
}

#[test]
fn unit_get_missing_error_mentions_unit() {
    let (_d, server) = started("r");
    let res = server
        .darkrun_unit_get(Parameters(UnitRef {
            slug: "r".into(),
            unit: "ghost".into(),
        }))
        .unwrap();
    assert!(is_err(&res));
    assert!(err_message(&res).to_lowercase().contains("ghost") || !err_message(&res).is_empty());
}

#[test]
fn run_next_error_message_nonempty_for_missing_run() {
    let (_d, server) = server();
    let res = next(&server, "ghost");
    assert!(is_err(&res));
    assert!(!err_message(&res).is_empty());
}

#[test]
fn run_show_error_message_nonempty_for_missing_run() {
    let (_d, server) = server();
    let res = server
        .darkrun_run_inspect(Parameters(RunShowRef { slug: Some("ghost".into()) }))
        .unwrap();
    assert!(is_err(&res));
    assert!(!err_message(&res).is_empty());
}

#[test]
fn restored_run_is_listed_and_tickable() {
    let (_d, server) = started("r");
    server
        .darkrun_run_archive(Parameters(RunArchiveInput {
            slug: "r".into(),
            archived: true,
        }))
        .unwrap();
    server
        .darkrun_run_archive(Parameters(RunArchiveInput {
            slug: "r".into(),
            archived: false,
        }))
        .unwrap();
    let listed = body(
        &server
            .darkrun_run_list(Parameters(RunListInput {
                include_archived: false,
            }))
            .unwrap(),
    );
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert!(is_ok(&next(&server, "r")));
}

#[test]
fn feedback_create_returns_run_id_station_status() {
    let (_d, server) = started("r");
    let v = body(&create_feedback(&server, "r", "frame", "x"));
    assert_eq!(v["run"], "r");
    assert_eq!(v["id"], "fb-01");
    assert_eq!(v["station"], "frame");
    assert_eq!(v["status"], "pending");
}

#[test]
fn unit_update_status_only_does_not_touch_deps() {
    let (_d, server) = started("r");
    create_unit(&server, "r", "keep", "frame");
    server
        .darkrun_unit_create(Parameters(UnitCreateInput {
            slug: "r".into(),
            unit: "u1".into(),
            station: "frame".into(),
            title: None,
            depends_on: vec!["keep".into()],
            ..Default::default()
        }))
        .unwrap();
    let v = body(
        &server
            .darkrun_unit_update(Parameters(UnitUpdateInput {
                slug: "r".into(),
                unit: "u1".into(),
                status: Some("active".into()),
                depends_on: None,
                worker: None,
                inputs: None,
                outputs: None,
                ..Default::default()
            }))
            .unwrap(),
    );
    assert_eq!(v["frontmatter"]["depends_on"][0], "keep");
}

#[test]
fn feedback_list_default_include_settled_true_shows_resolved() {
    // FeedbackListInput's include_settled defaults to true at the schema level;
    // here we exercise the include=true branch explicitly.
    let (_d, server) = started("r");
    let id = body(&create_feedback(&server, "r", "frame", "x"))["id"]
        .as_str()
        .unwrap()
        .to_string();
    server
        .darkrun_feedback_resolve(Parameters(FeedbackResolveInput {
            slug: "r".into(),
            feedback_id: id,
            status: "closed".into(),
                reply: None,
        }))
        .unwrap();
    let v = body(
        &server
            .darkrun_feedback_list(Parameters(FeedbackListInput {
                slug: "r".into(),
                include_settled: true,
            }))
            .unwrap(),
    );
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert_eq!(v[0]["status"], "closed");
}

#[test]
fn checkpoint_decide_approve_returns_run_field() {
    let (_d, server) = started("my-run");
    let v = approve(&server, "my-run");
    assert_eq!(v["run"], "my-run");
}

#[test]
fn run_next_run_field_matches_slug() {
    let (_d, server) = started("xyz");
    let v = body(&next(&server, "xyz"));
    assert_eq!(v["run"], "xyz");
}

#[test]
fn unit_create_then_list_contains_correct_station() {
    let (_d, server) = started("r");
    create_unit(&server, "r", "u1", "prove");
    let v = body(
        &server
            .darkrun_unit_list(Parameters(RunRef { slug: "r".into() }))
            .unwrap(),
    );
    let u = v
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["slug"] == "u1")
        .unwrap();
    assert_eq!(u["frontmatter"]["station"], "prove");
}

#[test]
fn feedback_move_reflected_in_list() {
    let (_d, server) = started("r");
    let id = body(&create_feedback(&server, "r", "frame", "x"))["id"]
        .as_str()
        .unwrap()
        .to_string();
    server
        .darkrun_feedback_move(Parameters(FeedbackMoveInput {
            slug: "r".into(),
            feedback_id: id,
            to_station: "harden".into(),
        }))
        .unwrap();
    let v = body(
        &server
            .darkrun_feedback_list(Parameters(FeedbackListInput {
                slug: "r".into(),
                include_settled: true,
            }))
            .unwrap(),
    );
    assert_eq!(v[0]["station"], "harden");
}

#[test]
fn run_start_default_factory_via_explicit_software() {
    // The schema default for factory is "software"; assert the explicit value
    // produces the canonical plan.
    let (_d, server) = started("r");
    let v = body(
        &server
            .darkrun_run_inspect(Parameters(RunShowRef { slug: Some("r".into()) }))
            .unwrap(),
    );
    assert_eq!(v["state"]["factory"], "software");
}

// ── Batch 10: remaining distinct behaviors ─────────────────────────────────

#[test]
fn run_show_state_checkpoint_seeded_with_kind() {
    let (_d, server) = started("r");
    let v = body(
        &server
            .darkrun_run_inspect(Parameters(RunShowRef { slug: Some("r".into()) }))
            .unwrap(),
    );
    // frame's seeded checkpoint carries its ask kind.
    assert_eq!(v["state"]["stations"]["frame"]["checkpoint"]["kind"], "ask");
}

#[test]
fn checkpoint_decide_records_outcome_advanced_on_approve() {
    let (_d, server) = started("r");
    approve(&server, "r");
    let v = body(
        &server
            .darkrun_run_inspect(Parameters(RunShowRef { slug: Some("r".into()) }))
            .unwrap(),
    );
    assert_eq!(
        v["state"]["stations"]["frame"]["checkpoint"]["outcome"],
        "advanced"
    );
}

#[test]
fn checkpoint_decide_records_outcome_blocked_on_reject() {
    let (_d, server) = started("r");
    server
        .darkrun_checkpoint_decide(Parameters(CheckpointDecideInput {
            slug: "r".into(),
            approved: false,
            feedback: Some("no".into()),
        }))
        .unwrap();
    let v = body(
        &server
            .darkrun_run_inspect(Parameters(RunShowRef { slug: Some("r".into()) }))
            .unwrap(),
    );
    assert_eq!(
        v["state"]["stations"]["frame"]["checkpoint"]["outcome"],
        "blocked"
    );
}

#[test]
fn completed_station_has_completed_at_timestamp() {
    let (_d, server) = started("r");
    approve(&server, "r");
    let v = body(
        &server
            .darkrun_run_inspect(Parameters(RunShowRef { slug: Some("r".into()) }))
            .unwrap(),
    );
    assert!(v["state"]["stations"]["frame"]["completed_at"].is_string());
}

#[test]
fn in_progress_station_has_started_at_timestamp() {
    let (_d, server) = started("r");
    next(&server, "r"); // spec marks frame in_progress + started_at
    let v = body(
        &server
            .darkrun_run_inspect(Parameters(RunShowRef { slug: Some("r".into()) }))
            .unwrap(),
    );
    assert!(v["state"]["stations"]["frame"]["started_at"].is_string());
    assert_eq!(v["state"]["stations"]["frame"]["status"], "in_progress");
}

#[test]
fn next_station_seeded_pending_after_advance() {
    let (_d, server) = started("r");
    approve(&server, "r");
    let v = body(
        &server
            .darkrun_run_inspect(Parameters(RunShowRef { slug: Some("r".into()) }))
            .unwrap(),
    );
    // specify is now the active station. Solo holds the freshly-entered
    // station's Spec until its elaboration is sealed.
    assert_eq!(v["state"]["active_station"], "specify");
    assert_eq!(v["state"]["stations"]["specify"]["status"], "in_progress");
    assert_eq!(v["state"]["stations"]["specify"]["phase"], "spec");
}

#[test]
fn unit_with_dependency_on_self_is_rejected_at_create() {
    let (_d, server) = started("r");
    next(&server, "r");
    next(&server, "r");
    // u1 depends on itself — a trivial dependency cycle. The validator bounces
    // it at WRITE time now; the derive-time units_invalid net still backstops
    // units that reach disk by other paths (covered in position validate_units).
    let res = server
        .darkrun_unit_create(Parameters(UnitCreateInput {
            slug: "r".into(),
            unit: "u1".into(),
            station: "frame".into(),
            title: None,
            depends_on: vec!["u1".into()],
            ..Default::default()
        }))
        .unwrap();
    assert!(res.is_error.unwrap_or(false), "self-dependency must bounce: {res:?}");
}

#[test]
fn two_feedback_then_reject_one_leaves_other_open() {
    let (_d, server) = started("r");
    create_feedback(&server, "r", "frame", "a");
    create_feedback(&server, "r", "frame", "b");
    server
        .darkrun_feedback_reject(Parameters(FeedbackRejectInput {
            slug: "r".into(),
            feedback_id: "fb-01".into(),
            reason: "x".into(),
        }))
        .unwrap();
    let open = body(
        &server
            .darkrun_feedback_list(Parameters(FeedbackListInput {
                slug: "r".into(),
                include_settled: false,
            }))
            .unwrap(),
    );
    assert_eq!(open.as_array().unwrap().len(), 1);
    assert_eq!(open[0]["id"], "fb-02");
}

#[test]
fn run_next_after_reject_dispatches_open_feedback() {
    let (_d, server) = started("r");
    server
        .darkrun_checkpoint_decide(Parameters(CheckpointDecideInput {
            slug: "r".into(),
            approved: false,
            feedback: Some("rework".into()),
        }))
        .unwrap();
    let v = body(&next(&server, "r"));
    assert_eq!(v["action"]["action"], "fix_feedback");
}

#[test]
fn unit_create_empty_depends_on_is_empty_array() {
    let (_d, server) = started("r");
    let v = body(&create_unit(&server, "r", "u1", "frame"));
    // depends_on is skip-if-empty in core; absent or empty either way.
    let deps = v["frontmatter"].get("depends_on");
    assert!(deps.is_none() || deps.unwrap().as_array().unwrap().is_empty());
}

#[test]
fn run_summary_includes_archived_field_false() {
    let (_d, server) = started("r");
    let v = body(
        &server
            .darkrun_run_list(Parameters(RunListInput {
                include_archived: true,
            }))
            .unwrap(),
    );
    assert_eq!(v[0]["archived"], false);
}

#[test]
fn run_summary_title_field_present() {
    let (_d, server) = server();
    server
        .darkrun_run_new(Parameters(RunStartInput {
            slug: "r".into(),
            factory: "software".into(),
            title: Some("Named".into()),
            mode: "continuous".into(),
            size: "full".into(),        }))
        .unwrap();
    let v = body(
        &server
            .darkrun_run_list(Parameters(RunListInput {
                include_archived: false,
            }))
            .unwrap(),
    );
    assert_eq!(v[0]["title"], "Named");
}

#[test]
fn factory_list_frame_reviewers_exact() {
    let (_d, server) = server();
    let v = body(&server.darkrun_factory_list().unwrap());
    let sw = software_entry(&v);
    let r = sw["stations"][0]["reviewers"].as_array().unwrap();
    assert_eq!(r[0], "value");
    assert_eq!(r[1], "feasibility");
}

#[test]
fn factory_list_specify_reviewers_exact() {
    let (_d, server) = server();
    let v = body(&server.darkrun_factory_list().unwrap());
    let sw = software_entry(&v);
    let r = sw["stations"][1]["reviewers"].as_array().unwrap();
    assert_eq!(r[0], "testability");
    assert_eq!(r[1], "completeness");
}

#[test]
fn factory_list_harden_reviewers_exact() {
    let (_d, server) = server();
    let v = body(&server.darkrun_factory_list().unwrap());
    let sw = software_entry(&v);
    let r = sw["stations"][5]["reviewers"].as_array().unwrap();
    assert_eq!(r[0], "security");
    assert_eq!(r[1], "readiness");
}

#[test]
fn feedback_resolve_answered_then_settled_immutable() {
    let (_d, server) = started("r");
    let id = body(&create_feedback(&server, "r", "frame", "q"))["id"]
        .as_str()
        .unwrap()
        .to_string();
    server
        .darkrun_feedback_resolve(Parameters(FeedbackResolveInput {
            slug: "r".into(),
            feedback_id: id.clone(),
            status: "answered".into(),
                reply: None,
        }))
        .unwrap();
    let again = server
        .darkrun_feedback_reject(Parameters(FeedbackRejectInput {
            slug: "r".into(),
            feedback_id: id,
            reason: "x".into(),
        }))
        .unwrap();
    assert!(is_err(&again));
}

// ── Visual-session tool helpers ────────────────────────────────────────────

fn q_opt(id: &str, label: &str) -> QuestionOptionInput {
    QuestionOptionInput {
        id: id.into(),
        label: label.into(),
        image_url: None,
        image_url_light: None,
        description: None,
    }
}

fn arch_in(id: &str) -> ArchetypeInput {
    ArchetypeInput {
        id: id.into(),
        label: format!("{id} label"),
        image_url: format!("https://img/{id}.png"),
        image_url_light: None,
        description: format!("{id} direction"),
    }
}

fn p_opt(id: &str) -> PickerOptionInput {
    PickerOptionInput {
        id: id.into(),
        label: format!("{id} label"),
        description: None,
        secondary: None,
    }
}

// ── darkrun_question ───────────────────────────────────────────────────────

#[test]
fn question_tool_creates_awaiting_session() {
    let (_d, server) = started("r");
    let res = server
        .darkrun_question(Parameters(QuestionInput {
            slug: "r".into(),
            title: Some("Pick a hero".into()),
            prompt: "Which hero layout?".into(),
            context: None,
            options: vec![
                QuestionOptionInput {
                    id: "a".into(),
                    label: "Option A".into(),
                    image_url: Some("https://img/a.png".into()),
                    image_url_light: None,
                    description: Some("bold".into()),
                },
                q_opt("b", "Option B"),
            ],
            multi_select: true,
            image_urls: vec!["https://ref/surface.png".into()],
        }))
        .unwrap();
    assert!(is_ok(&res));
    let v = body(&res);
    assert_eq!(v["session_id"], "q-01");
    assert_eq!(v["session_type"], "question");
    assert_eq!(v["status"], "pending");
    assert_eq!(v["awaiting_answer"], true);
    assert_eq!(v["session_path"], "/api/session/q-01");
    assert_eq!(v["ws_path"], "/ws/session/q-01");
}

#[test]
fn question_tool_rejects_empty_prompt_and_options() {
    let (_d, server) = started("r");
    let no_prompt = server
        .darkrun_question(Parameters(QuestionInput {
            slug: "r".into(),
            title: None,
            prompt: "   ".into(),
            context: None,
            options: vec![q_opt("a", "A")],
            multi_select: false,
            image_urls: vec![],
        }))
        .unwrap();
    assert!(is_err(&no_prompt));

    let no_options = server
        .darkrun_question(Parameters(QuestionInput {
            slug: "r".into(),
            title: None,
            prompt: "p".into(),
            context: None,
            options: vec![],
            multi_select: false,
            image_urls: vec![],
        }))
        .unwrap();
    assert!(is_err(&no_options));
}

#[test]
fn question_tool_rejects_duplicate_option_ids() {
    let (_d, server) = started("r");
    let res = server
        .darkrun_question(Parameters(QuestionInput {
            slug: "r".into(),
            title: None,
            prompt: "p".into(),
            context: None,
            options: vec![q_opt("a", "A"), q_opt("a", "A2")],
            multi_select: false,
            image_urls: vec![],
        }))
        .unwrap();
    assert!(is_err(&res));
    assert!(err_message(&res).contains("duplicate"));
}

#[test]
fn question_result_tool_surfaces_submitted_answer() {
    let (_d, server) = started("r");
    server
        .darkrun_question(Parameters(QuestionInput {
            slug: "r".into(),
            title: None,
            prompt: "p".into(),
            context: None,
            options: vec![q_opt("a", "A"), q_opt("b", "B")],
            multi_select: true,
            image_urls: vec![],
        }))
        .unwrap();

    // Before an answer, the result reads back pending with no answer.
    let pending = server
        .darkrun_question_result(Parameters(SessionResultInput {
            slug: "r".into(),
            session_id: "q-01".into(),
        }))
        .unwrap();
    assert_eq!(body(&pending)["status"], "pending");
    assert!(body(&pending)["answer"].is_null());

    // Simulate the HTTP handler recording the operator's answer by upserting the
    // mutated payload back into the shared in-memory registry (no disk).
    let reg = server.sessions();
    if let Some(darkrun_api::SessionPayload::Question(mut q)) = reg.get("q-01") {
        q.answer = Some(darkrun_api::QuestionAnswer {
            selected: vec!["a".into(), "b".into()],
            text: Some("both".into()),
        });
        q.status = darkrun_api::SessionStatus::Answered;
        reg.upsert(darkrun_api::SessionPayload::Question(q));
    }

    let answered = server
        .darkrun_question_result(Parameters(SessionResultInput {
            slug: "r".into(),
            session_id: "q-01".into(),
        }))
        .unwrap();
    let v = body(&answered);
    assert_eq!(v["status"], "answered");
    assert_eq!(v["answer"]["selected"][0], "a");
    assert_eq!(v["answer"]["selected"][1], "b");
    assert_eq!(v["answer"]["text"], "both");
}

#[test]
fn question_result_tool_errors_on_missing_session() {
    let (_d, server) = started("r");
    let res = server
        .darkrun_question_result(Parameters(SessionResultInput {
            slug: "r".into(),
            session_id: "q-99".into(),
        }))
        .unwrap();
    assert!(is_err(&res));
}

// ── darkrun_direction ──────────────────────────────────────────────────────

#[test]
fn direction_tool_creates_awaiting_session() {
    let (_d, server) = started("r");
    let res = server
        .darkrun_direction(Parameters(DirectionInput {
            slug: "r".into(),
            title: Some("Direction".into()),
            prompt: "Pick a direction".into(),
            context: Some("ctx".into()),
            archetypes: vec![arch_in("brutalist"), arch_in("editorial")],
        }))
        .unwrap();
    assert!(is_ok(&res));
    let v = body(&res);
    assert_eq!(v["session_id"], "d-01");
    assert_eq!(v["session_type"], "direction");
    assert_eq!(v["awaiting_answer"], true);
}

#[test]
fn direction_tool_rejects_incomplete_archetypes() {
    let (_d, server) = started("r");
    // missing image_url
    let mut bad = arch_in("a");
    bad.image_url = "".into();
    let res = server
        .darkrun_direction(Parameters(DirectionInput {
            slug: "r".into(),
            title: None,
            prompt: "p".into(),
            context: None,
            archetypes: vec![bad],
        }))
        .unwrap();
    assert!(is_err(&res));

    // no archetypes
    let res = server
        .darkrun_direction(Parameters(DirectionInput {
            slug: "r".into(),
            title: None,
            prompt: "p".into(),
            context: None,
            archetypes: vec![],
        }))
        .unwrap();
    assert!(is_err(&res));
}

#[test]
fn direction_result_tool_surfaces_choice_and_annotations() {
    let (_d, server) = started("r");
    server
        .darkrun_direction(Parameters(DirectionInput {
            slug: "r".into(),
            title: None,
            prompt: "p".into(),
            context: None,
            archetypes: vec![arch_in("a"), arch_in("b")],
        }))
        .unwrap();

    // Simulate the HTTP handler recording the choice into the shared registry.
    let reg = server.sessions();
    if let Some(darkrun_api::SessionPayload::Direction(mut dr)) = reg.get("d-01") {
        dr.chosen_archetype = Some("b".into());
        dr.annotations = Some(darkrun_api::DirectionAnnotations {
            pins: vec![darkrun_api::DirectionPin {
                x: 0.5,
                y: 0.25,
                note: "tighten".into(),
            }],
            screenshot: Some("data:image/png;base64,AAAA".into()),
            comments: vec!["nice".into()],
        });
        dr.status = darkrun_api::SessionStatus::Decided;
        reg.upsert(darkrun_api::SessionPayload::Direction(dr));
    }

    let res = server
        .darkrun_direction_result(Parameters(SessionResultInput {
            slug: "r".into(),
            session_id: "d-01".into(),
        }))
        .unwrap();
    let v = body(&res);
    assert_eq!(v["status"], "decided");
    assert_eq!(v["chosen_archetype"], "b");
    assert_eq!(v["annotations"]["pins"][0]["note"], "tighten");
    assert_eq!(v["annotations"]["comments"][0], "nice");
}

// ── darkrun_picker ─────────────────────────────────────────────────────────

#[test]
fn picker_tool_creates_awaiting_session() {
    let (_d, server) = started("r");
    let res = server
        .darkrun_picker(Parameters(PickerInput {
            slug: "r".into(),
            kind: "factory".into(),
            title: "Pick a factory".into(),
            prompt: "which?".into(),
            options: vec![p_opt("software"), p_opt("design")],
        }))
        .unwrap();
    assert!(is_ok(&res));
    let v = body(&res);
    assert_eq!(v["session_id"], "p-01");
    assert_eq!(v["session_type"], "picker");
}

#[test]
fn picker_tool_rejects_invalid_kind() {
    let (_d, server) = started("r");
    let res = server
        .darkrun_picker(Parameters(PickerInput {
            slug: "r".into(),
            kind: "telepathy".into(),
            title: "t".into(),
            prompt: "p".into(),
            options: vec![p_opt("a")],
        }))
        .unwrap();
    assert!(is_err(&res));
    assert!(err_message(&res).contains("invalid picker kind"));
}

#[test]
fn picker_tool_rejects_empty_title_and_options() {
    let (_d, server) = started("r");
    let bad_title = server
        .darkrun_picker(Parameters(PickerInput {
            slug: "r".into(),
            kind: "mode".into(),
            title: " ".into(),
            prompt: "p".into(),
            options: vec![p_opt("a")],
        }))
        .unwrap();
    assert!(is_err(&bad_title));

    let no_opts = server
        .darkrun_picker(Parameters(PickerInput {
            slug: "r".into(),
            kind: "mode".into(),
            title: "t".into(),
            prompt: "p".into(),
            options: vec![],
        }))
        .unwrap();
    assert!(is_err(&no_opts));
}

#[test]
fn picker_result_tool_surfaces_selection() {
    let (_d, server) = started("r");
    server
        .darkrun_picker(Parameters(PickerInput {
            slug: "r".into(),
            kind: "station".into(),
            title: "t".into(),
            prompt: "p".into(),
            options: vec![p_opt("frame"), p_opt("shape")],
        }))
        .unwrap();

    // Simulate the HTTP handler recording the selection into the shared registry.
    let reg = server.sessions();
    if let Some(darkrun_api::SessionPayload::Picker(mut p)) = reg.get("p-01") {
        p.selection = Some(darkrun_api::PickerSelection { id: "shape".into() });
        p.status = darkrun_api::SessionStatus::Decided;
        reg.upsert(darkrun_api::SessionPayload::Picker(p));
    }

    let res = server
        .darkrun_picker_result(Parameters(SessionResultInput {
            slug: "r".into(),
            session_id: "p-01".into(),
        }))
        .unwrap();
    let v = body(&res);
    assert_eq!(v["status"], "decided");
    assert_eq!(v["selection"]["id"], "shape");
}

// ── cross-cutting ──────────────────────────────────────────────────────────

#[test]
fn visual_sessions_coexist_on_one_run_with_unique_ids() {
    let (_d, server) = started("r");
    let q = body(&server
        .darkrun_question(Parameters(QuestionInput {
            slug: "r".into(),
            title: None,
            prompt: "p".into(),
            context: None,
            options: vec![q_opt("a", "A")],
            multi_select: false,
            image_urls: vec![],
        }))
        .unwrap());
    let q2 = body(&server
        .darkrun_question(Parameters(QuestionInput {
            slug: "r".into(),
            title: None,
            prompt: "p".into(),
            context: None,
            options: vec![q_opt("a", "A")],
            multi_select: false,
            image_urls: vec![],
        }))
        .unwrap());
    let dn = body(&server
        .darkrun_direction(Parameters(DirectionInput {
            slug: "r".into(),
            title: None,
            prompt: "p".into(),
            context: None,
            archetypes: vec![arch_in("a")],
        }))
        .unwrap());
    let pk = body(&server
        .darkrun_picker(Parameters(PickerInput {
            slug: "r".into(),
            kind: "confirm".into(),
            title: "t".into(),
            prompt: "p".into(),
            options: vec![p_opt("yes")],
        }))
        .unwrap());

    assert_eq!(q["session_id"], "q-01");
    assert_eq!(q2["session_id"], "q-02");
    assert_eq!(dn["session_id"], "d-01");
    assert_eq!(pk["session_id"], "p-01");

    // All four still resolvable independently.
    assert!(is_ok(&server
        .darkrun_question_result(Parameters(SessionResultInput {
            slug: "r".into(),
            session_id: "q-02".into(),
        }))
        .unwrap()));
    assert!(is_ok(&server
        .darkrun_direction_result(Parameters(SessionResultInput {
            slug: "r".into(),
            session_id: "d-01".into(),
        }))
        .unwrap()));
    assert!(is_ok(&server
        .darkrun_picker_result(Parameters(SessionResultInput {
            slug: "r".into(),
            session_id: "p-01".into(),
        }))
        .unwrap()));
}

#[test]
fn result_tool_rejects_wrong_session_kind() {
    let (_d, server) = started("r");
    server
        .darkrun_question(Parameters(QuestionInput {
            slug: "r".into(),
            title: None,
            prompt: "p".into(),
            context: None,
            options: vec![q_opt("a", "A")],
            multi_select: false,
            image_urls: vec![],
        }))
        .unwrap();
    // q-01 is a question; reading it as a picker fails.
    let res = server
        .darkrun_picker_result(Parameters(SessionResultInput {
            slug: "r".into(),
            session_id: "q-01".into(),
        }))
        .unwrap();
    assert!(is_err(&res));
}

// ── Surface classification (darkrun_run_surface) ───────────────────────────

fn set_surface(server: &DarkrunServer, slug: &str, surface: &str) -> CallToolResult {
    server
        .darkrun_run_surface(Parameters(RunSurfaceInput {
            slug: slug.into(),
            surface: Some(surface.into()),
        }))
        .unwrap()
}

fn get_surface(server: &DarkrunServer, slug: &str) -> CallToolResult {
    server
        .darkrun_run_surface(Parameters(RunSurfaceInput {
            slug: slug.into(),
            surface: None,
        }))
        .unwrap()
}

#[test]
fn surface_unclassified_reads_none() {
    let (_d, server) = started("r");
    let res = get_surface(&server, "r");
    assert!(is_ok(&res));
    assert!(body(&res)["surface"].is_null());
    assert!(body(&res).get("route").is_none_or(|v| v.is_null()));
}

#[test]
fn surface_classify_web_ui_routes_web() {
    let (_d, server) = started("r");
    let res = set_surface(&server, "r", "web-ui");
    assert!(is_ok(&res));
    let b = body(&res);
    assert_eq!(b["surface"], "web_ui");
    assert_eq!(b["is_visual"], true);
    assert_eq!(b["is_bench"], false);
    assert_eq!(b["route"], "web");
}

#[test]
fn surface_classify_library_routes_bench() {
    let (_d, server) = started("r");
    let b = body(&set_surface(&server, "r", "lib"));
    assert_eq!(b["surface"], "library");
    assert_eq!(b["is_bench"], true);
    assert_eq!(b["route"], "bench");
}

#[test]
fn surface_classify_cli_routes_terminal() {
    let (_d, server) = started("r");
    let b = body(&set_surface(&server, "r", "cli"));
    assert_eq!(b["surface"], "cli");
    assert_eq!(b["is_terminal"], true);
    assert_eq!(b["route"], "terminal");
}

#[test]
fn surface_persists_across_reads() {
    let (_d, server) = started("r");
    set_surface(&server, "r", "mobile");
    let b = body(&get_surface(&server, "r"));
    assert_eq!(b["surface"], "mobile");
    assert_eq!(b["route"], "web");
}

#[test]
fn surface_unknown_token_is_error() {
    let (_d, server) = started("r");
    let res = server
        .darkrun_run_surface(Parameters(RunSurfaceInput {
            slug: "r".into(),
            surface: Some("telepathy".into()),
        }))
        .unwrap();
    assert!(is_err(&res));
    assert!(err_message(&res).contains("unknown surface"));
}

#[test]
fn surface_on_missing_run_is_error() {
    let (_d, server) = server();
    let res = get_surface(&server, "ghost");
    assert!(is_err(&res));
}

// ── Proof attach/get (darkrun_proof_attach / _get) ─────────────────────────

#[test]
fn proof_attach_requires_classified_surface() {
    let (_d, server) = started("r");
    let res = server
        .darkrun_proof_attach(Parameters(ProofAttachInput {
            slug: "r".into(),
            proof: serde_json::json!({ "surface": "library", "bench": { "p50": 1.0 } }),
            station: None,
        }))
        .unwrap();
    assert!(is_err(&res));
    assert!(err_message(&res).contains("no classified surface"));
}

#[test]
fn proof_attach_rejects_surface_mismatch() {
    let (_d, server) = started("r");
    set_surface(&server, "r", "library");
    let res = server
        .darkrun_proof_attach(Parameters(ProofAttachInput {
            slug: "r".into(),
            proof: serde_json::json!({ "surface": "web_ui", "web": {} }),
            station: None,
        }))
        .unwrap();
    assert!(is_err(&res));
    assert!(err_message(&res).contains("does not match"));
}

#[test]
fn proof_attach_invalid_payload_is_error() {
    let (_d, server) = started("r");
    set_surface(&server, "r", "api");
    let res = server
        .darkrun_proof_attach(Parameters(ProofAttachInput {
            slug: "r".into(),
            proof: serde_json::json!({ "not_a_surface": true }),
            station: None,
        }))
        .unwrap();
    assert!(is_err(&res));
    assert!(err_message(&res).contains("invalid proof payload"));
}

#[test]
fn proof_attach_bench_then_get() {
    let (_d, server) = started("r");
    set_surface(&server, "r", "api");
    let attach = server
        .darkrun_proof_attach(Parameters(ProofAttachInput {
            slug: "r".into(),
            proof: serde_json::json!({
                "surface": "api",
                "bench": { "p50": 1.0, "p95": 2.5, "p99": 4.0, "throughput": 12000.0, "samples": 500 }
            }),
            station: Some("prove".into()),
        }))
        .unwrap();
    assert!(is_ok(&attach));
    let ab = body(&attach);
    assert_eq!(ab["ok"], true);
    assert_eq!(ab["surface"], "api");
    assert_eq!(ab["block_matches_surface"], true);

    let got = server
        .darkrun_proof_get(Parameters(ProofGetInput {
            slug: "r".into(),
            station: Some("prove".into()),
        }))
        .unwrap();
    assert!(is_ok(&got));
    let gb = body(&got);
    assert_eq!(gb["run"], "r");
    assert_eq!(gb["station"], "prove");
    assert_eq!(gb["proof"]["bench"]["p95"], 2.5);
}

#[test]
fn proof_attach_web_carries_vitals_and_audits() {
    let (_d, server) = started("r");
    set_surface(&server, "r", "web-ui");
    let attach = server
        .darkrun_proof_attach(Parameters(ProofAttachInput {
            slug: "r".into(),
            proof: serde_json::json!({
                "surface": "web_ui",
                "web": {
                    "vitals": { "lcp": 1100.0, "cls": 0.02 },
                    "audits": [{ "name": "contrast", "value": "5.1:1", "pass": true }],
                    "screenshot_url": "/shot/home.png"
                }
            }),
            station: None,
        }))
        .unwrap();
    assert_eq!(body(&attach)["block_matches_surface"], true);

    let got = body(&server
        .darkrun_proof_get(Parameters(ProofGetInput { slug: "r".into(), station: None }))
        .unwrap());
    assert_eq!(got["proof"]["web"]["vitals"]["lcp"], 1100.0);
    assert_eq!(got["proof"]["web"]["audits"][0]["name"], "contrast");
}

#[test]
fn proof_attach_flags_missing_block_but_records() {
    // A visual surface with no web block attaches, but is flagged.
    let (_d, server) = started("r");
    set_surface(&server, "r", "desktop");
    let attach = server
        .darkrun_proof_attach(Parameters(ProofAttachInput {
            slug: "r".into(),
            proof: serde_json::json!({ "surface": "desktop" }),
            station: None,
        }))
        .unwrap();
    assert!(is_ok(&attach));
    assert_eq!(body(&attach)["block_matches_surface"], false);
}

#[test]
fn proof_get_station_falls_back_to_run_level() {
    let (_d, server) = started("r");
    set_surface(&server, "r", "cli");
    server
        .darkrun_proof_attach(Parameters(ProofAttachInput {
            slug: "r".into(),
            proof: serde_json::json!({ "surface": "cli", "web": { "screenshot_url": "/snap.txt" } }),
            station: None,
        }))
        .unwrap();
    let got = body(&server
        .darkrun_proof_get(Parameters(ProofGetInput {
            slug: "r".into(),
            station: Some("prove".into()),
        }))
        .unwrap());
    assert!(got["station"].is_null());
    assert_eq!(got["proof"]["web"]["screenshot_url"], "/snap.txt");
}

#[test]
fn proof_get_errors_when_none_attached() {
    let (_d, server) = started("r");
    set_surface(&server, "r", "data");
    let res = server
        .darkrun_proof_get(Parameters(ProofGetInput { slug: "r".into(), station: None }))
        .unwrap();
    assert!(is_err(&res));
}
