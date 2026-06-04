//! End-to-end manager tests: the full station walk, every phase, every
//! checkpoint kind, the three-track priority, feedback preemption, and the
//! reject-routes-feedback path.
//!
//! These drive the public `darkrun-mcp` API (`run_start`, `run_tick`,
//! `checkpoint_decide`, `derive_position`) over a real on-disk `.darkrun/`
//! tree, so they exercise the same code path the MCP tools do.

use darkrun_core::domain::{
    CheckpointKind, Drift, DriftKind, FeedbackStatus, Status, StationPhase, Unit, UnitFrontmatter,
};
use darkrun_core::StateStore;
use darkrun_mcp::position::{checkpoint_decide, derive_position, run_start, run_tick, RunAction};
use darkrun_mcp::{drift, feedback, Track};
use tempfile::TempDir;

fn store() -> (TempDir, StateStore) {
    let dir = TempDir::new().expect("tmp");
    let store = StateStore::new(dir.path());
    (dir, store)
}

/// Decompose a single completed unit onto a station so the Manufacture phase
/// has work to dispatch and immediately clears.
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

/// Drive a station from wherever its cursor sits all the way to its open
/// Checkpoint, seeding a trivial completed unit so Manufacture clears. Tolerant
/// of the starting phase, because `checkpoint_decide` re-ticks once into the
/// next station before this helper runs. Returns the checkpoint action.
fn walk_to_checkpoint(store: &StateStore, run: &str, station: &str) -> RunAction {
    seed_completed_unit(store, run, station, &format!("{station}-u1"));
    for _ in 0..12 {
        let t = run_tick(store, run).expect("tick");
        match &t.action {
            // The pre-execution operator gate — approve it so the wave releases.
            RunAction::UserGate { station: s, .. } if s == station => {
                checkpoint_decide(store, run, true, None).expect("clear gate");
            }
            // The gate — a local Checkpoint or, for an external station, an
            // ExternalReviewRequested.
            RunAction::Checkpoint { station: s, .. }
            | RunAction::ExternalReviewRequested { station: s, .. }
                if s == station =>
            {
                return t.action
            }
            RunAction::Spec { station: s, .. }
            | RunAction::Review { station: s, .. }
            | RunAction::Manufacture { station: s, .. }
            | RunAction::Audit { station: s, .. }
            | RunAction::Reflect { station: s, .. }
                if s == station => {}
            other => panic!("unexpected action while walking {station}: {other:?}"),
        }
    }
    panic!("station {station} never reached its checkpoint");
}

#[test]
fn full_run_walks_all_six_stations_to_sealed() {
    let (_d, store) = store();
    run_start(&store, "r", "software", Some("Ship".into()), "continuous").expect("start");

    // frame: ask → operator approves to advance.
    let cp = walk_to_checkpoint(&store, "r", "frame");
    assert!(matches!(cp, RunAction::Checkpoint { kind: CheckpointKind::Ask, .. }));
    checkpoint_decide(&store, "r", true, None).expect("approve frame");

    // specify: ask.
    let cp = walk_to_checkpoint(&store, "r", "specify");
    assert!(matches!(cp, RunAction::Checkpoint { kind: CheckpointKind::Ask, .. }));
    checkpoint_decide(&store, "r", true, None).expect("approve specify");

    // shape: ask.
    let cp = walk_to_checkpoint(&store, "r", "shape");
    assert!(matches!(cp, RunAction::Checkpoint { kind: CheckpointKind::Ask, .. }));
    checkpoint_decide(&store, "r", true, None).expect("approve shape");

    // build: ask → operator approves to advance.
    let cp = walk_to_checkpoint(&store, "r", "build");
    assert!(matches!(cp, RunAction::Checkpoint { kind: CheckpointKind::Ask, .. }));
    checkpoint_decide(&store, "r", true, None).expect("approve build");
    let s = store.read_state("r").unwrap().unwrap();
    assert_eq!(s.stations["build"].status, Status::Completed);
    assert_eq!(s.active_station, "prove");

    // prove: ask.
    let cp = walk_to_checkpoint(&store, "r", "prove");
    assert!(matches!(cp, RunAction::Checkpoint { kind: CheckpointKind::Ask, .. }));
    checkpoint_decide(&store, "r", true, None).expect("approve prove");
    assert_eq!(
        store.read_state("r").unwrap().unwrap().active_station,
        "harden"
    );

    // harden: ask → holds until decide.
    let cp = walk_to_checkpoint(&store, "r", "harden");
    assert!(matches!(cp, RunAction::Checkpoint { kind: CheckpointKind::Ask, .. }));
    let s = store.read_state("r").unwrap().unwrap();
    assert_eq!(s.stations["harden"].status, Status::InProgress);

    // Operator approves the final gate → the run enters the whole-run review.
    let decided = checkpoint_decide(&store, "r", true, None).expect("approve harden");
    let final_action = match decided.action {
        RunAction::Sealed { .. } => decided.action,
        RunAction::RunReview { reviewers, .. } => {
            for r in reviewers {
                darkrun_mcp::position::run_review_stamp(&store, "r", &r).expect("run review stamp");
            }
            derive_position(&store, "r").unwrap().action.unwrap()
        }
        other => panic!("expected RunReview or Sealed, got {other:?}"),
    };
    assert!(
        matches!(&final_action, RunAction::Sealed { run } if run == "r"),
        "expected Sealed, got {final_action:?}"
    );
}

#[test]
fn every_phase_appears_in_order_for_a_station() {
    let (_d, store) = store();
    run_start(&store, "r", "software", None, "continuous").expect("start");

    let phases: Vec<&str> = {
        let mut seen = Vec::new();
        // Spec
        let t = run_tick(&store, "r").unwrap();
        seen.push(action_name(&t.action));
        // Review
        let t = run_tick(&store, "r").unwrap();
        seen.push(action_name(&t.action));
        // Decompose then Manufacture
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
        // The pre-execution operator gate holds before manufacture.
        let t = run_tick(&store, "r").unwrap();
        seen.push(action_name(&t.action));
        // Operator clears the gate → manufacture releases.
        checkpoint_decide(&store, "r", true, None).expect("clear gate");
        let t = run_tick(&store, "r").unwrap();
        seen.push(action_name(&t.action));
        // complete the unit so the next ticks audit/test/checkpoint
        let mut done = store.read_unit("r", "u1").unwrap();
        done.frontmatter.status = Status::Completed;
        store.write_unit("r", &done).unwrap();
        let t = run_tick(&store, "r").unwrap(); // audit (folds in the old tests phase)
        seen.push(action_name(&t.action));
        let t = run_tick(&store, "r").unwrap(); // reflect
        seen.push(action_name(&t.action));
        let t = run_tick(&store, "r").unwrap(); // checkpoint
        seen.push(action_name(&t.action));
        seen
    };

    assert_eq!(
        phases,
        vec!["spec", "review", "user_gate", "manufacture", "audit", "reflect", "checkpoint"]
    );
}

fn action_name(a: &RunAction) -> &'static str {
    // Delegate to the crate's single source of truth for action → tag.
    darkrun_mcp::position::action_tag(a)
}

#[test]
fn manufacture_holds_mid_wave_when_units_in_flight() {
    let (_d, store) = store();
    run_start(&store, "r", "software", None, "continuous").expect("start");
    run_tick(&store, "r").unwrap(); // spec → review
    run_tick(&store, "r").unwrap(); // review → user_gate

    // A dispatched unit still in flight (InProgress): no wave-ready Pending
    // unit, and not all complete → mid-wave noop. (A pending unit with a
    // dangling dep would instead be a UnitsInvalid decomposition error.)
    let in_flight = Unit {
        slug: "u1".into(),
        frontmatter: UnitFrontmatter {
            status: Status::InProgress,
            station: Some("frame".into()),
            ..Default::default()
        },
        title: "u1".into(),
        body: String::new(),
    };
    store.write_unit("r", &in_flight).unwrap();
    // Clear the pre-execution operator gate → the station enters Manufacture.
    checkpoint_decide(&store, "r", true, None).expect("clear gate");

    let pos = derive_position(&store, "r").unwrap();
    assert_eq!(pos.track, Track::Run);
    assert!(pos.action.is_none(), "expected mid-wave noop, got {:?}", pos.action);

    // run_tick surfaces a Noop action for the null position.
    let t = run_tick(&store, "r").unwrap();
    assert!(matches!(t.action, RunAction::Noop { .. }));
}

#[test]
fn three_track_priority_drift_beats_feedback_beats_run() {
    let (_d, store) = store();
    run_start(&store, "r", "software", None, "continuous").expect("start");

    // Run-only: the bare position is a run action (spec).
    let pos = derive_position(&store, "r").unwrap();
    assert_eq!(pos.track, Track::Run);

    // Add open feedback → feedback preempts run.
    feedback::create(&store, "r", "frame", "broken", None).unwrap();
    let pos = derive_position(&store, "r").unwrap();
    assert_eq!(pos.track, Track::Feedback);
    assert!(matches!(pos.action, Some(RunAction::FixFeedback { .. })));

    // Add a drift entry → drift preempts feedback (and run).
    let d = Drift {
        path: "frame/frame.md".into(),
        station: "frame".into(),
        run: "r".into(),
        kind: DriftKind::Spec,
        age: "1m".into(),
        unit: None,
    };
    drift::record(&store, "r", "d-01", &d).unwrap();
    let pos = derive_position(&store, "r").unwrap();
    assert_eq!(pos.track, Track::Drift);
    assert!(
        matches!(&pos.action, Some(RunAction::ResolveDrift { path, .. }) if path == "frame/frame.md")
    );
}

#[test]
fn feedback_preemption_pauses_then_resumes_run() {
    let (_d, store) = store();
    run_start(&store, "r", "software", None, "continuous").expect("start");

    // Mid-walk: advance into the frame station.
    run_tick(&store, "r").unwrap(); // spec → review

    // File feedback → next position is the feedback track.
    let fb = feedback::create(&store, "r", "frame", "fix this", None).unwrap();
    let pos = derive_position(&store, "r").unwrap();
    assert_eq!(pos.track, Track::Feedback);

    // Resolve the feedback terminally → run track resumes where it left off.
    feedback::set_status(&store, "r", &fb.id, FeedbackStatus::Addressed).unwrap();
    let pos = derive_position(&store, "r").unwrap();
    assert_eq!(pos.track, Track::Run);
    // Still on frame, now in the Review phase (the phase advanced on the
    // earlier spec tick) — feedback paused the walk without losing position.
    let state = store.read_state("r").unwrap().unwrap();
    assert_eq!(state.stations["frame"].phase, StationPhase::Review);
    assert!(matches!(pos.action, Some(RunAction::Review { .. })));
}

#[test]
fn checkpoint_reject_routes_rework_as_feedback() {
    let (_d, store) = store();
    run_start(&store, "r", "software", None, "continuous").expect("start");

    let res = checkpoint_decide(&store, "r", false, Some("not good enough".into()))
        .expect("reject");

    // The reject filed feedback, which now preempts the run track.
    assert_eq!(res.position.track, Track::Feedback);
    assert!(matches!(res.action, RunAction::FixFeedback { .. }));

    // The station is held (blocked) until the rework is addressed.
    let state = store.read_state("r").unwrap().unwrap();
    assert_eq!(state.stations["frame"].status, Status::Blocked);

    // The rework feedback is the one the manager dispatches.
    let all = feedback::list(&store, "r").unwrap();
    assert_eq!(all.len(), 1);
    assert!(all[0].body.contains("not good enough"));
}

#[test]
fn checkpoint_approve_completes_and_advances() {
    let (_d, store) = store();
    run_start(&store, "r", "software", None, "continuous").expect("start");
    let decided = checkpoint_decide(&store, "r", true, None).expect("approve");
    // frame completes; cursor advances to specify's Spec.
    assert!(matches!(&decided.action, RunAction::Spec { station, .. } if station == "specify"));
    let s = store.read_state("r").unwrap().unwrap();
    assert_eq!(s.stations["frame"].status, Status::Completed);
    assert_eq!(s.active_station, "specify");
}

#[test]
fn drift_track_station_defaults_to_current_when_unspecified() {
    let (_d, store) = store();
    run_start(&store, "r", "software", None, "continuous").expect("start");
    // A drift entry with no station should attribute to the active station.
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
    assert_eq!(pos.track, Track::Drift);
    assert!(
        matches!(&pos.action, Some(RunAction::ResolveDrift { station, .. }) if station == "frame")
    );
}

#[test]
fn sealed_when_all_stations_complete() {
    let (_d, store) = store();
    run_start(&store, "r", "software", None, "continuous").expect("start");
    // Walk every station to its checkpoint; gated stations (ask/external) need
    // an explicit approval, auto stations advance during the walk.
    for station in ["frame", "specify", "shape", "build", "prove", "harden"] {
        let cp = walk_to_checkpoint(&store, "r", station);
        let gated = matches!(
            cp,
            RunAction::Checkpoint {
                kind: CheckpointKind::Ask | CheckpointKind::External | CheckpointKind::Await,
                ..
            }
        ) || matches!(cp, RunAction::ExternalReviewRequested { .. });
        if gated {
            checkpoint_decide(&store, "r", true, None).expect("approve");
        }
    }
    // The whole-run review gates before seal; sign the run reviewers off.
    if let Some(RunAction::RunReview { reviewers, .. }) = derive_position(&store, "r").unwrap().action {
        for r in reviewers {
            darkrun_mcp::position::run_review_stamp(&store, "r", &r).expect("run review stamp");
        }
    }
    let pos = derive_position(&store, "r").unwrap();
    assert!(matches!(pos.action, Some(RunAction::Sealed { .. })));
}
