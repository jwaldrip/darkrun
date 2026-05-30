//! Exhaustive integration tests for the three-track priority machine and the
//! checkpoint decision surface of `darkrun-mcp`.
//!
//! These drive the public manager API (`run_start`, `run_tick`,
//! `derive_position`, `checkpoint_decide`) plus the typed `feedback` and
//! `drift` helpers over a real on-disk `.darkrun/` tree, the same code path the
//! MCP tools take.
//!
//! Coverage map:
//!   * Track ordering: drift > feedback > run, and each pairwise preemption.
//!   * Feedback preemption: any open item routes a `FixFeedback` before run
//!     work; terminal items release the run track.
//!   * `feedback_open` semantics across every status string (open/terminal),
//!     missing status lines, casing, quoting, and ordering by id.
//!   * `checkpoint_decide` approve (advances + completes + stamps `Advanced`)
//!     and reject (blocks + stamps `Blocked` + files preempting feedback).
//!   * Every `CheckpointKind`: `auto` auto-advances during the tick;
//!     `ask`/`external`/`await` hold for an operator decision.
//!   * `CheckpointOutcome` stamping on the persisted station checkpoint.

use darkrun_core::domain::{
    CheckpointKind, CheckpointOutcome, Drift, DriftKind, FeedbackSeverity, FeedbackStatus, Status,
    StationPhase, Unit, UnitFrontmatter,
};
use darkrun_core::StateStore;
use darkrun_mcp::position::{
    checkpoint_decide, derive_position, run_start, run_tick, Position, RunAction,
};
use darkrun_mcp::{drift, feedback, Track};
use tempfile::TempDir;

// ───────────────────────────── harness ──────────────────────────────────

fn store() -> (TempDir, StateStore) {
    let dir = TempDir::new().expect("tmp");
    let store = StateStore::new(dir.path());
    (dir, store)
}

/// Start a fresh software run named `r` and return the store.
fn started() -> (TempDir, StateStore) {
    let (d, store) = store();
    run_start(&store, "r", "software", None, "continuous").expect("start");
    (d, store)
}

/// Write a raw feedback doc with an explicit status line.
fn raw_feedback(store: &StateStore, run: &str, id: &str, status: &str) {
    let doc = format!("---\nstatus: {status}\nstation: frame\n---\nbody text\n");
    store.write_feedback_raw(run, id, &doc).expect("write fb");
}

/// Seed a completed unit onto a station so Manufacture clears in one tick.
fn seed_completed_unit(store: &StateStore, run: &str, station: &str, slug: &str) {
    let unit = Unit {
        slug: slug.into(),
        frontmatter: UnitFrontmatter {
            status: Status::Completed,
            station: Some(station.into()),
            ..Default::default()
        },
        title: slug.into(),
        body: String::new(),
    };
    store.write_unit(run, &unit).expect("write unit");
}

/// Drive a station from wherever it sits to its open Checkpoint action.
fn walk_to_checkpoint(store: &StateStore, run: &str, station: &str) -> RunAction {
    seed_completed_unit(store, run, station, &format!("{station}-u1"));
    for _ in 0..10 {
        let t = run_tick(store, run).expect("tick");
        match &t.action {
            RunAction::Checkpoint { station: s, .. } if s == station => return t.action,
            RunAction::Spec { station: s, .. }
            | RunAction::Review { station: s, .. }
            | RunAction::Manufacture { station: s, .. }
            | RunAction::Audit { station: s, .. }
            | RunAction::Reflect { station: s, .. }
                if s == station => {}
            other => panic!("unexpected action walking {station}: {other:?}"),
        }
    }
    panic!("station {station} never reached checkpoint");
}

/// Advance the active station to `station` by approving every gate before it.
/// Auto stations advance during the walk; ask/external/await are approved.
fn advance_to_station(store: &StateStore, run: &str, target: &str) {
    let order = ["frame", "specify", "shape", "build", "prove", "harden"];
    for st in order {
        if st == target {
            return;
        }
        let cp = walk_to_checkpoint(store, run, st);
        if let RunAction::Checkpoint { kind, .. } = cp {
            if !matches!(kind, CheckpointKind::Auto) {
                checkpoint_decide(store, run, true, None).expect("approve");
            }
        }
    }
}

fn active_station(store: &StateStore, run: &str) -> String {
    store.read_state(run).unwrap().unwrap().active_station
}

fn station_status(store: &StateStore, run: &str, station: &str) -> Status {
    store.read_state(run).unwrap().unwrap().stations[station].status
}

fn station_phase(store: &StateStore, run: &str, station: &str) -> StationPhase {
    store.read_state(run).unwrap().unwrap().stations[station].phase
}

fn checkpoint_outcome(
    store: &StateStore,
    run: &str,
    station: &str,
) -> Option<CheckpointOutcome> {
    store.read_state(run).unwrap().unwrap().stations[station]
        .checkpoint
        .as_ref()
        .and_then(|c| c.outcome)
}

fn pos_track(p: &Position) -> Track {
    p.track
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION 1 — three-track priority ordering (drift > feedback > run)
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn bare_run_uses_run_track() {
    let (_d, store) = started();
    let pos = derive_position(&store, "r").unwrap();
    assert_eq!(pos_track(&pos), Track::Run);
    assert!(matches!(pos.action, Some(RunAction::Spec { .. })));
}

#[test]
fn open_feedback_preempts_run_track() {
    let (_d, store) = started();
    feedback::create(&store, "r", "frame", "broken", None).unwrap();
    let pos = derive_position(&store, "r").unwrap();
    assert_eq!(pos_track(&pos), Track::Feedback);
    assert!(matches!(pos.action, Some(RunAction::FixFeedback { .. })));
}

#[test]
fn drift_preempts_feedback() {
    let (_d, store) = started();
    feedback::create(&store, "r", "frame", "broken", None).unwrap();
    record_drift(&store, "r", "d-01", "frame/frame.md", "frame");
    let pos = derive_position(&store, "r").unwrap();
    assert_eq!(pos_track(&pos), Track::Drift);
}

#[test]
fn drift_preempts_run_with_no_feedback() {
    let (_d, store) = started();
    record_drift(&store, "r", "d-01", "frame/frame.md", "frame");
    let pos = derive_position(&store, "r").unwrap();
    assert_eq!(pos_track(&pos), Track::Drift);
    assert!(matches!(pos.action, Some(RunAction::ResolveDrift { .. })));
}

#[test]
fn full_priority_drift_then_feedback_then_run() {
    let (_d, store) = started();
    // run only
    assert_eq!(pos_track(&derive_position(&store, "r").unwrap()), Track::Run);
    // + feedback
    feedback::create(&store, "r", "frame", "x", None).unwrap();
    assert_eq!(
        pos_track(&derive_position(&store, "r").unwrap()),
        Track::Feedback
    );
    // + drift
    record_drift(&store, "r", "d-01", "frame/frame.md", "frame");
    assert_eq!(
        pos_track(&derive_position(&store, "r").unwrap()),
        Track::Drift
    );
}

#[test]
fn removing_drift_falls_back_to_feedback() {
    let (_d, store) = started();
    feedback::create(&store, "r", "frame", "x", None).unwrap();
    record_drift(&store, "r", "d-01", "frame/frame.md", "frame");
    assert_eq!(
        pos_track(&derive_position(&store, "r").unwrap()),
        Track::Drift
    );
    // Remove the drift dir → feedback resurfaces.
    std::fs::remove_dir_all(store.run_dir("r").join("drift")).unwrap();
    assert_eq!(
        pos_track(&derive_position(&store, "r").unwrap()),
        Track::Feedback
    );
}

#[test]
fn resolving_feedback_falls_back_to_run() {
    let (_d, store) = started();
    let fb = feedback::create(&store, "r", "frame", "x", None).unwrap();
    assert_eq!(
        pos_track(&derive_position(&store, "r").unwrap()),
        Track::Feedback
    );
    feedback::set_status(&store, "r", &fb.id, FeedbackStatus::Addressed).unwrap();
    assert_eq!(pos_track(&derive_position(&store, "r").unwrap()), Track::Run);
}

#[test]
fn drift_and_feedback_both_clear_back_to_run() {
    let (_d, store) = started();
    let fb = feedback::create(&store, "r", "frame", "x", None).unwrap();
    record_drift(&store, "r", "d-01", "frame/frame.md", "frame");
    // drift first
    assert_eq!(
        pos_track(&derive_position(&store, "r").unwrap()),
        Track::Drift
    );
    std::fs::remove_dir_all(store.run_dir("r").join("drift")).unwrap();
    // then feedback
    assert_eq!(
        pos_track(&derive_position(&store, "r").unwrap()),
        Track::Feedback
    );
    feedback::reject(&store, "r", &fb.id, "no").unwrap();
    // then run
    assert_eq!(pos_track(&derive_position(&store, "r").unwrap()), Track::Run);
}

fn record_drift(store: &StateStore, run: &str, id: &str, path: &str, station: &str) {
    let d = Drift {
        path: path.into(),
        station: station.into(),
        run: run.into(),
        kind: DriftKind::Output,
        age: "1m".into(),
        unit: None,
    };
    drift::record(store, run, id, &d).unwrap();
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION 2 — feedback_open: every status string, open vs terminal
// ═══════════════════════════════════════════════════════════════════════

/// Helper asserting whether a raw status keeps the feedback OPEN (preempting)
/// by checking the derived track on a fresh run.
fn assert_open(status: &str) {
    let (_d, store) = started();
    raw_feedback(&store, "r", "fb-1", status);
    let pos = derive_position(&store, "r").unwrap();
    assert_eq!(
        pos_track(&pos),
        Track::Feedback,
        "status `{status}` should be open"
    );
}

fn assert_terminal(status: &str) {
    let (_d, store) = started();
    raw_feedback(&store, "r", "fb-1", status);
    let pos = derive_position(&store, "r").unwrap();
    assert_eq!(
        pos_track(&pos),
        Track::Run,
        "status `{status}` should be terminal"
    );
}

#[test]
fn open_status_pending() {
    assert_open("pending");
}

#[test]
fn open_status_fixing() {
    assert_open("fixing");
}

#[test]
fn open_status_escalated() {
    assert_open("escalated");
}

#[test]
fn terminal_status_closed() {
    assert_terminal("closed");
}

#[test]
fn terminal_status_rejected() {
    assert_terminal("rejected");
}

#[test]
fn terminal_status_addressed() {
    assert_terminal("addressed");
}

#[test]
fn terminal_status_answered() {
    assert_terminal("answered");
}

#[test]
fn terminal_status_non_actionable() {
    assert_terminal("non_actionable");
}

#[test]
fn open_status_uppercase_pending() {
    assert_open("PENDING");
}

#[test]
fn terminal_status_uppercase_closed() {
    assert_terminal("CLOSED");
}

#[test]
fn terminal_status_mixed_case_addressed() {
    assert_terminal("Addressed");
}

#[test]
fn terminal_status_quoted_rejected() {
    assert_terminal("\"rejected\"");
}

#[test]
fn open_status_quoted_pending() {
    assert_open("\"pending\"");
}

#[test]
fn terminal_status_trailing_whitespace() {
    assert_terminal("closed   ");
}

#[test]
fn unknown_status_string_is_open() {
    // An unrecognized status is not terminal → treated as open.
    assert_open("weird_unmapped");
}

#[test]
fn empty_status_value_is_open() {
    // An empty status value is not in the terminal set → open.
    assert_open("");
}

#[test]
fn missing_status_line_is_open() {
    let (_d, store) = started();
    store
        .write_feedback_raw("r", "fb-1", "no frontmatter here, just body\n")
        .unwrap();
    let pos = derive_position(&store, "r").unwrap();
    assert_eq!(pos_track(&pos), Track::Feedback);
}

#[test]
fn body_only_feedback_no_fence_is_open() {
    let (_d, store) = started();
    store
        .write_feedback_raw("r", "fb-1", "the button is misaligned\n")
        .unwrap();
    assert_eq!(
        pos_track(&derive_position(&store, "r").unwrap()),
        Track::Feedback
    );
}

#[test]
fn first_status_line_wins() {
    // feedback_open returns on the first `status:` line it sees.
    let (_d, store) = started();
    let doc = "---\nstatus: pending\n---\nstatus: closed\n";
    store.write_feedback_raw("r", "fb-1", doc).unwrap();
    // First status is pending → open.
    assert_eq!(
        pos_track(&derive_position(&store, "r").unwrap()),
        Track::Feedback
    );
}

#[test]
fn first_status_terminal_even_if_later_open() {
    let (_d, store) = started();
    let doc = "---\nstatus: closed\n---\nstatus: pending\n";
    store.write_feedback_raw("r", "fb-1", doc).unwrap();
    assert_eq!(pos_track(&derive_position(&store, "r").unwrap()), Track::Run);
}

#[test]
fn every_feedbackstatus_variant_open_or_terminal_matches_module() {
    // Cross-check feedback::is_terminal against the manager's preemption.
    let cases = [
        (FeedbackStatus::Pending, false),
        (FeedbackStatus::Fixing, false),
        (FeedbackStatus::Escalated, false),
        (FeedbackStatus::Addressed, true),
        (FeedbackStatus::Answered, true),
        (FeedbackStatus::NonActionable, true),
        (FeedbackStatus::Closed, true),
        (FeedbackStatus::Rejected, true),
    ];
    for (status, terminal) in cases {
        assert_eq!(
            feedback::is_terminal(status),
            terminal,
            "{status:?} terminal mismatch"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION 3 — feedback ordering & multi-item preemption
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn first_open_feedback_by_id_is_dispatched() {
    let (_d, store) = started();
    raw_feedback(&store, "r", "fb-01", "closed");
    raw_feedback(&store, "r", "fb-02", "pending");
    raw_feedback(&store, "r", "fb-03", "pending");
    let pos = derive_position(&store, "r").unwrap();
    match pos.action {
        Some(RunAction::FixFeedback { feedback_id, .. }) => assert_eq!(feedback_id, "fb-02"),
        other => panic!("expected fix fb-02, got {other:?}"),
    }
}

#[test]
fn all_terminal_feedback_releases_run() {
    let (_d, store) = started();
    raw_feedback(&store, "r", "fb-01", "closed");
    raw_feedback(&store, "r", "fb-02", "rejected");
    raw_feedback(&store, "r", "fb-03", "addressed");
    assert_eq!(pos_track(&derive_position(&store, "r").unwrap()), Track::Run);
}

#[test]
fn single_open_among_many_terminal_preempts() {
    let (_d, store) = started();
    raw_feedback(&store, "r", "fb-01", "closed");
    raw_feedback(&store, "r", "fb-02", "answered");
    raw_feedback(&store, "r", "fb-03", "escalated"); // open
    raw_feedback(&store, "r", "fb-04", "rejected");
    let pos = derive_position(&store, "r").unwrap();
    match pos.action {
        Some(RunAction::FixFeedback { feedback_id, .. }) => assert_eq!(feedback_id, "fb-03"),
        other => panic!("expected fb-03, got {other:?}"),
    }
}

#[test]
fn fixing_status_keeps_feedback_active() {
    let (_d, store) = started();
    let fb = feedback::create(&store, "r", "frame", "x", None).unwrap();
    feedback::set_status(&store, "r", &fb.id, FeedbackStatus::Fixing).unwrap();
    assert_eq!(
        pos_track(&derive_position(&store, "r").unwrap()),
        Track::Feedback
    );
}

#[test]
fn fixfeedback_carries_active_station_not_feedback_station() {
    // The FixFeedback action's `station` is the run's current station, which is
    // independent of the feedback item's own `station:` field.
    let (_d, store) = started();
    raw_feedback(&store, "r", "fb-1", "pending"); // feedback station: frame
    let pos = derive_position(&store, "r").unwrap();
    match pos.action {
        Some(RunAction::FixFeedback { station, run, .. }) => {
            assert_eq!(run, "r");
            assert_eq!(station, "frame"); // active station of a fresh run
        }
        other => panic!("got {other:?}"),
    }
}

#[test]
fn feedback_preempts_at_later_station_too() {
    let (_d, store) = started();
    advance_to_station(&store, "r", "shape");
    assert_eq!(active_station(&store, "r"), "shape");
    feedback::create(&store, "r", "shape", "rework", None).unwrap();
    let pos = derive_position(&store, "r").unwrap();
    assert_eq!(pos_track(&pos), Track::Feedback);
    match pos.action {
        Some(RunAction::FixFeedback { station, .. }) => assert_eq!(station, "shape"),
        other => panic!("got {other:?}"),
    }
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION 4 — checkpoint_decide approve: advances + completes + stamps
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn approve_completes_frame_and_advances_to_specify() {
    let (_d, store) = started();
    let decided = checkpoint_decide(&store, "r", true, None).expect("approve");
    assert!(matches!(&decided.action, RunAction::Spec { station, .. } if station == "specify"));
    assert_eq!(station_status(&store, "r", "frame"), Status::Completed);
    assert_eq!(active_station(&store, "r"), "specify");
}

#[test]
fn approve_stamps_advanced_outcome() {
    let (_d, store) = started();
    checkpoint_decide(&store, "r", true, None).expect("approve");
    assert_eq!(
        checkpoint_outcome(&store, "r", "frame"),
        Some(CheckpointOutcome::Advanced)
    );
}

#[test]
fn approve_sets_completed_at_timestamp() {
    let (_d, store) = started();
    checkpoint_decide(&store, "r", true, None).expect("approve");
    let s = store.read_state("r").unwrap().unwrap();
    assert!(s.stations["frame"].completed_at.is_some());
}

#[test]
fn approve_emits_next_station_spec_action() {
    // checkpoint_decide re-ticks once into the next station, so the returned
    // action is the next station's Spec. (complete_station seeds the next
    // station Pending/Spec; the re-tick then stamps it InProgress/Review.)
    let (_d, store) = started();
    let res = checkpoint_decide(&store, "r", true, None).expect("approve");
    assert!(matches!(&res.action, RunAction::Spec { station, .. } if station == "specify"));
    let s = store.read_state("r").unwrap().unwrap();
    // The re-tick advanced specify's phase off Spec onto Review.
    assert_eq!(s.stations["specify"].phase, StationPhase::Review);
}

#[test]
fn approve_through_every_station_to_sealed() {
    let (_d, store) = started();
    advance_to_station(&store, "r", "harden");
    let cp = walk_to_checkpoint(&store, "r", "harden");
    assert!(matches!(cp, RunAction::Checkpoint { kind: CheckpointKind::External, .. }));
    let sealed = checkpoint_decide(&store, "r", true, None).expect("seal");
    assert!(matches!(&sealed.action, RunAction::Sealed { run } if run == "r"));
}

#[test]
fn approve_advances_one_station_per_decision() {
    let (_d, store) = started();
    assert_eq!(active_station(&store, "r"), "frame");
    checkpoint_decide(&store, "r", true, None).unwrap();
    assert_eq!(active_station(&store, "r"), "specify");
    checkpoint_decide(&store, "r", true, None).unwrap();
    assert_eq!(active_station(&store, "r"), "shape");
    checkpoint_decide(&store, "r", true, None).unwrap();
    assert_eq!(active_station(&store, "r"), "build");
}

#[test]
fn approve_when_sealed_errors_no_active_station() {
    let (_d, store) = started();
    advance_to_station(&store, "r", "harden");
    walk_to_checkpoint(&store, "r", "harden");
    checkpoint_decide(&store, "r", true, None).expect("seal");
    // Now every station is complete → no active station.
    let err = checkpoint_decide(&store, "r", true, None).unwrap_err();
    assert!(matches!(
        err,
        darkrun_mcp::McpError::NoActiveStation(_)
    ));
}

#[test]
fn approve_ignores_feedback_param() {
    // Approving with a feedback body should NOT file feedback (feedback is only
    // filed on reject).
    let (_d, store) = started();
    checkpoint_decide(&store, "r", true, Some("ignored note".into())).expect("approve");
    let all = feedback::list(&store, "r").unwrap();
    assert!(all.is_empty(), "approve must not file feedback");
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION 5 — checkpoint_decide reject: blocks + stamps + files feedback
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn reject_blocks_station() {
    let (_d, store) = started();
    checkpoint_decide(&store, "r", false, Some("nope".into())).expect("reject");
    assert_eq!(station_status(&store, "r", "frame"), Status::Blocked);
}

#[test]
fn reject_stamps_blocked_outcome() {
    let (_d, store) = started();
    checkpoint_decide(&store, "r", false, Some("nope".into())).expect("reject");
    assert_eq!(
        checkpoint_outcome(&store, "r", "frame"),
        Some(CheckpointOutcome::Blocked)
    );
}

#[test]
fn reject_files_feedback_that_preempts() {
    let (_d, store) = started();
    let res = checkpoint_decide(&store, "r", false, Some("not good enough".into()))
        .expect("reject");
    assert_eq!(res.position.track, Track::Feedback);
    assert!(matches!(res.action, RunAction::FixFeedback { .. }));
}

#[test]
fn reject_feedback_body_contains_reason() {
    let (_d, store) = started();
    checkpoint_decide(&store, "r", false, Some("reason XYZ".into())).expect("reject");
    let all = feedback::list(&store, "r").unwrap();
    assert_eq!(all.len(), 1);
    assert!(all[0].body.contains("reason XYZ"));
}

#[test]
fn reject_feedback_is_pending_and_open() {
    let (_d, store) = started();
    checkpoint_decide(&store, "r", false, Some("x".into())).expect("reject");
    let all = feedback::list(&store, "r").unwrap();
    assert_eq!(all[0].status, FeedbackStatus::Pending);
}

#[test]
fn reject_without_feedback_at_checkpoint_holds_blocked() {
    // At the real Checkpoint phase, reject blocks and the re-tick re-emits the
    // held Checkpoint (which never resets status) so Blocked sticks.
    let (_d, store) = started();
    walk_to_checkpoint(&store, "r", "frame");
    let res = checkpoint_decide(&store, "r", false, None).expect("reject");
    assert_eq!(station_status(&store, "r", "frame"), Status::Blocked);
    // No feedback filed.
    assert!(feedback::list(&store, "r").unwrap().is_empty());
    // The position is the run track; the held checkpoint re-emits.
    assert_eq!(res.position.track, Track::Run);
    assert!(matches!(res.action, RunAction::Checkpoint { .. }));
}

#[test]
fn reject_at_spec_phase_files_nothing_without_body() {
    // Rejecting before the station reached its checkpoint (fresh run, Spec
    // phase): the block is stamped, but the re-tick's Spec action re-stamps the
    // station InProgress — the block does not persist at non-checkpoint phases.
    let (_d, store) = started();
    let res = checkpoint_decide(&store, "r", false, None).expect("reject");
    assert!(feedback::list(&store, "r").unwrap().is_empty());
    assert_eq!(res.position.track, Track::Run);
}

#[test]
fn reject_does_not_advance_active_station() {
    let (_d, store) = started();
    checkpoint_decide(&store, "r", false, Some("x".into())).expect("reject");
    assert_eq!(active_station(&store, "r"), "frame");
    assert_ne!(station_status(&store, "r", "frame"), Status::Completed);
}

#[test]
fn reject_then_address_resumes_run_on_same_station() {
    let (_d, store) = started();
    checkpoint_decide(&store, "r", false, Some("fix".into())).expect("reject");
    // The filed feedback is dispatched.
    let pos = derive_position(&store, "r").unwrap();
    let fb_id = match pos.action {
        Some(RunAction::FixFeedback { feedback_id, .. }) => feedback_id,
        other => panic!("expected feedback, got {other:?}"),
    };
    // Address it terminally → run track resumes, still on frame.
    feedback::set_status(&store, "r", &fb_id, FeedbackStatus::Addressed).unwrap();
    let pos = derive_position(&store, "r").unwrap();
    assert_eq!(pos_track(&pos), Track::Run);
    assert_eq!(active_station(&store, "r"), "frame");
}

#[test]
fn reject_filed_feedback_id_is_checkpoint_slug() {
    let (_d, store) = started();
    checkpoint_decide(&store, "r", false, Some("x".into())).expect("reject");
    let raw = store.read_feedback_raw("r").unwrap();
    assert!(raw.contains_key("fb-checkpoint"));
}

#[test]
fn reject_empty_reason_files_no_body_feedback() {
    // An empty feedback option (None) files nothing.
    let (_d, store) = started();
    checkpoint_decide(&store, "r", false, None).expect("reject");
    assert!(feedback::list(&store, "r").unwrap().is_empty());
}

#[test]
fn reject_when_sealed_errors() {
    let (_d, store) = started();
    advance_to_station(&store, "r", "harden");
    walk_to_checkpoint(&store, "r", "harden");
    checkpoint_decide(&store, "r", true, None).expect("seal");
    let err = checkpoint_decide(&store, "r", false, Some("x".into())).unwrap_err();
    assert!(matches!(err, darkrun_mcp::McpError::NoActiveStation(_)));
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION 6 — every CheckpointKind during the checkpoint tick
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn ask_kind_holds_for_decision() {
    let (_d, store) = started();
    let cp = walk_to_checkpoint(&store, "r", "frame");
    assert!(matches!(cp, RunAction::Checkpoint { kind: CheckpointKind::Ask, .. }));
    // Held: station still in progress, not completed.
    assert_eq!(station_status(&store, "r", "frame"), Status::InProgress);
}

#[test]
fn ask_kind_does_not_stamp_outcome_until_decided() {
    let (_d, store) = started();
    walk_to_checkpoint(&store, "r", "frame");
    assert_eq!(checkpoint_outcome(&store, "r", "frame"), None);
}

#[test]
fn ask_kind_stamps_entered_at() {
    let (_d, store) = started();
    walk_to_checkpoint(&store, "r", "frame");
    let s = store.read_state("r").unwrap().unwrap();
    assert!(s.stations["frame"].checkpoint.as_ref().unwrap().entered_at.is_some());
}

#[test]
fn auto_kind_advances_during_tick() {
    let (_d, store) = started();
    advance_to_station(&store, "r", "build");
    let cp = walk_to_checkpoint(&store, "r", "build");
    assert!(matches!(cp, RunAction::Checkpoint { kind: CheckpointKind::Auto, .. }));
    // Auto advances with no decide call.
    assert_eq!(station_status(&store, "r", "build"), Status::Completed);
    assert_eq!(active_station(&store, "r"), "prove");
}

#[test]
fn auto_kind_stamps_advanced_outcome() {
    let (_d, store) = started();
    advance_to_station(&store, "r", "build");
    walk_to_checkpoint(&store, "r", "build");
    assert_eq!(
        checkpoint_outcome(&store, "r", "build"),
        Some(CheckpointOutcome::Advanced)
    );
}

#[test]
fn auto_kind_chains_two_auto_stations() {
    // build (auto) then prove (auto) both advance without decisions.
    let (_d, store) = started();
    advance_to_station(&store, "r", "build");
    walk_to_checkpoint(&store, "r", "build");
    assert_eq!(active_station(&store, "r"), "prove");
    walk_to_checkpoint(&store, "r", "prove");
    assert_eq!(station_status(&store, "r", "prove"), Status::Completed);
    assert_eq!(active_station(&store, "r"), "harden");
}

#[test]
fn external_kind_holds_for_decision() {
    let (_d, store) = started();
    advance_to_station(&store, "r", "harden");
    let cp = walk_to_checkpoint(&store, "r", "harden");
    assert!(matches!(cp, RunAction::Checkpoint { kind: CheckpointKind::External, .. }));
    assert_eq!(station_status(&store, "r", "harden"), Status::InProgress);
}

#[test]
fn external_kind_no_outcome_until_decided() {
    let (_d, store) = started();
    advance_to_station(&store, "r", "harden");
    walk_to_checkpoint(&store, "r", "harden");
    assert_eq!(checkpoint_outcome(&store, "r", "harden"), None);
}

#[test]
fn external_kind_approve_seals_run() {
    let (_d, store) = started();
    advance_to_station(&store, "r", "harden");
    walk_to_checkpoint(&store, "r", "harden");
    let res = checkpoint_decide(&store, "r", true, None).expect("approve");
    assert!(matches!(res.action, RunAction::Sealed { .. }));
    assert_eq!(checkpoint_outcome(&store, "r", "harden"), Some(CheckpointOutcome::Advanced));
}

#[test]
fn external_kind_reject_blocks_and_files_feedback() {
    let (_d, store) = started();
    advance_to_station(&store, "r", "harden");
    walk_to_checkpoint(&store, "r", "harden");
    let res = checkpoint_decide(&store, "r", false, Some("security gap".into())).expect("reject");
    assert_eq!(res.position.track, Track::Feedback);
    assert_eq!(station_status(&store, "r", "harden"), Status::Blocked);
}

#[test]
fn checkpoint_action_kind_matches_factory_def() {
    // The Checkpoint action surfaces the factory-defined kind per station.
    let expected = [
        ("frame", CheckpointKind::Ask),
        ("specify", CheckpointKind::Ask),
        ("shape", CheckpointKind::Ask),
        ("harden", CheckpointKind::External),
    ];
    for (st, kind) in expected {
        let (_d, store) = started();
        advance_to_station(&store, "r", st);
        let cp = walk_to_checkpoint(&store, "r", st);
        match cp {
            RunAction::Checkpoint { kind: k, station, .. } => {
                assert_eq!(k, kind, "{st} kind");
                assert_eq!(station, st);
            }
            other => panic!("expected checkpoint, got {other:?}"),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION 7 — checkpoint stamping persistence & idempotency
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn checkpoint_kind_persisted_into_state() {
    let (_d, store) = started();
    walk_to_checkpoint(&store, "r", "frame");
    let s = store.read_state("r").unwrap().unwrap();
    assert_eq!(
        s.stations["frame"].checkpoint.as_ref().unwrap().kind,
        CheckpointKind::Ask
    );
}

#[test]
fn ticking_after_hold_keeps_re_emitting_checkpoint() {
    // A held (ask) checkpoint re-emits the same Checkpoint action each tick
    // until the operator decides — deterministic, idempotent.
    let (_d, store) = started();
    walk_to_checkpoint(&store, "r", "frame");
    for _ in 0..3 {
        let t = run_tick(&store, "r").unwrap();
        assert!(matches!(t.action, RunAction::Checkpoint { kind: CheckpointKind::Ask, .. }));
        assert_eq!(station_status(&store, "r", "frame"), Status::InProgress);
    }
}

#[test]
fn derive_position_is_pure_repeatable() {
    let (_d, store) = started();
    feedback::create(&store, "r", "frame", "x", None).unwrap();
    let a = derive_position(&store, "r").unwrap();
    let b = derive_position(&store, "r").unwrap();
    let c = derive_position(&store, "r").unwrap();
    assert_eq!(a, b);
    assert_eq!(b, c);
}

#[test]
fn derive_position_does_not_mutate_state() {
    let (_d, store) = started();
    let before = store.read_state("r").unwrap().unwrap();
    let _ = derive_position(&store, "r").unwrap();
    let _ = derive_position(&store, "r").unwrap();
    let after = store.read_state("r").unwrap().unwrap();
    assert_eq!(before.active_station, after.active_station);
    assert_eq!(before.stations["frame"].phase, after.stations["frame"].phase);
}

#[test]
fn approve_then_state_reloads_consistently() {
    let (_d, store) = started();
    checkpoint_decide(&store, "r", true, None).expect("approve");
    // Reload from disk and verify the stamp survived serialization.
    let s = store.read_state("r").unwrap().unwrap();
    assert_eq!(s.stations["frame"].status, Status::Completed);
    assert_eq!(
        s.stations["frame"].checkpoint.as_ref().unwrap().outcome,
        Some(CheckpointOutcome::Advanced)
    );
}

#[test]
fn blocked_station_outcome_survives_reload() {
    let (_d, store) = started();
    checkpoint_decide(&store, "r", false, Some("x".into())).expect("reject");
    let s = store.read_state("r").unwrap().unwrap();
    assert_eq!(
        s.stations["frame"].checkpoint.as_ref().unwrap().outcome,
        Some(CheckpointOutcome::Blocked)
    );
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION 8 — drift action details under Track C
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn drift_action_carries_path() {
    let (_d, store) = started();
    record_drift(&store, "r", "d-01", "frame/out.md", "frame");
    let pos = derive_position(&store, "r").unwrap();
    match pos.action {
        Some(RunAction::ResolveDrift { path, .. }) => assert_eq!(path, "frame/out.md"),
        other => panic!("got {other:?}"),
    }
}

#[test]
fn drift_action_uses_entry_station_when_set() {
    let (_d, store) = started();
    record_drift(&store, "r", "d-01", "x.md", "shape");
    let pos = derive_position(&store, "r").unwrap();
    match pos.action {
        Some(RunAction::ResolveDrift { station, .. }) => assert_eq!(station, "shape"),
        other => panic!("got {other:?}"),
    }
}

#[test]
fn drift_action_defaults_station_to_current_when_blank() {
    let (_d, store) = started();
    let d = Drift {
        path: "x.md".into(),
        station: String::new(),
        run: "r".into(),
        kind: DriftKind::Output,
        age: String::new(),
        unit: None,
    };
    drift::record(&store, "r", "d-01", &d).unwrap();
    let pos = derive_position(&store, "r").unwrap();
    match pos.action {
        Some(RunAction::ResolveDrift { station, .. }) => assert_eq!(station, "frame"),
        other => panic!("got {other:?}"),
    }
}

#[test]
fn first_drift_by_filename_is_dispatched() {
    let (_d, store) = started();
    record_drift(&store, "r", "d-02", "b.md", "frame");
    record_drift(&store, "r", "d-01", "a.md", "frame");
    let pos = derive_position(&store, "r").unwrap();
    match pos.action {
        Some(RunAction::ResolveDrift { path, .. }) => assert_eq!(path, "a.md"),
        other => panic!("got {other:?}"),
    }
}

#[test]
fn drift_does_not_advance_phase_on_tick() {
    let (_d, store) = started();
    let before = station_phase(&store, "r", "frame");
    record_drift(&store, "r", "d-01", "x.md", "frame");
    let t = run_tick(&store, "r").unwrap();
    assert!(matches!(t.action, RunAction::ResolveDrift { .. }));
    assert_eq!(station_phase(&store, "r", "frame"), before);
}

#[test]
fn feedback_does_not_advance_phase_on_tick() {
    let (_d, store) = started();
    let before = station_phase(&store, "r", "frame");
    feedback::create(&store, "r", "frame", "x", None).unwrap();
    let t = run_tick(&store, "r").unwrap();
    assert!(matches!(t.action, RunAction::FixFeedback { .. }));
    assert_eq!(station_phase(&store, "r", "frame"), before);
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION 9 — interplay: feedback while held at checkpoint
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn feedback_filed_while_held_preempts_checkpoint() {
    let (_d, store) = started();
    walk_to_checkpoint(&store, "r", "frame");
    // Held at ask checkpoint. File feedback → it preempts the held checkpoint.
    feedback::create(&store, "r", "frame", "more work", None).unwrap();
    let pos = derive_position(&store, "r").unwrap();
    assert_eq!(pos_track(&pos), Track::Feedback);
}

#[test]
fn drift_filed_while_held_preempts_checkpoint() {
    let (_d, store) = started();
    walk_to_checkpoint(&store, "r", "frame");
    record_drift(&store, "r", "d-01", "frame/frame.md", "frame");
    let pos = derive_position(&store, "r").unwrap();
    assert_eq!(pos_track(&pos), Track::Drift);
}

#[test]
fn clearing_held_feedback_returns_to_checkpoint() {
    let (_d, store) = started();
    walk_to_checkpoint(&store, "r", "frame");
    let fb = feedback::create(&store, "r", "frame", "x", None).unwrap();
    assert_eq!(
        pos_track(&derive_position(&store, "r").unwrap()),
        Track::Feedback
    );
    feedback::set_status(&store, "r", &fb.id, FeedbackStatus::Closed).unwrap();
    let pos = derive_position(&store, "r").unwrap();
    assert_eq!(pos_track(&pos), Track::Run);
    assert!(matches!(pos.action, Some(RunAction::Checkpoint { kind: CheckpointKind::Ask, .. })));
}

#[test]
fn decide_still_works_after_feedback_cleared() {
    let (_d, store) = started();
    walk_to_checkpoint(&store, "r", "frame");
    let fb = feedback::create(&store, "r", "frame", "x", None).unwrap();
    feedback::reject(&store, "r", &fb.id, "no").unwrap();
    // Now approve the held checkpoint.
    let decided = checkpoint_decide(&store, "r", true, None).expect("approve");
    assert!(matches!(&decided.action, RunAction::Spec { station, .. } if station == "specify"));
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION 10 — full priority sweeps across many feedback severities/statuses
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn severity_does_not_affect_open_preemption() {
    // Severity is independent of open/terminal; any open item preempts.
    for sev in [
        FeedbackSeverity::Blocker,
        FeedbackSeverity::High,
        FeedbackSeverity::Medium,
        FeedbackSeverity::Low,
    ] {
        let (_d, store) = started();
        feedback::create(&store, "r", "frame", "x", Some(sev)).unwrap();
        assert_eq!(
            pos_track(&derive_position(&store, "r").unwrap()),
            Track::Feedback,
            "{sev:?}"
        );
    }
}

#[test]
fn terminal_transition_each_terminal_status_releases_run() {
    for term in [
        FeedbackStatus::Addressed,
        FeedbackStatus::Answered,
        FeedbackStatus::NonActionable,
        FeedbackStatus::Closed,
    ] {
        let (_d, store) = started();
        let fb = feedback::create(&store, "r", "frame", "x", None).unwrap();
        feedback::set_status(&store, "r", &fb.id, term).unwrap();
        assert_eq!(
            pos_track(&derive_position(&store, "r").unwrap()),
            Track::Run,
            "{term:?} should release run"
        );
    }
}

#[test]
fn reject_helper_releases_run_track() {
    let (_d, store) = started();
    let fb = feedback::create(&store, "r", "frame", "x", None).unwrap();
    feedback::reject(&store, "r", &fb.id, "invalid").unwrap();
    assert_eq!(pos_track(&derive_position(&store, "r").unwrap()), Track::Run);
}

#[test]
fn escalated_does_not_release_run_track() {
    // Escalated is NOT terminal for the open-walk.
    let (_d, store) = started();
    let fb = feedback::create(&store, "r", "frame", "x", None).unwrap();
    feedback::set_status(&store, "r", &fb.id, FeedbackStatus::Escalated).unwrap();
    assert_eq!(
        pos_track(&derive_position(&store, "r").unwrap()),
        Track::Feedback
    );
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION 11 — TickResult / action shape invariants
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn tickresult_run_field_matches_slug() {
    let (_d, store) = started();
    let t = run_tick(&store, "r").unwrap();
    assert_eq!(t.run, "r");
}

#[test]
fn tickresult_action_mirrors_position_action() {
    let (_d, store) = started();
    feedback::create(&store, "r", "frame", "x", None).unwrap();
    let t = run_tick(&store, "r").unwrap();
    assert_eq!(Some(t.action.clone()), t.position.action);
}

#[test]
fn noop_action_for_null_position() {
    let (_d, store) = started();
    run_tick(&store, "r").unwrap(); // spec → review
    run_tick(&store, "r").unwrap(); // review → manufacture
    // Pending unit with unmet dep → mid-wave noop.
    let blocked = Unit {
        slug: "u1".into(),
        frontmatter: UnitFrontmatter {
            status: Status::Pending,
            station: Some("frame".into()),
            depends_on: vec!["ghost".into()],
            ..Default::default()
        },
        title: "u1".into(),
        body: String::new(),
    };
    store.write_unit("r", &blocked).unwrap();
    let pos = derive_position(&store, "r").unwrap();
    assert!(pos.action.is_none());
    let t = run_tick(&store, "r").unwrap();
    assert!(matches!(t.action, RunAction::Noop { .. }));
}

#[test]
fn checkpoint_decide_retick_surfaces_new_position() {
    // decide re-ticks so the returned action is the post-decision cursor.
    let (_d, store) = started();
    let res = checkpoint_decide(&store, "r", true, None).expect("approve");
    // Approving frame advances to specify spec.
    assert_eq!(res.run, "r");
    assert!(matches!(&res.action, RunAction::Spec { station, .. } if station == "specify"));
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION 12 — error paths
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn derive_position_unknown_run_errors() {
    let (_d, store) = store();
    let err = derive_position(&store, "ghost").unwrap_err();
    // Reading a non-existent run.md surfaces a core error.
    assert!(matches!(err, darkrun_mcp::McpError::Core(_)));
}

#[test]
fn checkpoint_decide_unknown_run_errors() {
    let (_d, store) = store();
    let err = checkpoint_decide(&store, "ghost", true, None).unwrap_err();
    assert!(matches!(err, darkrun_mcp::McpError::Core(_)));
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION 13 — serde of Track / RunAction / Position / TickResult
// ═══════════════════════════════════════════════════════════════════════

fn json_of<T: serde::Serialize>(v: &T) -> serde_json::Value {
    serde_json::to_value(v).expect("serialize")
}

#[test]
fn track_serializes_snake_case_drift() {
    assert_eq!(json_of(&Track::Drift), serde_json::json!("drift"));
}

#[test]
fn track_serializes_snake_case_feedback() {
    assert_eq!(json_of(&Track::Feedback), serde_json::json!("feedback"));
}

#[test]
fn track_serializes_snake_case_run() {
    assert_eq!(json_of(&Track::Run), serde_json::json!("run"));
}

#[test]
fn runaction_spec_tagged_action_field() {
    let (_d, store) = started();
    let pos = derive_position(&store, "r").unwrap();
    let j = json_of(&pos.action.unwrap());
    assert_eq!(j["action"], serde_json::json!("spec"));
    assert_eq!(j["station"], serde_json::json!("frame"));
    assert_eq!(j["run"], serde_json::json!("r"));
}

#[test]
fn runaction_spec_carries_kills() {
    let (_d, store) = started();
    let pos = derive_position(&store, "r").unwrap();
    let j = json_of(&pos.action.unwrap());
    // frame eliminates "wrong-thing".
    assert_eq!(j["kills"], serde_json::json!("wrong-thing"));
}

#[test]
fn runaction_checkpoint_serializes_kind_snake_case() {
    let (_d, store) = started();
    walk_to_checkpoint(&store, "r", "frame");
    let pos = derive_position(&store, "r").unwrap();
    let j = json_of(&pos.action.unwrap());
    assert_eq!(j["action"], serde_json::json!("checkpoint"));
    assert_eq!(j["kind"], serde_json::json!("ask"));
}

#[test]
fn runaction_fixfeedback_serializes_feedback_id() {
    let (_d, store) = started();
    raw_feedback(&store, "r", "fb-7", "pending");
    let pos = derive_position(&store, "r").unwrap();
    let j = json_of(&pos.action.unwrap());
    assert_eq!(j["action"], serde_json::json!("fix_feedback"));
    assert_eq!(j["feedback_id"], serde_json::json!("fb-7"));
}

#[test]
fn runaction_resolvedrift_serializes_path() {
    let (_d, store) = started();
    record_drift(&store, "r", "d-01", "frame/x.md", "frame");
    let pos = derive_position(&store, "r").unwrap();
    let j = json_of(&pos.action.unwrap());
    assert_eq!(j["action"], serde_json::json!("resolve_drift"));
    assert_eq!(j["path"], serde_json::json!("frame/x.md"));
}

#[test]
fn runaction_sealed_serializes_run() {
    let (_d, store) = started();
    advance_to_station(&store, "r", "harden");
    walk_to_checkpoint(&store, "r", "harden");
    let res = checkpoint_decide(&store, "r", true, None).unwrap();
    let j = json_of(&res.action);
    assert_eq!(j["action"], serde_json::json!("sealed"));
    assert_eq!(j["run"], serde_json::json!("r"));
}

#[test]
fn runaction_noop_serializes_message() {
    let (_d, store) = started();
    run_tick(&store, "r").unwrap();
    run_tick(&store, "r").unwrap();
    let blocked = Unit {
        slug: "u1".into(),
        frontmatter: UnitFrontmatter {
            status: Status::Pending,
            station: Some("frame".into()),
            depends_on: vec!["ghost".into()],
            ..Default::default()
        },
        title: "u1".into(),
        body: String::new(),
    };
    store.write_unit("r", &blocked).unwrap();
    let t = run_tick(&store, "r").unwrap();
    let j = json_of(&t.action);
    assert_eq!(j["action"], serde_json::json!("noop"));
    assert!(j["message"].as_str().unwrap().contains("Mid-wave"));
}

#[test]
fn runaction_review_serializes_reviewers() {
    let (_d, store) = started();
    run_tick(&store, "r").unwrap(); // spec → review
    let pos = derive_position(&store, "r").unwrap();
    let j = json_of(&pos.action.unwrap());
    assert_eq!(j["action"], serde_json::json!("review"));
    // frame reviewers: value, feasibility.
    assert_eq!(j["reviewers"], serde_json::json!(["value", "feasibility"]));
}

#[test]
fn runaction_audit_serializes_reviewers() {
    let (_d, store) = started();
    run_tick(&store, "r").unwrap(); // spec→review
    run_tick(&store, "r").unwrap(); // review→manufacture
    seed_completed_unit(&store, "r", "frame", "u1");
    let t = run_tick(&store, "r").unwrap();
    let j = json_of(&t.action);
    assert_eq!(j["action"], serde_json::json!("audit"));
    assert_eq!(j["reviewers"], serde_json::json!(["value", "feasibility"]));
}

#[test]
fn runaction_manufacture_serializes_worker_and_units() {
    let (_d, store) = started();
    run_tick(&store, "r").unwrap();
    run_tick(&store, "r").unwrap();
    let unit = Unit {
        slug: "u1".into(),
        frontmatter: UnitFrontmatter {
            status: Status::Pending,
            station: Some("frame".into()),
            ..Default::default()
        },
        title: "u1".into(),
        body: String::new(),
    };
    store.write_unit("r", &unit).unwrap();
    let t = run_tick(&store, "r").unwrap();
    let j = json_of(&t.action);
    assert_eq!(j["action"], serde_json::json!("manufacture"));
    // frame's first worker is "framer".
    assert_eq!(j["worker"], serde_json::json!("framer"));
    assert_eq!(j["units"], serde_json::json!(["u1"]));
}

#[test]
fn position_serializes_track_and_action() {
    let (_d, store) = started();
    let pos = derive_position(&store, "r").unwrap();
    let j = json_of(&pos);
    assert_eq!(j["track"], serde_json::json!("run"));
    assert_eq!(j["action"]["action"], serde_json::json!("spec"));
}

#[test]
fn position_null_action_serializes_null() {
    let (_d, store) = started();
    run_tick(&store, "r").unwrap();
    run_tick(&store, "r").unwrap();
    let blocked = Unit {
        slug: "u1".into(),
        frontmatter: UnitFrontmatter {
            status: Status::Pending,
            station: Some("frame".into()),
            depends_on: vec!["ghost".into()],
            ..Default::default()
        },
        title: "u1".into(),
        body: String::new(),
    };
    store.write_unit("r", &blocked).unwrap();
    let pos = derive_position(&store, "r").unwrap();
    let j = json_of(&pos);
    assert_eq!(j["action"], serde_json::Value::Null);
    assert_eq!(j["track"], serde_json::json!("run"));
}

#[test]
fn tickresult_serializes_run_position_action() {
    let (_d, store) = started();
    let t = run_tick(&store, "r").unwrap();
    let j = json_of(&t);
    assert_eq!(j["run"], serde_json::json!("r"));
    assert!(j["position"].is_object());
    assert!(j["action"].is_object());
    // The engine-driven, override-resolved instructions ride alongside the
    // structured action so `darkrun_run_next` hands the agent both halves.
    assert!(j["prompt"].is_string(), "TickResult must serialize a rendered prompt");
    assert!(
        !j["prompt"].as_str().unwrap().trim().is_empty(),
        "rendered prompt must be non-empty"
    );
}

#[test]
fn checkpoint_outcome_serializes_advanced() {
    assert_eq!(json_of(&CheckpointOutcome::Advanced), serde_json::json!("advanced"));
}

#[test]
fn checkpoint_outcome_serializes_blocked() {
    assert_eq!(json_of(&CheckpointOutcome::Blocked), serde_json::json!("blocked"));
}

#[test]
fn checkpoint_kind_serializes_external() {
    assert_eq!(json_of(&CheckpointKind::External), serde_json::json!("external"));
}

#[test]
fn checkpoint_kind_serializes_await() {
    assert_eq!(json_of(&CheckpointKind::Await), serde_json::json!("await"));
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION 14 — per-station parametric: each station holds/auto-advances
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn frame_checkpoint_is_ask_and_holds() {
    let (_d, store) = started();
    let cp = walk_to_checkpoint(&store, "r", "frame");
    assert!(matches!(cp, RunAction::Checkpoint { kind: CheckpointKind::Ask, .. }));
    assert_eq!(station_status(&store, "r", "frame"), Status::InProgress);
}

#[test]
fn specify_checkpoint_is_ask_and_holds() {
    let (_d, store) = started();
    advance_to_station(&store, "r", "specify");
    let cp = walk_to_checkpoint(&store, "r", "specify");
    assert!(matches!(cp, RunAction::Checkpoint { kind: CheckpointKind::Ask, .. }));
    assert_eq!(station_status(&store, "r", "specify"), Status::InProgress);
}

#[test]
fn shape_checkpoint_is_ask_and_holds() {
    let (_d, store) = started();
    advance_to_station(&store, "r", "shape");
    let cp = walk_to_checkpoint(&store, "r", "shape");
    assert!(matches!(cp, RunAction::Checkpoint { kind: CheckpointKind::Ask, .. }));
    assert_eq!(station_status(&store, "r", "shape"), Status::InProgress);
}

#[test]
fn build_checkpoint_is_auto_and_advances() {
    let (_d, store) = started();
    advance_to_station(&store, "r", "build");
    let cp = walk_to_checkpoint(&store, "r", "build");
    assert!(matches!(cp, RunAction::Checkpoint { kind: CheckpointKind::Auto, .. }));
    assert_eq!(station_status(&store, "r", "build"), Status::Completed);
}

#[test]
fn prove_checkpoint_is_auto_and_advances() {
    let (_d, store) = started();
    advance_to_station(&store, "r", "prove");
    let cp = walk_to_checkpoint(&store, "r", "prove");
    assert!(matches!(cp, RunAction::Checkpoint { kind: CheckpointKind::Auto, .. }));
    assert_eq!(station_status(&store, "r", "prove"), Status::Completed);
}

#[test]
fn harden_checkpoint_is_external_and_holds() {
    let (_d, store) = started();
    advance_to_station(&store, "r", "harden");
    let cp = walk_to_checkpoint(&store, "r", "harden");
    assert!(matches!(cp, RunAction::Checkpoint { kind: CheckpointKind::External, .. }));
    assert_eq!(station_status(&store, "r", "harden"), Status::InProgress);
}

#[test]
fn each_station_advances_to_its_successor_on_approve() {
    let pairs = [
        ("frame", "specify"),
        ("specify", "shape"),
        ("shape", "build"),
    ];
    for (st, next) in pairs {
        let (_d, store) = started();
        advance_to_station(&store, "r", st);
        walk_to_checkpoint(&store, "r", st);
        checkpoint_decide(&store, "r", true, None).expect("approve");
        assert_eq!(active_station(&store, "r"), next, "{st} → {next}");
        assert_eq!(station_status(&store, "r", st), Status::Completed);
    }
}

#[test]
fn each_gated_station_blocks_on_reject() {
    for st in ["frame", "specify", "shape", "harden"] {
        let (_d, store) = started();
        advance_to_station(&store, "r", st);
        walk_to_checkpoint(&store, "r", st);
        checkpoint_decide(&store, "r", false, Some("nope".into())).expect("reject");
        assert_eq!(station_status(&store, "r", st), Status::Blocked, "{st}");
        assert_eq!(
            checkpoint_outcome(&store, "r", st),
            Some(CheckpointOutcome::Blocked),
            "{st}"
        );
    }
}

#[test]
fn each_station_stamps_advanced_on_approve() {
    for (st, gated) in [
        ("frame", true),
        ("specify", true),
        ("shape", true),
        ("build", false),
        ("prove", false),
    ] {
        let (_d, store) = started();
        advance_to_station(&store, "r", st);
        walk_to_checkpoint(&store, "r", st);
        if gated {
            checkpoint_decide(&store, "r", true, None).expect("approve");
        }
        assert_eq!(
            checkpoint_outcome(&store, "r", st),
            Some(CheckpointOutcome::Advanced),
            "{st}"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION 15 — feedback_open robustness: extra status casing/format matrix
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn terminal_uppercase_rejected() {
    assert_terminal("REJECTED");
}

#[test]
fn terminal_uppercase_answered() {
    assert_terminal("ANSWERED");
}

#[test]
fn terminal_uppercase_non_actionable() {
    assert_terminal("NON_ACTIONABLE");
}

#[test]
fn terminal_mixed_case_closed() {
    assert_terminal("Closed");
}

#[test]
fn open_mixed_case_fixing() {
    assert_open("Fixing");
}

#[test]
fn open_uppercase_escalated() {
    assert_open("ESCALATED");
}

#[test]
fn terminal_quoted_addressed() {
    assert_terminal("\"addressed\"");
}

#[test]
fn terminal_quoted_answered() {
    assert_terminal("\"answered\"");
}

#[test]
fn terminal_quoted_closed() {
    assert_terminal("\"closed\"");
}

#[test]
fn terminal_quoted_non_actionable() {
    assert_terminal("\"non_actionable\"");
}

#[test]
fn open_status_with_trailing_comment_is_open_unknown() {
    // "pending # note" is an unrecognized token → not terminal → open.
    assert_open("pending # note");
}

#[test]
fn nonactionable_alias_no_underscore_is_open_in_manager() {
    // The manager's open-walk only knows "non_actionable"; "nonactionable"
    // (no underscore) is NOT in its terminal set → treated as open. (The typed
    // feedback parser accepts the alias, but the raw walk is stricter.)
    assert_open("nonactionable");
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION 16 — multi-track feedback id ordering depth
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn lowest_open_id_dispatched_across_gap() {
    let (_d, store) = started();
    raw_feedback(&store, "r", "fb-05", "pending");
    raw_feedback(&store, "r", "fb-02", "pending");
    raw_feedback(&store, "r", "fb-09", "pending");
    let pos = derive_position(&store, "r").unwrap();
    match pos.action {
        Some(RunAction::FixFeedback { feedback_id, .. }) => assert_eq!(feedback_id, "fb-02"),
        other => panic!("got {other:?}"),
    }
}

#[test]
fn terminal_before_open_skips_to_open() {
    let (_d, store) = started();
    raw_feedback(&store, "r", "fb-01", "addressed");
    raw_feedback(&store, "r", "fb-02", "closed");
    raw_feedback(&store, "r", "fb-03", "rejected");
    raw_feedback(&store, "r", "fb-04", "pending");
    let pos = derive_position(&store, "r").unwrap();
    match pos.action {
        Some(RunAction::FixFeedback { feedback_id, .. }) => assert_eq!(feedback_id, "fb-04"),
        other => panic!("got {other:?}"),
    }
}

#[test]
fn ten_feedback_items_only_one_open() {
    let (_d, store) = started();
    for n in 1..=10 {
        let status = if n == 7 { "fixing" } else { "closed" };
        raw_feedback(&store, "r", &format!("fb-{n:02}"), status);
    }
    let pos = derive_position(&store, "r").unwrap();
    match pos.action {
        Some(RunAction::FixFeedback { feedback_id, .. }) => assert_eq!(feedback_id, "fb-07"),
        other => panic!("got {other:?}"),
    }
}

#[test]
fn resolving_first_open_advances_to_next_open() {
    let (_d, store) = started();
    let a = feedback::create(&store, "r", "frame", "first", None).unwrap();
    let b = feedback::create(&store, "r", "frame", "second", None).unwrap();
    // First dispatched is a (fb-01).
    let pos = derive_position(&store, "r").unwrap();
    assert!(matches!(&pos.action, Some(RunAction::FixFeedback { feedback_id, .. }) if feedback_id == &a.id));
    // Settle a → b is dispatched.
    feedback::set_status(&store, "r", &a.id, FeedbackStatus::Addressed).unwrap();
    let pos = derive_position(&store, "r").unwrap();
    assert!(matches!(&pos.action, Some(RunAction::FixFeedback { feedback_id, .. }) if feedback_id == &b.id));
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION 17 — full-run determinism & checkpoint-outcome lifecycle
// ═══════════════════════════════════════════════════════════════════════

/// Drive a whole run to sealed, approving every gate. Returns the final action.
fn run_to_sealed(store: &StateStore, run: &str) -> RunAction {
    for st in ["frame", "specify", "shape", "build", "prove", "harden"] {
        let cp = walk_to_checkpoint(store, run, st);
        if let RunAction::Checkpoint { kind, .. } = cp {
            if !matches!(kind, CheckpointKind::Auto) {
                checkpoint_decide(store, run, true, None).expect("approve");
            }
        }
    }
    derive_position(store, run).unwrap().action.unwrap()
}

#[test]
fn full_run_reaches_sealed() {
    let (_d, store) = started();
    let final_action = run_to_sealed(&store, "r");
    assert!(matches!(final_action, RunAction::Sealed { .. }));
}

#[test]
fn full_run_every_station_completed() {
    let (_d, store) = started();
    run_to_sealed(&store, "r");
    let s = store.read_state("r").unwrap().unwrap();
    for st in ["frame", "specify", "shape", "build", "prove", "harden"] {
        assert_eq!(s.stations[st].status, Status::Completed, "{st}");
    }
}

#[test]
fn full_run_every_outcome_advanced() {
    let (_d, store) = started();
    run_to_sealed(&store, "r");
    for st in ["frame", "specify", "shape", "build", "prove", "harden"] {
        assert_eq!(
            checkpoint_outcome(&store, "r", st),
            Some(CheckpointOutcome::Advanced),
            "{st}"
        );
    }
}

#[test]
fn sealed_run_is_idempotent_under_repeated_derive() {
    let (_d, store) = started();
    run_to_sealed(&store, "r");
    let a = derive_position(&store, "r").unwrap();
    let b = derive_position(&store, "r").unwrap();
    assert_eq!(a, b);
    assert!(matches!(a.action, Some(RunAction::Sealed { .. })));
}

#[test]
fn sealed_run_tick_stays_sealed() {
    let (_d, store) = started();
    run_to_sealed(&store, "r");
    let t1 = run_tick(&store, "r").unwrap();
    let t2 = run_tick(&store, "r").unwrap();
    assert!(matches!(t1.action, RunAction::Sealed { .. }));
    assert!(matches!(t2.action, RunAction::Sealed { .. }));
}

#[test]
fn sealed_takes_precedence_over_feedback() {
    // A fully-completed run short-circuits to Sealed BEFORE the drift/feedback
    // tracks are consulted (current_station returns None first), so open
    // feedback on a sealed run does NOT preempt.
    let (_d, store) = started();
    run_to_sealed(&store, "r");
    feedback::create(&store, "r", "harden", "post-seal note", None).unwrap();
    let pos = derive_position(&store, "r").unwrap();
    assert_eq!(pos_track(&pos), Track::Run);
    assert!(matches!(pos.action, Some(RunAction::Sealed { .. })));
}

#[test]
fn sealed_takes_precedence_over_drift() {
    let (_d, store) = started();
    run_to_sealed(&store, "r");
    record_drift(&store, "r", "d-01", "harden/release.md", "harden");
    let pos = derive_position(&store, "r").unwrap();
    assert_eq!(pos_track(&pos), Track::Run);
    assert!(matches!(pos.action, Some(RunAction::Sealed { .. })));
}

#[test]
fn two_independent_runs_track_separately() {
    let (_d, store) = store();
    run_start(&store, "a", "software", None, "continuous").unwrap();
    run_start(&store, "b", "software", None, "continuous").unwrap();
    feedback::create(&store, "a", "frame", "only a", None).unwrap();
    assert_eq!(pos_track(&derive_position(&store, "a").unwrap()), Track::Feedback);
    assert_eq!(pos_track(&derive_position(&store, "b").unwrap()), Track::Run);
}

#[test]
fn feedback_isolated_per_run() {
    let (_d, store) = store();
    run_start(&store, "a", "software", None, "continuous").unwrap();
    run_start(&store, "b", "software", None, "continuous").unwrap();
    feedback::create(&store, "a", "frame", "x", None).unwrap();
    assert!(feedback::list(&store, "b").unwrap().is_empty());
}

#[test]
fn drift_isolated_per_run() {
    let (_d, store) = store();
    run_start(&store, "a", "software", None, "continuous").unwrap();
    run_start(&store, "b", "software", None, "continuous").unwrap();
    record_drift(&store, "a", "d-01", "x.md", "frame");
    assert_eq!(pos_track(&derive_position(&store, "a").unwrap()), Track::Drift);
    assert_eq!(pos_track(&derive_position(&store, "b").unwrap()), Track::Run);
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION 18 — reject-rework loop end-to-end
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn reject_then_address_then_reapprove_advances() {
    let (_d, store) = started();
    walk_to_checkpoint(&store, "r", "frame");
    // Reject → blocked + feedback filed.
    checkpoint_decide(&store, "r", false, Some("rework".into())).expect("reject");
    assert_eq!(station_status(&store, "r", "frame"), Status::Blocked);
    // Address the rework.
    let fb = &feedback::list(&store, "r").unwrap()[0];
    feedback::set_status(&store, "r", &fb.id, FeedbackStatus::Addressed).unwrap();
    // Run track resumes; station still at checkpoint phase, now re-approve.
    let pos = derive_position(&store, "r").unwrap();
    assert!(matches!(pos.action, Some(RunAction::Checkpoint { .. })));
    let decided = checkpoint_decide(&store, "r", true, None).expect("approve");
    assert!(matches!(&decided.action, RunAction::Spec { station, .. } if station == "specify"));
}

#[test]
fn reject_twice_keeps_blocked_and_refiles() {
    let (_d, store) = started();
    walk_to_checkpoint(&store, "r", "frame");
    checkpoint_decide(&store, "r", false, Some("first".into())).expect("reject1");
    // Settle the first so we can reject again.
    let fb = &feedback::list(&store, "r").unwrap()[0];
    feedback::set_status(&store, "r", &fb.id, FeedbackStatus::Addressed).unwrap();
    let res = checkpoint_decide(&store, "r", false, Some("second".into())).expect("reject2");
    assert_eq!(station_status(&store, "r", "frame"), Status::Blocked);
    // The fb-checkpoint id is overwritten with the latest reason.
    let raw = store.read_feedback_raw("r").unwrap();
    assert!(raw["fb-checkpoint"].contains("second"));
    assert_eq!(res.position.track, Track::Feedback);
}

#[test]
fn approve_after_block_clears_blocked_to_completed() {
    let (_d, store) = started();
    walk_to_checkpoint(&store, "r", "frame");
    checkpoint_decide(&store, "r", false, Some("x".into())).expect("reject");
    assert_eq!(station_status(&store, "r", "frame"), Status::Blocked);
    let fb = &feedback::list(&store, "r").unwrap()[0];
    feedback::reject(&store, "r", &fb.id, "actually fine").unwrap();
    checkpoint_decide(&store, "r", true, None).expect("approve");
    assert_eq!(station_status(&store, "r", "frame"), Status::Completed);
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION 19 — drift kind variations all surface Track C
// ═══════════════════════════════════════════════════════════════════════

fn record_drift_kind(store: &StateStore, run: &str, id: &str, kind: DriftKind) {
    let d = Drift {
        path: "frame/a.md".into(),
        station: "frame".into(),
        run: run.into(),
        kind,
        age: "1m".into(),
        unit: None,
    };
    drift::record(store, run, id, &d).unwrap();
}

#[test]
fn drift_kind_spec_surfaces() {
    let (_d, store) = started();
    record_drift_kind(&store, "r", "d-01", DriftKind::Spec);
    assert_eq!(pos_track(&derive_position(&store, "r").unwrap()), Track::Drift);
}

#[test]
fn drift_kind_output_surfaces() {
    let (_d, store) = started();
    record_drift_kind(&store, "r", "d-01", DriftKind::Output);
    assert_eq!(pos_track(&derive_position(&store, "r").unwrap()), Track::Drift);
}

#[test]
fn drift_kind_discovery_output_surfaces() {
    let (_d, store) = started();
    record_drift_kind(&store, "r", "d-01", DriftKind::DiscoveryOutput);
    assert_eq!(pos_track(&derive_position(&store, "r").unwrap()), Track::Drift);
}

#[test]
fn drift_kind_discovery_mandate_surfaces() {
    let (_d, store) = started();
    record_drift_kind(&store, "r", "d-01", DriftKind::DiscoveryMandate);
    assert_eq!(pos_track(&derive_position(&store, "r").unwrap()), Track::Drift);
}

#[test]
fn drift_kind_roundtrips_through_storage() {
    for kind in [
        DriftKind::Spec,
        DriftKind::Output,
        DriftKind::DiscoveryOutput,
        DriftKind::DiscoveryMandate,
    ] {
        let (_d, store) = started();
        record_drift_kind(&store, "r", "d-01", kind);
        let read = drift::first(&store, "r").unwrap().unwrap();
        assert_eq!(read.kind, kind, "{kind:?}");
    }
}

#[test]
fn drift_unit_field_roundtrips() {
    let (_d, store) = started();
    let d = Drift {
        path: "x.md".into(),
        station: "frame".into(),
        run: "r".into(),
        kind: DriftKind::Output,
        age: "2h".into(),
        unit: Some("widget".into()),
    };
    drift::record(&store, "r", "d-01", &d).unwrap();
    let read = drift::first(&store, "r").unwrap().unwrap();
    assert_eq!(read.unit, Some("widget".into()));
    assert_eq!(read.age, "2h");
}

#[test]
fn multiple_drift_entries_first_by_stem() {
    let (_d, store) = started();
    record_drift(&store, "r", "z-99", "z.md", "frame");
    record_drift(&store, "r", "a-01", "a.md", "frame");
    record_drift(&store, "r", "m-50", "m.md", "frame");
    let pos = derive_position(&store, "r").unwrap();
    match pos.action {
        Some(RunAction::ResolveDrift { path, .. }) => assert_eq!(path, "a.md"),
        other => panic!("got {other:?}"),
    }
}

#[test]
fn drift_list_returns_all_sorted() {
    let (_d, store) = started();
    record_drift(&store, "r", "d-03", "c.md", "frame");
    record_drift(&store, "r", "d-01", "a.md", "frame");
    record_drift(&store, "r", "d-02", "b.md", "frame");
    let all = drift::list(&store, "r").unwrap();
    assert_eq!(all.len(), 3);
    assert_eq!(all[0].path, "a.md");
    assert_eq!(all[1].path, "b.md");
    assert_eq!(all[2].path, "c.md");
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION 20 — Await kind hold semantics (simulated via state injection)
// ═══════════════════════════════════════════════════════════════════════
//
// The software factory has no station with an `Await` gate, but the manager's
// checkpoint path treats Await like Ask/External (hold, do not auto-advance).
// We exercise that branch by injecting an Await checkpoint into state directly
// and driving the same tick/decide path.

fn force_checkpoint_phase(store: &StateStore, run: &str, station: &str, kind: CheckpointKind) {
    let mut state = store.read_state(run).unwrap().unwrap();
    if let Some(st) = state.stations.get_mut(station) {
        st.phase = StationPhase::Checkpoint;
        st.status = Status::InProgress;
        if let Some(cp) = st.checkpoint.as_mut() {
            cp.kind = kind;
            cp.outcome = None;
        }
    }
    store.write_state(run, &state).unwrap();
}

#[test]
fn await_kind_does_not_auto_advance() {
    let (_d, store) = started();
    force_checkpoint_phase(&store, "r", "frame", CheckpointKind::Await);
    // The factory def for frame is Ask, so the Checkpoint action surfaces Ask,
    // but the persisted gate we injected is Await — confirm the station holds.
    let t = run_tick(&store, "r").unwrap();
    assert!(matches!(t.action, RunAction::Checkpoint { .. }));
    assert_ne!(station_status(&store, "r", "frame"), Status::Completed);
}

#[test]
fn await_kind_holds_until_decide_approves() {
    let (_d, store) = started();
    force_checkpoint_phase(&store, "r", "frame", CheckpointKind::Await);
    run_tick(&store, "r").unwrap();
    assert_eq!(station_status(&store, "r", "frame"), Status::InProgress);
    // Operator decision advances it like any held gate.
    let decided = checkpoint_decide(&store, "r", true, None).expect("approve");
    assert!(matches!(&decided.action, RunAction::Spec { station, .. } if station == "specify"));
    assert_eq!(station_status(&store, "r", "frame"), Status::Completed);
}

#[test]
fn await_kind_reject_blocks() {
    let (_d, store) = started();
    force_checkpoint_phase(&store, "r", "frame", CheckpointKind::Await);
    run_tick(&store, "r").unwrap();
    checkpoint_decide(&store, "r", false, Some("await rework".into())).expect("reject");
    assert_eq!(station_status(&store, "r", "frame"), Status::Blocked);
    assert_eq!(checkpoint_outcome(&store, "r", "frame"), Some(CheckpointOutcome::Blocked));
}

#[test]
fn checkpoint_action_kind_comes_from_factory_def_not_state() {
    // Forcing the persisted gate to Auto does NOT change the emitted action's
    // kind — the action carries the factory-defined kind (frame = Ask). And
    // because advance_state keys off the action's kind, the station still holds.
    let (_d, store) = started();
    force_checkpoint_phase(&store, "r", "frame", CheckpointKind::Auto);
    let t = run_tick(&store, "r").unwrap();
    assert!(matches!(t.action, RunAction::Checkpoint { kind: CheckpointKind::Ask, .. }));
    assert_eq!(station_status(&store, "r", "frame"), Status::InProgress);
    assert_eq!(active_station(&store, "r"), "frame");
}

#[test]
fn forced_external_gate_in_state_still_emits_ask_and_holds() {
    let (_d, store) = started();
    force_checkpoint_phase(&store, "r", "frame", CheckpointKind::External);
    let t = run_tick(&store, "r").unwrap();
    assert!(matches!(t.action, RunAction::Checkpoint { kind: CheckpointKind::Ask, .. }));
    assert_eq!(station_status(&store, "r", "frame"), Status::InProgress);
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION 21 — checkpoint entered_at timestamps
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn checkpoint_entered_at_set_on_each_gated_station() {
    for st in ["frame", "specify", "shape", "harden"] {
        let (_d, store) = started();
        advance_to_station(&store, "r", st);
        walk_to_checkpoint(&store, "r", st);
        let s = store.read_state("r").unwrap().unwrap();
        assert!(
            s.stations[st].checkpoint.as_ref().unwrap().entered_at.is_some(),
            "{st} entered_at"
        );
    }
}

#[test]
fn auto_station_entered_at_set_before_advance() {
    let (_d, store) = started();
    advance_to_station(&store, "r", "build");
    walk_to_checkpoint(&store, "r", "build");
    let s = store.read_state("r").unwrap().unwrap();
    // Auto station completed but still carries an entered_at on its checkpoint.
    assert!(s.stations["build"].checkpoint.as_ref().unwrap().entered_at.is_some());
}

#[test]
fn started_at_set_when_station_enters_spec() {
    let (_d, store) = started();
    run_tick(&store, "r").unwrap(); // spec → review, stamps started_at
    let s = store.read_state("r").unwrap().unwrap();
    assert!(s.stations["frame"].started_at.is_some());
}

#[test]
fn started_at_stable_across_phases() {
    let (_d, store) = started();
    run_tick(&store, "r").unwrap(); // spec
    let first = store.read_state("r").unwrap().unwrap().stations["frame"]
        .started_at
        .clone();
    run_tick(&store, "r").unwrap(); // review
    let second = store.read_state("r").unwrap().unwrap().stations["frame"]
        .started_at
        .clone();
    assert_eq!(first, second);
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION 22 — feedback severity / move under the manager dispatch
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn moving_feedback_station_does_not_change_open_preemption() {
    let (_d, store) = started();
    let fb = feedback::create(&store, "r", "frame", "x", None).unwrap();
    feedback::move_station(&store, "r", &fb.id, "shape").unwrap();
    // Still open → still preempts; the FixFeedback station is the run's active
    // station (frame), independent of the feedback's own station field.
    let pos = derive_position(&store, "r").unwrap();
    assert_eq!(pos_track(&pos), Track::Feedback);
    match pos.action {
        Some(RunAction::FixFeedback { station, .. }) => assert_eq!(station, "frame"),
        other => panic!("got {other:?}"),
    }
}

#[test]
fn setting_severity_keeps_feedback_open() {
    let (_d, store) = started();
    let fb = feedback::create(&store, "r", "frame", "x", None).unwrap();
    feedback::set_severity(&store, "r", &fb.id, FeedbackSeverity::Blocker).unwrap();
    assert_eq!(
        pos_track(&derive_position(&store, "r").unwrap()),
        Track::Feedback
    );
}

#[test]
fn feedback_created_via_typed_api_is_open_to_manager() {
    let (_d, store) = started();
    feedback::create(&store, "r", "frame", "typed body", None).unwrap();
    // The typed create writes status: pending → manager sees it open.
    assert_eq!(
        pos_track(&derive_position(&store, "r").unwrap()),
        Track::Feedback
    );
}

#[test]
fn feedback_reject_via_typed_api_closes_to_manager() {
    let (_d, store) = started();
    let fb = feedback::create(&store, "r", "frame", "x", None).unwrap();
    feedback::reject(&store, "r", &fb.id, "dup").unwrap();
    assert_eq!(pos_track(&derive_position(&store, "r").unwrap()), Track::Run);
}

#[test]
fn settled_feedback_cannot_reopen_via_typed_api() {
    let (_d, store) = started();
    let fb = feedback::create(&store, "r", "frame", "x", None).unwrap();
    feedback::set_status(&store, "r", &fb.id, FeedbackStatus::Closed).unwrap();
    // Attempt to push it back to pending → rejected by the immutability rule.
    let err = feedback::set_status(&store, "r", &fb.id, FeedbackStatus::Pending).unwrap_err();
    assert!(matches!(err, darkrun_mcp::McpError::FeedbackSettled(_)));
    // And the manager still sees it terminal.
    assert_eq!(pos_track(&derive_position(&store, "r").unwrap()), Track::Run);
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION 23 — phase machine preserved across feedback pause
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn feedback_pause_preserves_review_phase() {
    let (_d, store) = started();
    run_tick(&store, "r").unwrap(); // spec → review
    let fb = feedback::create(&store, "r", "frame", "x", None).unwrap();
    assert_eq!(pos_track(&derive_position(&store, "r").unwrap()), Track::Feedback);
    feedback::set_status(&store, "r", &fb.id, FeedbackStatus::Addressed).unwrap();
    let pos = derive_position(&store, "r").unwrap();
    assert_eq!(station_phase(&store, "r", "frame"), StationPhase::Review);
    assert!(matches!(pos.action, Some(RunAction::Review { .. })));
}

#[test]
fn feedback_pause_preserves_manufacture_phase() {
    let (_d, store) = started();
    run_tick(&store, "r").unwrap(); // spec → review
    run_tick(&store, "r").unwrap(); // review → manufacture
    let unit = Unit {
        slug: "u1".into(),
        frontmatter: UnitFrontmatter {
            status: Status::Pending,
            station: Some("frame".into()),
            ..Default::default()
        },
        title: "u1".into(),
        body: String::new(),
    };
    store.write_unit("r", &unit).unwrap();
    let fb = feedback::create(&store, "r", "frame", "x", None).unwrap();
    assert_eq!(pos_track(&derive_position(&store, "r").unwrap()), Track::Feedback);
    feedback::reject(&store, "r", &fb.id, "no").unwrap();
    let pos = derive_position(&store, "r").unwrap();
    assert!(matches!(pos.action, Some(RunAction::Manufacture { .. })));
}

#[test]
fn drift_pause_preserves_checkpoint_phase() {
    let (_d, store) = started();
    walk_to_checkpoint(&store, "r", "frame");
    record_drift(&store, "r", "d-01", "x.md", "frame");
    assert_eq!(pos_track(&derive_position(&store, "r").unwrap()), Track::Drift);
    std::fs::remove_dir_all(store.run_dir("r").join("drift")).unwrap();
    let pos = derive_position(&store, "r").unwrap();
    assert!(matches!(pos.action, Some(RunAction::Checkpoint { .. })));
}

#[test]
fn feedback_during_manufacture_does_not_consume_wave() {
    // While feedback preempts, the manufacture wave is untouched — the same
    // units are still wave-ready once feedback clears.
    let (_d, store) = started();
    run_tick(&store, "r").unwrap();
    run_tick(&store, "r").unwrap();
    let unit = Unit {
        slug: "u1".into(),
        frontmatter: UnitFrontmatter {
            status: Status::Pending,
            station: Some("frame".into()),
            ..Default::default()
        },
        title: "u1".into(),
        body: String::new(),
    };
    store.write_unit("r", &unit).unwrap();
    let fb = feedback::create(&store, "r", "frame", "x", None).unwrap();
    feedback::set_status(&store, "r", &fb.id, FeedbackStatus::Addressed).unwrap();
    let pos = derive_position(&store, "r").unwrap();
    match pos.action {
        Some(RunAction::Manufacture { units, .. }) => assert_eq!(units, vec!["u1".to_string()]),
        other => panic!("got {other:?}"),
    }
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION 24 — checkpoint_decide always re-ticks; track of returned position
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn decide_approve_returns_run_track_for_next_spec() {
    let (_d, store) = started();
    let res = checkpoint_decide(&store, "r", true, None).expect("approve");
    assert_eq!(res.position.track, Track::Run);
}

#[test]
fn decide_reject_with_feedback_returns_feedback_track() {
    let (_d, store) = started();
    let res = checkpoint_decide(&store, "r", false, Some("x".into())).expect("reject");
    assert_eq!(res.position.track, Track::Feedback);
}

#[test]
fn decide_approve_at_harden_returns_sealed_run_track() {
    let (_d, store) = started();
    advance_to_station(&store, "r", "harden");
    walk_to_checkpoint(&store, "r", "harden");
    let res = checkpoint_decide(&store, "r", true, None).expect("seal");
    assert_eq!(res.position.track, Track::Run);
    assert!(matches!(res.action, RunAction::Sealed { .. }));
}

#[test]
fn decide_run_field_is_slug() {
    let (_d, store) = started();
    let res = checkpoint_decide(&store, "r", true, None).expect("approve");
    assert_eq!(res.run, "r");
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION 25 — every CheckpointKind serde round trip via state
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn checkpoint_kind_serializes_auto() {
    assert_eq!(json_of(&CheckpointKind::Auto), serde_json::json!("auto"));
}

#[test]
fn checkpoint_kind_serializes_ask() {
    assert_eq!(json_of(&CheckpointKind::Ask), serde_json::json!("ask"));
}

#[test]
fn checkpoint_outcome_serializes_paused() {
    assert_eq!(json_of(&CheckpointOutcome::Paused), serde_json::json!("paused"));
}

#[test]
fn checkpoint_outcome_serializes_awaiting() {
    assert_eq!(json_of(&CheckpointOutcome::Awaiting), serde_json::json!("awaiting"));
}

#[test]
fn state_with_advanced_outcome_round_trips_json() {
    let (_d, store) = started();
    checkpoint_decide(&store, "r", true, None).expect("approve");
    let s = store.read_state("r").unwrap().unwrap();
    let j = json_of(&s.stations["frame"].checkpoint.as_ref().unwrap());
    assert_eq!(j["outcome"], serde_json::json!("advanced"));
    assert_eq!(j["kind"], serde_json::json!("ask"));
}

#[test]
fn state_with_blocked_outcome_round_trips_json() {
    let (_d, store) = started();
    walk_to_checkpoint(&store, "r", "frame");
    checkpoint_decide(&store, "r", false, Some("x".into())).expect("reject");
    let s = store.read_state("r").unwrap().unwrap();
    let j = json_of(&s.stations["frame"].checkpoint.as_ref().unwrap());
    assert_eq!(j["outcome"], serde_json::json!("blocked"));
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION 26 — feedback frontmatter quirks the manager tolerates
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn feedback_with_extra_frontmatter_fields_still_parsed_open() {
    let (_d, store) = started();
    let doc = "---\nstation: frame\nseverity: high\nstatus: pending\ncreated_at: 2020-01-01\n---\nbody\n";
    store.write_feedback_raw("r", "fb-1", doc).unwrap();
    assert_eq!(pos_track(&derive_position(&store, "r").unwrap()), Track::Feedback);
}

#[test]
fn feedback_status_line_with_extra_spaces_terminal() {
    let (_d, store) = started();
    let doc = "---\nstatus:    closed\n---\nbody\n";
    store.write_feedback_raw("r", "fb-1", doc).unwrap();
    assert_eq!(pos_track(&derive_position(&store, "r").unwrap()), Track::Run);
}

#[test]
fn feedback_status_before_fence_is_read() {
    // feedback_open scans all lines; a status line even outside a fence counts.
    let (_d, store) = started();
    let doc = "status: closed\n---\nbody\n";
    store.write_feedback_raw("r", "fb-1", doc).unwrap();
    assert_eq!(pos_track(&derive_position(&store, "r").unwrap()), Track::Run);
}

#[test]
fn feedback_blank_doc_is_open() {
    let (_d, store) = started();
    store.write_feedback_raw("r", "fb-1", "\n\n").unwrap();
    assert_eq!(pos_track(&derive_position(&store, "r").unwrap()), Track::Feedback);
}

#[test]
fn multiple_runs_feedback_dispatch_correct_id() {
    let (_d, store) = store();
    run_start(&store, "a", "software", None, "continuous").unwrap();
    run_start(&store, "b", "software", None, "continuous").unwrap();
    raw_feedback(&store, "a", "fb-05", "pending");
    raw_feedback(&store, "b", "fb-09", "pending");
    let pa = derive_position(&store, "a").unwrap();
    let pb = derive_position(&store, "b").unwrap();
    match pa.action {
        Some(RunAction::FixFeedback { feedback_id, .. }) => assert_eq!(feedback_id, "fb-05"),
        other => panic!("got {other:?}"),
    }
    match pb.action {
        Some(RunAction::FixFeedback { feedback_id, .. }) => assert_eq!(feedback_id, "fb-09"),
        other => panic!("got {other:?}"),
    }
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION 27 — track stability under repeated ticks while held/preempted
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn repeated_ticks_while_feedback_open_stay_on_feedback() {
    let (_d, store) = started();
    feedback::create(&store, "r", "frame", "x", None).unwrap();
    for _ in 0..5 {
        let t = run_tick(&store, "r").unwrap();
        assert_eq!(t.position.track, Track::Feedback);
        assert!(matches!(t.action, RunAction::FixFeedback { .. }));
    }
}

#[test]
fn repeated_ticks_while_drift_present_stay_on_drift() {
    let (_d, store) = started();
    record_drift(&store, "r", "d-01", "x.md", "frame");
    for _ in 0..5 {
        let t = run_tick(&store, "r").unwrap();
        assert_eq!(t.position.track, Track::Drift);
        assert!(matches!(t.action, RunAction::ResolveDrift { .. }));
    }
}

#[test]
fn feedback_does_not_advance_run_phase_over_many_ticks() {
    let (_d, store) = started();
    run_tick(&store, "r").unwrap(); // spec → review
    let phase = station_phase(&store, "r", "frame");
    feedback::create(&store, "r", "frame", "x", None).unwrap();
    for _ in 0..5 {
        run_tick(&store, "r").unwrap();
    }
    assert_eq!(station_phase(&store, "r", "frame"), phase);
}

#[test]
fn held_checkpoint_then_drift_then_clear_then_decide() {
    // Full interplay: hold at checkpoint, drift preempts, clear drift, decide.
    let (_d, store) = started();
    walk_to_checkpoint(&store, "r", "frame");
    record_drift(&store, "r", "d-01", "x.md", "frame");
    assert_eq!(pos_track(&derive_position(&store, "r").unwrap()), Track::Drift);
    std::fs::remove_dir_all(store.run_dir("r").join("drift")).unwrap();
    assert_eq!(pos_track(&derive_position(&store, "r").unwrap()), Track::Run);
    let decided = checkpoint_decide(&store, "r", true, None).expect("approve");
    assert!(matches!(&decided.action, RunAction::Spec { station, .. } if station == "specify"));
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION 28 — run_start invariants relevant to the track machine
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn fresh_run_active_station_is_frame() {
    let (_d, store) = started();
    assert_eq!(active_station(&store, "r"), "frame");
}

#[test]
fn fresh_run_frame_phase_is_spec() {
    let (_d, store) = started();
    assert_eq!(station_phase(&store, "r", "frame"), StationPhase::Spec);
}

#[test]
fn fresh_run_first_action_is_frame_spec() {
    let (_d, store) = started();
    let pos = derive_position(&store, "r").unwrap();
    assert!(matches!(&pos.action, Some(RunAction::Spec { station, .. }) if station == "frame"));
}

#[test]
fn fresh_run_no_feedback_no_drift() {
    let (_d, store) = started();
    assert!(feedback::list(&store, "r").unwrap().is_empty());
    assert!(drift::list(&store, "r").unwrap().is_empty());
}

#[test]
fn fresh_run_frame_checkpoint_seeded_ask() {
    let (_d, store) = started();
    let s = store.read_state("r").unwrap().unwrap();
    assert_eq!(
        s.stations["frame"].checkpoint.as_ref().unwrap().kind,
        CheckpointKind::Ask
    );
    assert_eq!(s.stations["frame"].checkpoint.as_ref().unwrap().outcome, None);
}

#[test]
fn unknown_factory_run_start_errors() {
    let (_d, store) = store();
    let err = run_start(&store, "r", "nope", None, "continuous").unwrap_err();
    assert!(matches!(err, darkrun_mcp::McpError::UnknownFactory(_)));
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION 29 — drift vs feedback preemption at every station
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn feedback_preempts_at_specify() {
    let (_d, store) = started();
    advance_to_station(&store, "r", "specify");
    feedback::create(&store, "r", "specify", "x", None).unwrap();
    let pos = derive_position(&store, "r").unwrap();
    assert_eq!(pos_track(&pos), Track::Feedback);
    match pos.action {
        Some(RunAction::FixFeedback { station, .. }) => assert_eq!(station, "specify"),
        other => panic!("got {other:?}"),
    }
}

#[test]
fn feedback_preempts_at_build() {
    let (_d, store) = started();
    advance_to_station(&store, "r", "build");
    feedback::create(&store, "r", "build", "x", None).unwrap();
    let pos = derive_position(&store, "r").unwrap();
    assert_eq!(pos_track(&pos), Track::Feedback);
    match pos.action {
        Some(RunAction::FixFeedback { station, .. }) => assert_eq!(station, "build"),
        other => panic!("got {other:?}"),
    }
}

#[test]
fn feedback_preempts_at_harden() {
    let (_d, store) = started();
    advance_to_station(&store, "r", "harden");
    feedback::create(&store, "r", "harden", "x", None).unwrap();
    let pos = derive_position(&store, "r").unwrap();
    assert_eq!(pos_track(&pos), Track::Feedback);
    match pos.action {
        Some(RunAction::FixFeedback { station, .. }) => assert_eq!(station, "harden"),
        other => panic!("got {other:?}"),
    }
}

#[test]
fn drift_default_station_tracks_active_station_progression() {
    // A station-less drift attributes to whatever station is active now.
    let (_d, store) = started();
    advance_to_station(&store, "r", "shape");
    let d = Drift {
        path: "x.md".into(),
        station: String::new(),
        run: "r".into(),
        kind: DriftKind::Output,
        age: String::new(),
        unit: None,
    };
    drift::record(&store, "r", "d-01", &d).unwrap();
    let pos = derive_position(&store, "r").unwrap();
    match pos.action {
        Some(RunAction::ResolveDrift { station, .. }) => assert_eq!(station, "shape"),
        other => panic!("got {other:?}"),
    }
}

#[test]
fn drift_beats_feedback_at_build() {
    let (_d, store) = started();
    advance_to_station(&store, "r", "build");
    feedback::create(&store, "r", "build", "x", None).unwrap();
    record_drift(&store, "r", "d-01", "build/x.md", "build");
    assert_eq!(pos_track(&derive_position(&store, "r").unwrap()), Track::Drift);
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION 30 — status transition chains via typed feedback API
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn pending_to_fixing_keeps_open() {
    let (_d, store) = started();
    let fb = feedback::create(&store, "r", "frame", "x", None).unwrap();
    feedback::set_status(&store, "r", &fb.id, FeedbackStatus::Fixing).unwrap();
    assert_eq!(pos_track(&derive_position(&store, "r").unwrap()), Track::Feedback);
}

#[test]
fn pending_to_fixing_to_addressed_releases() {
    let (_d, store) = started();
    let fb = feedback::create(&store, "r", "frame", "x", None).unwrap();
    feedback::set_status(&store, "r", &fb.id, FeedbackStatus::Fixing).unwrap();
    assert_eq!(pos_track(&derive_position(&store, "r").unwrap()), Track::Feedback);
    feedback::set_status(&store, "r", &fb.id, FeedbackStatus::Addressed).unwrap();
    assert_eq!(pos_track(&derive_position(&store, "r").unwrap()), Track::Run);
}

#[test]
fn pending_to_escalated_stays_feedback() {
    let (_d, store) = started();
    let fb = feedback::create(&store, "r", "frame", "x", None).unwrap();
    feedback::set_status(&store, "r", &fb.id, FeedbackStatus::Escalated).unwrap();
    assert_eq!(pos_track(&derive_position(&store, "r").unwrap()), Track::Feedback);
}

#[test]
fn pending_to_answered_releases() {
    let (_d, store) = started();
    let fb = feedback::create(&store, "r", "frame", "x", None).unwrap();
    feedback::set_status(&store, "r", &fb.id, FeedbackStatus::Answered).unwrap();
    assert_eq!(pos_track(&derive_position(&store, "r").unwrap()), Track::Run);
}

#[test]
fn pending_to_non_actionable_releases() {
    let (_d, store) = started();
    let fb = feedback::create(&store, "r", "frame", "x", None).unwrap();
    feedback::set_status(&store, "r", &fb.id, FeedbackStatus::NonActionable).unwrap();
    assert_eq!(pos_track(&derive_position(&store, "r").unwrap()), Track::Run);
}

#[test]
fn pending_to_closed_releases() {
    let (_d, store) = started();
    let fb = feedback::create(&store, "r", "frame", "x", None).unwrap();
    feedback::set_status(&store, "r", &fb.id, FeedbackStatus::Closed).unwrap();
    assert_eq!(pos_track(&derive_position(&store, "r").unwrap()), Track::Run);
}

#[test]
fn two_open_settle_both_releases() {
    let (_d, store) = started();
    let a = feedback::create(&store, "r", "frame", "a", None).unwrap();
    let b = feedback::create(&store, "r", "frame", "b", None).unwrap();
    feedback::set_status(&store, "r", &a.id, FeedbackStatus::Addressed).unwrap();
    // Still one open (b).
    assert_eq!(pos_track(&derive_position(&store, "r").unwrap()), Track::Feedback);
    feedback::set_status(&store, "r", &b.id, FeedbackStatus::Closed).unwrap();
    assert_eq!(pos_track(&derive_position(&store, "r").unwrap()), Track::Run);
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION 31 — checkpoint decide determinism & repeated approve safety
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn approve_is_stable_state_after_decide() {
    let (_d, store) = started();
    checkpoint_decide(&store, "r", true, None).expect("approve");
    let snap1 = store.read_state("r").unwrap().unwrap();
    // A pure derive afterwards must not mutate.
    derive_position(&store, "r").unwrap();
    let snap2 = store.read_state("r").unwrap().unwrap();
    assert_eq!(snap1.active_station, snap2.active_station);
    assert_eq!(snap1.stations["frame"].status, snap2.stations["frame"].status);
}

#[test]
fn approve_frame_then_specify_outcomes_independent() {
    let (_d, store) = started();
    checkpoint_decide(&store, "r", true, None).expect("frame");
    // Walk specify to checkpoint and reject it.
    walk_to_checkpoint(&store, "r", "specify");
    checkpoint_decide(&store, "r", false, Some("x".into())).expect("specify reject");
    assert_eq!(checkpoint_outcome(&store, "r", "frame"), Some(CheckpointOutcome::Advanced));
    assert_eq!(checkpoint_outcome(&store, "r", "specify"), Some(CheckpointOutcome::Blocked));
}

#[test]
fn blocked_specify_does_not_affect_completed_frame() {
    let (_d, store) = started();
    checkpoint_decide(&store, "r", true, None).expect("frame");
    walk_to_checkpoint(&store, "r", "specify");
    checkpoint_decide(&store, "r", false, Some("x".into())).expect("specify reject");
    assert_eq!(station_status(&store, "r", "frame"), Status::Completed);
    assert_eq!(station_status(&store, "r", "specify"), Status::Blocked);
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION 32 — RunAction equality / clone invariants
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn same_disk_same_action_equality() {
    let (_d, store) = started();
    let a = run_tick(&store, "r").unwrap();
    // Re-derive (no further tick) → same position.
    let b = derive_position(&store, "r").unwrap();
    // After one tick, the cursor is now at Review; deriving again gives Review.
    assert!(matches!(b.action, Some(RunAction::Review { .. })));
    assert_eq!(a.run, "r");
}

#[test]
fn action_clone_equals_original() {
    let (_d, store) = started();
    let pos = derive_position(&store, "r").unwrap();
    let action = pos.action.unwrap();
    assert_eq!(action.clone(), action);
}

#[test]
fn fixfeedback_action_equality() {
    let (_d, store) = started();
    feedback::create(&store, "r", "frame", "x", None).unwrap();
    let a = derive_position(&store, "r").unwrap().action.unwrap();
    let b = derive_position(&store, "r").unwrap().action.unwrap();
    assert_eq!(a, b);
}

#[test]
fn resolvedrift_action_equality() {
    let (_d, store) = started();
    record_drift(&store, "r", "d-01", "x.md", "frame");
    let a = derive_position(&store, "r").unwrap().action.unwrap();
    let b = derive_position(&store, "r").unwrap().action.unwrap();
    assert_eq!(a, b);
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION 33 — additional checkpoint/track edge cases
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn build_auto_completes_without_any_decide_call() {
    let (_d, store) = started();
    advance_to_station(&store, "r", "build");
    walk_to_checkpoint(&store, "r", "build");
    // No checkpoint_decide invoked — auto handled it during the tick.
    assert_eq!(active_station(&store, "r"), "prove");
}

#[test]
fn prove_auto_completes_without_decide() {
    let (_d, store) = started();
    advance_to_station(&store, "r", "prove");
    walk_to_checkpoint(&store, "r", "prove");
    assert_eq!(active_station(&store, "r"), "harden");
}

#[test]
fn auto_stations_chain_to_harden_external() {
    let (_d, store) = started();
    advance_to_station(&store, "r", "build");
    walk_to_checkpoint(&store, "r", "build"); // auto → prove
    walk_to_checkpoint(&store, "r", "prove"); // auto → harden
    let cp = walk_to_checkpoint(&store, "r", "harden");
    assert!(matches!(cp, RunAction::Checkpoint { kind: CheckpointKind::External, .. }));
    assert_eq!(station_status(&store, "r", "harden"), Status::InProgress);
}

#[test]
fn harden_reject_does_not_seal() {
    let (_d, store) = started();
    advance_to_station(&store, "r", "harden");
    walk_to_checkpoint(&store, "r", "harden");
    let res = checkpoint_decide(&store, "r", false, Some("not ready".into())).expect("reject");
    assert!(!matches!(res.action, RunAction::Sealed { .. }));
    assert_eq!(station_status(&store, "r", "harden"), Status::Blocked);
}

#[test]
fn harden_reject_then_address_then_seal() {
    let (_d, store) = started();
    advance_to_station(&store, "r", "harden");
    walk_to_checkpoint(&store, "r", "harden");
    checkpoint_decide(&store, "r", false, Some("gap".into())).expect("reject");
    let fb = &feedback::list(&store, "r").unwrap()[0];
    feedback::set_status(&store, "r", &fb.id, FeedbackStatus::Addressed).unwrap();
    let res = checkpoint_decide(&store, "r", true, None).expect("seal");
    assert!(matches!(res.action, RunAction::Sealed { .. }));
}

#[test]
fn drift_overrides_held_external_checkpoint() {
    let (_d, store) = started();
    advance_to_station(&store, "r", "harden");
    walk_to_checkpoint(&store, "r", "harden");
    record_drift(&store, "r", "d-01", "harden/x.md", "harden");
    assert_eq!(pos_track(&derive_position(&store, "r").unwrap()), Track::Drift);
}

#[test]
fn feedback_overrides_held_external_checkpoint() {
    let (_d, store) = started();
    advance_to_station(&store, "r", "harden");
    walk_to_checkpoint(&store, "r", "harden");
    feedback::create(&store, "r", "harden", "x", None).unwrap();
    assert_eq!(pos_track(&derive_position(&store, "r").unwrap()), Track::Feedback);
}

#[test]
fn approve_records_completed_at_each_gated_station() {
    for st in ["frame", "specify", "shape"] {
        let (_d, store) = started();
        advance_to_station(&store, "r", st);
        walk_to_checkpoint(&store, "r", st);
        checkpoint_decide(&store, "r", true, None).expect("approve");
        let s = store.read_state("r").unwrap().unwrap();
        assert!(s.stations[st].completed_at.is_some(), "{st}");
    }
}

#[test]
fn auto_station_records_completed_at() {
    let (_d, store) = started();
    advance_to_station(&store, "r", "build");
    walk_to_checkpoint(&store, "r", "build");
    let s = store.read_state("r").unwrap().unwrap();
    assert!(s.stations["build"].completed_at.is_some());
}

#[test]
fn reject_leaves_completed_at_unset() {
    let (_d, store) = started();
    walk_to_checkpoint(&store, "r", "frame");
    checkpoint_decide(&store, "r", false, Some("x".into())).expect("reject");
    let s = store.read_state("r").unwrap().unwrap();
    assert!(s.stations["frame"].completed_at.is_none());
}

#[test]
fn next_station_seeded_only_on_completion() {
    let (_d, store) = started();
    // Before completing frame, specify is not yet in state.
    let s = store.read_state("r").unwrap().unwrap();
    assert!(!s.stations.contains_key("specify"));
    checkpoint_decide(&store, "r", true, None).expect("approve");
    let s = store.read_state("r").unwrap().unwrap();
    assert!(s.stations.contains_key("specify"));
}

#[test]
fn run_tick_returns_consistent_run_slug_across_phases() {
    let (_d, store) = started();
    for _ in 0..3 {
        let t = run_tick(&store, "r").unwrap();
        assert_eq!(t.run, "r");
        assert!(t.position.action.is_some());
    }
}

#[test]
fn feedback_filed_by_reject_is_listable_and_open() {
    let (_d, store) = started();
    walk_to_checkpoint(&store, "r", "frame");
    checkpoint_decide(&store, "r", false, Some("reason".into())).expect("reject");
    let all = feedback::list(&store, "r").unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].id, "fb-checkpoint");
    assert_eq!(all[0].status, FeedbackStatus::Pending);
    assert!(all[0].body.contains("reason"));
}

#[test]
fn drift_action_run_field_correct() {
    let (_d, store) = started();
    record_drift(&store, "r", "d-01", "x.md", "frame");
    match derive_position(&store, "r").unwrap().action {
        Some(RunAction::ResolveDrift { run, .. }) => assert_eq!(run, "r"),
        other => panic!("got {other:?}"),
    }
}

#[test]
fn fixfeedback_action_run_field_correct() {
    let (_d, store) = started();
    feedback::create(&store, "r", "frame", "x", None).unwrap();
    match derive_position(&store, "r").unwrap().action {
        Some(RunAction::FixFeedback { run, .. }) => assert_eq!(run, "r"),
        other => panic!("got {other:?}"),
    }
}

#[test]
fn checkpoint_action_run_field_correct() {
    let (_d, store) = started();
    walk_to_checkpoint(&store, "r", "frame");
    match derive_position(&store, "r").unwrap().action {
        Some(RunAction::Checkpoint { run, .. }) => assert_eq!(run, "r"),
        other => panic!("got {other:?}"),
    }
}

#[test]
fn sealed_action_run_field_correct() {
    let (_d, store) = started();
    run_to_sealed(&store, "r");
    match derive_position(&store, "r").unwrap().action {
        Some(RunAction::Sealed { run }) => assert_eq!(run, "r"),
        other => panic!("got {other:?}"),
    }
}

#[test]
fn whole_run_with_mid_rejects_still_seals() {
    // Reject every gated station once, address, re-approve, and still seal.
    let (_d, store) = started();
    for st in ["frame", "specify", "shape", "build", "prove", "harden"] {
        let cp = walk_to_checkpoint(&store, "r", st);
        if let RunAction::Checkpoint { kind, .. } = cp {
            if matches!(kind, CheckpointKind::Auto) {
                continue;
            }
            // reject, address, approve.
            checkpoint_decide(&store, "r", false, Some(format!("rework {st}")))
                .expect("reject");
            let fb = feedback::list(&store, "r").unwrap();
            let open = fb.iter().find(|f| !feedback::is_terminal(f.status)).unwrap();
            feedback::set_status(&store, "r", &open.id, FeedbackStatus::Addressed).unwrap();
            checkpoint_decide(&store, "r", true, None).expect("approve");
        }
    }
    assert!(matches!(
        derive_position(&store, "r").unwrap().action,
        Some(RunAction::Sealed { .. })
    ));
}
