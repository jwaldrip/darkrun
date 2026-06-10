//! End-to-end lifecycle tests for the darkrun software factory.
//!
//! These drive a FULL Run through `darkrun-mcp` + `darkrun-core` (+ the
//! `darkrun-content` embedded corpus and the `darkrun-api` wire contract) as a
//! single system: `run_start` -> repeated `run_tick` -> `checkpoint_decide`,
//! walking every station (frame…harden) through every phase
//! (spec->review->manufacture->audit->tests->checkpoint) until the Run is
//! Sealed. Nothing is mocked — every assertion is against real on-disk
//! `.darkrun/` state produced by the manager.

mod common;

use common::*;
use darkrun_core::domain::{
    CheckpointKind, CheckpointOutcome, Status, StationPhase, Unit, UnitFrontmatter,
};
use darkrun_mcp::position::{derive_position, run_tick, RunAction, Track};

// ===========================================================================
// Section 1 — run_start: seeding a fresh Run
// ===========================================================================

#[test]
fn start_seeds_run_md_on_disk() {
    let h = Harness::start("s1");
    assert!(h.store.run_dir("s1").join("run.md").exists());
}

#[test]
fn start_seeds_state_json_on_disk() {
    let h = Harness::start("s1");
    assert!(h.store.run_dir("s1").join("state.json").exists());
}

#[test]
fn start_active_station_is_frame() {
    let h = Harness::start("s1");
    assert_eq!(h.active(), "frame");
}

#[test]
fn start_frame_phase_is_spec() {
    let h = Harness::start("s1");
    assert_eq!(h.phase("frame"), StationPhase::Spec);
}

#[test]
fn start_run_status_is_active() {
    let h = Harness::start("s1");
    let run = h.store.read_run("s1").unwrap();
    assert_eq!(run.frontmatter.status, Status::Active);
}

#[test]
fn start_factory_recorded_in_frontmatter() {
    let h = Harness::start("s1");
    let run = h.store.read_run("s1").unwrap();
    assert_eq!(run.frontmatter.factory, "software");
}

#[test]
fn start_factory_recorded_in_state() {
    let h = Harness::start("s1");
    assert_eq!(h.state().factory, "software");
}

#[test]
fn start_records_started_at_timestamp() {
    let h = Harness::start("s1");
    let run = h.store.read_run("s1").unwrap();
    assert!(run.frontmatter.started_at.is_some());
}

#[test]
fn start_with_title_sets_title() {
    let h = Harness::start_with("s1", "software", Some("Ship the slice"), "continuous");
    let run = h.store.read_run("s1").unwrap();
    assert_eq!(run.title, "Ship the slice");
}

#[test]
fn start_without_title_defaults_to_slug() {
    let h = Harness::start("my-slug");
    let run = h.store.read_run("my-slug").unwrap();
    assert_eq!(run.title, "my-slug");
}

#[test]
fn start_records_mode() {
    let h = Harness::start_with("s1", "software", None, "team");
    let run = h.store.read_run("s1").unwrap();
    assert_eq!(run.frontmatter.mode, darkrun_core::domain::Mode::Team);
}

#[test]
fn start_seeds_only_frame_station_entry() {
    let h = Harness::start("s1");
    let state = h.state();
    assert!(state.stations.contains_key("frame"));
    // Later stations are seeded lazily as the cursor reaches them.
    assert!(!state.stations.contains_key("specify"));
}

#[test]
fn start_seeds_frame_checkpoint_kind() {
    let h = Harness::start("s1");
    let state = h.state();
    let cp = state.stations["frame"].checkpoint.as_ref().unwrap();
    assert_eq!(cp.kind, CheckpointKind::Ask);
}

#[test]
fn start_frame_status_pending() {
    let h = Harness::start("s1");
    assert_eq!(h.station_status("frame"), Status::Pending);
}

#[test]
fn start_unknown_factory_errors() {
    let dir = tempfile::tempdir().unwrap();
    let store = darkrun_core::StateStore::new(dir.path());
    let err = darkrun_mcp::position::run_start(&store, "x", "nope", None, darkrun_core::domain::Mode::Solo, "full");
    assert!(err.is_err());
}

// ===========================================================================
// Section 2 — The first station (frame) phase machine, tick by tick
// ===========================================================================

#[test]
fn frame_tick1_emits_spec() {
    let h = Harness::start("f");
    assert!(is_spec(&h.tick().action, "frame"));
}

#[test]
fn frame_tick1_advances_phase_to_review() {
    let h = Harness::start("f");
    h.tick();
    assert_eq!(h.phase("frame"), StationPhase::Review);
}

#[test]
fn frame_tick1_marks_in_progress() {
    let h = Harness::start("f");
    h.tick();
    assert_eq!(h.station_status("frame"), Status::InProgress);
}

#[test]
fn frame_tick1_stamps_started_at() {
    let h = Harness::start("f");
    h.tick();
    assert!(h.state().stations["frame"].started_at.is_some());
}

#[test]
fn frame_tick2_emits_review() {
    let h = Harness::start("f");
    h.tick();
    assert!(is_review(&h.tick().action, "frame"));
}

#[test]
fn frame_tick2_advances_phase_to_user_gate() {
    let h = Harness::start("f");
    h.tick();
    h.tick();
    // frame is interactive (ask): after Review the cursor holds at the
    // pre-execution operator gate before manufacture.
    assert_eq!(h.phase("frame"), StationPhase::UserGate);
}

#[test]
fn frame_review_carries_reviewers() {
    let h = Harness::start("f");
    h.tick();
    let t = h.tick();
    match t.action {
        RunAction::Review { reviewers, .. } => {
            assert_eq!(reviewers, vec!["value", "feasibility"]);
        }
        other => panic!("expected Review, got {other:?}"),
    }
}

#[test]
fn frame_manufacture_without_units_falls_back_to_spec() {
    let h = Harness::start("f");
    h.tick(); // spec
    h.tick(); // review -> user gate
    // Clear the operator gate with no units decomposed; Manufacture has no wave,
    // so the derived action falls back to Spec.
    let t = h.decide(true, None);
    assert!(is_spec(&t.action, "frame"));
}

#[test]
fn frame_manufacture_dispatches_decomposed_unit() {
    let h = Harness::start("f");
    h.tick();
    h.tick();
    h.decompose("frame", &[("u1", &[])]);
    let t = h.tick();
    match t.action {
        RunAction::Manufacture { units, .. } => assert_eq!(units, vec!["u1".to_string()]),
        other => panic!("expected Manufacture, got {other:?}"),
    }
}

#[test]
fn frame_manufacture_carries_worker_beat() {
    let h = Harness::start("f");
    h.tick();
    h.tick();
    h.decompose("frame", &[("u1", &[])]);
    let t = h.tick();
    match t.action {
        RunAction::Manufacture { worker, .. } => assert_eq!(worker, "framer"),
        other => panic!("expected Manufacture, got {other:?}"),
    }
}

#[test]
fn frame_manufacture_stays_in_manufacture_until_locked() {
    let h = Harness::start("f");
    h.tick();
    h.tick();
    h.decompose("frame", &[("u1", &[])]);
    h.tick(); // dispatch wave
    assert_eq!(h.phase("frame"), StationPhase::Manufacture);
}

#[test]
fn frame_midwave_noop_when_units_in_flight() {
    let h = Harness::start("f");
    h.tick();
    h.tick();
    h.decompose("frame", &[("u1", &[])]);
    h.tick(); // dispatch; unit still pending (not completed) but no wave-ready left? It IS ready until completed.
              // Mark it active (in flight, not completed, not pending) → no wave-ready, not all complete → noop.
    let mut u = h.store.read_unit("f", "u1").unwrap();
    u.frontmatter.status = Status::Active;
    h.store.write_unit("f", &u).unwrap();
    let pos = derive_position(&h.store, "f").unwrap();
    assert!(pos.action.is_none(), "expected mid-wave noop, got {:?}", pos.action);
}

#[test]
fn frame_midwave_noop_tick_yields_noop_action() {
    let h = Harness::start("f");
    h.tick();
    h.tick();
    h.decompose("frame", &[("u1", &[])]);
    h.tick();
    let mut u = h.store.read_unit("f", "u1").unwrap();
    u.frontmatter.status = Status::Active;
    h.store.write_unit("f", &u).unwrap();
    let t = h.tick();
    assert!(matches!(t.action, RunAction::Noop { .. }));
}

#[test]
fn frame_audit_after_units_complete() {
    let h = Harness::start("f");
    h.tick();
    h.tick();
    h.decompose("frame", &[("u1", &[])]);
    h.tick();
    h.complete_unit("u1");
    assert!(is_audit(&h.tick().action, "frame"));
}

#[test]
fn frame_audit_advances_phase_to_reflect() {
    let h = Harness::start("f");
    h.tick();
    h.tick();
    h.decompose("frame", &[("u1", &[])]);
    h.tick();
    h.complete_unit("u1");
    h.tick(); // audit
    assert_eq!(h.phase("frame"), StationPhase::Reflect);
}

#[test]
fn frame_reflect_action_after_audit() {
    let h = Harness::start("f");
    h.tick();
    h.tick();
    h.decompose("frame", &[("u1", &[])]);
    h.tick();
    h.complete_unit("u1");
    h.tick(); // audit
    assert!(is_reflect(&h.tick().action, "frame"));
}

#[test]
fn frame_reflect_advances_phase_to_checkpoint() {
    let h = Harness::start("f");
    h.tick();
    h.tick();
    h.decompose("frame", &[("u1", &[])]);
    h.tick();
    h.complete_unit("u1");
    h.tick(); // audit
    h.tick(); // reflect
    assert_eq!(h.phase("frame"), StationPhase::Checkpoint);
}

#[test]
fn frame_checkpoint_action_holds_for_ask_gate() {
    let h = Harness::start("f");
    let actions = h.walk_station_to_checkpoint("frame", &["u1"]);
    assert!(is_checkpoint(actions.last().unwrap(), "frame"));
}

#[test]
fn frame_checkpoint_kind_is_ask() {
    let h = Harness::start("f");
    let actions = h.walk_station_to_checkpoint("frame", &["u1"]);
    match actions.last().unwrap() {
        RunAction::Checkpoint { kind, .. } => assert_eq!(*kind, CheckpointKind::Ask),
        other => panic!("expected Checkpoint, got {other:?}"),
    }
}

#[test]
fn frame_ask_gate_does_not_auto_complete() {
    let h = Harness::start("f");
    h.walk_station_to_checkpoint("frame", &["u1"]);
    // Held; still in progress until decided.
    assert_eq!(h.station_status("frame"), Status::InProgress);
}

#[test]
fn frame_checkpoint_records_entered_at() {
    let h = Harness::start("f");
    h.walk_station_to_checkpoint("frame", &["u1"]);
    let cp = h.state().stations["frame"].checkpoint.clone().unwrap();
    assert!(cp.entered_at.is_some());
}

#[test]
fn frame_approve_completes_station() {
    let h = Harness::start("f");
    h.walk_station_to_checkpoint("frame", &["u1"]);
    h.decide(true, None);
    assert_eq!(h.station_status("frame"), Status::Completed);
}

#[test]
fn frame_approve_advances_active_to_specify() {
    let h = Harness::start("f");
    h.walk_station_to_checkpoint("frame", &["u1"]);
    h.decide(true, None);
    assert_eq!(h.active(), "specify");
}

#[test]
fn frame_approve_emits_specify_spec() {
    let h = Harness::start("f");
    h.walk_station_to_checkpoint("frame", &["u1"]);
    let decided = h.decide(true, None);
    assert!(is_spec(&decided.action, "specify"));
}

#[test]
fn frame_approve_stamps_completed_at() {
    let h = Harness::start("f");
    h.walk_station_to_checkpoint("frame", &["u1"]);
    h.decide(true, None);
    assert!(h.state().stations["frame"].completed_at.is_some());
}

#[test]
fn frame_approve_marks_checkpoint_outcome_advanced() {
    let h = Harness::start("f");
    h.walk_station_to_checkpoint("frame", &["u1"]);
    h.decide(true, None);
    let cp = h.state().stations["frame"].checkpoint.clone().unwrap();
    assert_eq!(cp.outcome, Some(CheckpointOutcome::Advanced));
}

// ===========================================================================
// Section 3 — Full lifecycle: walking every station to Sealed
// ===========================================================================

#[test]
fn full_run_reaches_sealed() {
    let h = Harness::start("full");
    h.run_to_seal();
    assert!(matches!(
        h.position().action,
        Some(RunAction::Sealed { .. })
    ));
}

#[test]
fn full_run_sealed_action_names_run() {
    let h = Harness::start("full");
    h.run_to_seal();
    match h.position().action {
        Some(RunAction::Sealed { run }) => assert_eq!(run, "full"),
        other => panic!("expected Sealed, got {other:?}"),
    }
}

#[test]
fn full_run_sealed_is_run_track() {
    let h = Harness::start("full");
    h.run_to_seal();
    assert_eq!(h.position().track, Track::Run);
}

#[test]
fn full_run_completes_every_station() {
    let h = Harness::start("full");
    h.run_to_seal();
    for s in STATIONS {
        assert_eq!(h.station_status(s), Status::Completed, "{s}");
    }
}

#[test]
fn full_run_active_pointer_ends_on_harden() {
    let h = Harness::start("full");
    // Walk all but seal-confirmation; active stays on last station once it
    // completes (no next station to advance to).
    h.run_to_seal();
    assert_eq!(h.active(), "harden");
}

#[test]
fn full_run_every_station_has_completed_at() {
    let h = Harness::start("full");
    h.run_to_seal();
    for s in STATIONS {
        assert!(
            h.state().stations[s].completed_at.is_some(),
            "{s} missing completed_at"
        );
    }
}

#[test]
fn full_run_every_station_has_started_at() {
    let h = Harness::start("full");
    h.run_to_seal();
    for s in STATIONS {
        assert!(h.state().stations[s].started_at.is_some(), "{s}");
    }
}

#[test]
fn full_run_action_log_starts_with_frame_spec() {
    let h = Harness::start("full");
    let log = h.run_to_seal();
    assert!(is_spec(&log[0], "frame"));
}

#[test]
fn full_run_action_log_has_six_specs() {
    let h = Harness::start("full");
    let log = h.run_to_seal();
    let specs = log
        .iter()
        .filter(|a| matches!(a, RunAction::Spec { .. }))
        .count();
    assert_eq!(specs, 6);
}

#[test]
fn full_run_action_log_has_six_reviews() {
    let h = Harness::start("full");
    let log = h.run_to_seal();
    let reviews = log
        .iter()
        .filter(|a| matches!(a, RunAction::Review { .. }))
        .count();
    assert_eq!(reviews, 6);
}

#[test]
fn full_run_action_log_has_six_audits() {
    let h = Harness::start("full");
    let log = h.run_to_seal();
    let audits = log
        .iter()
        .filter(|a| matches!(a, RunAction::Audit { .. }))
        .count();
    assert_eq!(audits, 6);
}

#[test]
fn full_run_action_log_has_six_reflects() {
    let h = Harness::start("full");
    let log = h.run_to_seal();
    let reflects = log
        .iter()
        .filter(|a| matches!(a, RunAction::Reflect { .. }))
        .count();
    assert_eq!(reflects, 6);
}

#[test]
fn full_run_action_log_has_six_checkpoints() {
    let h = Harness::start("full");
    let log = h.run_to_seal();
    // Every station gates locally with an `ask` Checkpoint — six gates total.
    let cps = log
        .iter()
        .filter(|a| matches!(a, RunAction::Checkpoint { .. }))
        .count();
    assert_eq!(cps, 6);
}

#[test]
fn full_run_action_log_has_six_manufactures() {
    let h = Harness::start("full");
    let log = h.run_to_seal();
    let m = log
        .iter()
        .filter(|a| matches!(a, RunAction::Manufacture { .. }))
        .count();
    assert_eq!(m, 6);
}

#[test]
fn full_run_spec_station_order_is_canonical() {
    let h = Harness::start("full");
    let log = h.run_to_seal();
    let spec_stations: Vec<String> = log
        .iter()
        .filter_map(|a| match a {
            RunAction::Spec { station, .. } => Some(station.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(spec_stations, STATIONS.to_vec());
}

#[test]
fn full_run_checkpoint_station_order_is_canonical() {
    let h = Harness::start("full");
    let log = h.run_to_seal();
    // Every station's gate, in order — a local Checkpoint for five, an external
    // review gate for harden.
    let cp_stations: Vec<String> = log
        .iter()
        .filter_map(|a| match a {
            RunAction::Checkpoint { station, .. }
            | RunAction::ExternalReviewRequested { station, .. } => Some(station.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(cp_stations, STATIONS.to_vec());
}

#[test]
fn full_run_checkpoint_kinds_in_order() {
    let h = Harness::start("full");
    let log = h.run_to_seal();
    // Every station gates `ask` by default.
    let kinds: Vec<CheckpointKind> = log
        .iter()
        .filter_map(|a| match a {
            RunAction::Checkpoint { kind, .. } => Some(*kind),
            _ => None,
        })
        .collect();
    assert_eq!(kinds, vec![CheckpointKind::Ask; 6]);
}

// Auto-gate stations advance themselves on the checkpoint tick — no decide.
#[test]
fn build_ask_gate_holds_until_decided() {
    let h = Harness::start("ag");
    h.complete_station("frame", &["a"]);
    h.complete_station("specify", &["b"]);
    h.complete_station("shape", &["c"]);
    assert_eq!(h.active(), "build");
    // build now gates `ask` — it HOLDS at the checkpoint until the operator
    // decides, rather than self-completing on the tick.
    h.walk_station_to_checkpoint("build", &["d"]);
    assert_eq!(h.station_status("build"), Status::InProgress);
    h.decide(true, None);
    assert_eq!(h.station_status("build"), Status::Completed);
}

#[test]
fn build_ask_gate_advances_active_to_prove_on_decide() {
    let h = Harness::start("ag");
    h.complete_station("frame", &["a"]);
    h.complete_station("specify", &["b"]);
    h.complete_station("shape", &["c"]);
    h.walk_station_to_checkpoint("build", &["d"]);
    h.decide(true, None);
    assert_eq!(h.active(), "prove");
}

#[test]
fn harden_external_gate_holds_until_decided() {
    let h = Harness::start("hd");
    for (i, s) in ["frame", "specify", "shape", "build", "prove"].iter().enumerate() {
        let u = format!("u{i}");
        h.complete_station(s, &[u.as_str()]);
    }
    assert_eq!(h.active(), "harden");
    h.walk_station_to_checkpoint("harden", &["z"]);
    // External gate does NOT auto-complete.
    assert_eq!(h.station_status("harden"), Status::InProgress);
}

#[test]
fn harden_external_decide_seals_run() {
    let h = Harness::start("hd");
    for (i, s) in ["frame", "specify", "shape", "build", "prove"].iter().enumerate() {
        let u = format!("u{i}");
        h.complete_station(s, &[u.as_str()]);
    }
    h.walk_station_to_checkpoint("harden", &["z"]);
    let decided = h.decide(true, None);
    // After the final station the run holds in the whole-run review; once the
    // run reviewers sign off, it seals.
    match decided.action {
        RunAction::Sealed { .. } => {}
        RunAction::RunReview { reviewers, .. } => {
            for r in reviewers {
                darkrun_mcp::position::run_review_stamp(&h.store, "hd", &r).expect("stamp");
            }
            assert!(matches!(
                h.position().action,
                Some(RunAction::Sealed { .. })
            ));
        }
        other => panic!("expected RunReview or Sealed, got {other:?}"),
    }
}

// ===========================================================================
// Section 4 — derive_position determinism (pure read)
// ===========================================================================

#[test]
fn derive_position_is_deterministic_at_start() {
    let h = Harness::start("det");
    let p1 = derive_position(&h.store, "det").unwrap();
    let p2 = derive_position(&h.store, "det").unwrap();
    assert_eq!(p1, p2);
}

#[test]
fn derive_position_does_not_mutate_state() {
    let h = Harness::start("det");
    let before = h.state().active_station;
    derive_position(&h.store, "det").unwrap();
    derive_position(&h.store, "det").unwrap();
    assert_eq!(h.state().active_station, before);
}

#[test]
fn derive_position_does_not_advance_phase() {
    let h = Harness::start("det");
    derive_position(&h.store, "det").unwrap();
    assert_eq!(h.phase("frame"), StationPhase::Spec);
}

#[test]
fn derive_position_stable_across_many_reads() {
    let h = Harness::start("det");
    h.tick();
    h.tick();
    h.decompose("frame", &[("u1", &[])]);
    let first = derive_position(&h.store, "det").unwrap();
    for _ in 0..50 {
        assert_eq!(derive_position(&h.store, "det").unwrap(), first);
    }
}

#[test]
fn derive_position_stable_at_checkpoint() {
    let h = Harness::start("det");
    h.walk_station_to_checkpoint("frame", &["u1"]);
    let a = derive_position(&h.store, "det").unwrap();
    let b = derive_position(&h.store, "det").unwrap();
    assert_eq!(a, b);
    assert!(is_checkpoint(a.action.as_ref().unwrap(), "frame"));
}

#[test]
fn derive_position_stable_when_sealed() {
    let h = Harness::start("det");
    h.run_to_seal();
    let a = derive_position(&h.store, "det").unwrap();
    let b = derive_position(&h.store, "det").unwrap();
    assert_eq!(a, b);
    assert!(matches!(a.action, Some(RunAction::Sealed { .. })));
}

#[test]
fn derive_position_track_is_run_at_start() {
    let h = Harness::start("det");
    assert_eq!(derive_position(&h.store, "det").unwrap().track, Track::Run);
}

#[test]
fn derive_position_missing_run_errors() {
    let dir = tempfile::tempdir().unwrap();
    let store = darkrun_core::StateStore::new(dir.path());
    assert!(derive_position(&store, "ghost").is_err());
}

// ===========================================================================
// Section 5 — Multi-unit waves with dependencies (the unit DAG)
// ===========================================================================

#[test]
fn wave_dispatches_only_ready_units() {
    let h = Harness::start("wave");
    h.tick(); // spec
    h.tick(); // review
              // u2 depends on u1; only u1 is wave-ready.
    h.decompose("frame", &[("u1", &[]), ("u2", &["u1"])]);
    let t = h.tick();
    match t.action {
        RunAction::Manufacture { units, .. } => assert_eq!(units, vec!["u1".to_string()]),
        other => panic!("expected Manufacture, got {other:?}"),
    }
}

#[test]
fn wave_unblocks_dependent_after_dep_completes() {
    let h = Harness::start("wave");
    h.tick();
    h.tick();
    h.decompose("frame", &[("u1", &[]), ("u2", &["u1"])]);
    h.tick(); // wave 1: u1
    h.complete_unit("u1");
    let t = h.tick(); // wave 2: u2 now ready
    match t.action {
        RunAction::Manufacture { units, .. } => assert_eq!(units, vec!["u2".to_string()]),
        other => panic!("expected Manufacture, got {other:?}"),
    }
}

#[test]
fn wave_two_independent_units_dispatch_together() {
    let h = Harness::start("wave");
    h.tick();
    h.tick();
    h.decompose("frame", &[("u1", &[]), ("u2", &[])]);
    let t = h.tick();
    match t.action {
        RunAction::Manufacture { mut units, .. } => {
            units.sort();
            assert_eq!(units, vec!["u1".to_string(), "u2".to_string()]);
        }
        other => panic!("expected Manufacture, got {other:?}"),
    }
}

#[test]
fn wave_diamond_dag_resolves_in_three_waves() {
    // a -> {b, c} -> d.
    let h = Harness::start("diamond");
    h.tick();
    h.tick();
    h.decompose(
        "frame",
        &[("a", &[]), ("b", &["a"]), ("c", &["a"]), ("d", &["b", "c"])],
    );
    // Wave 1: a only.
    let w1 = h.tick();
    assert!(matches!(w1.action, RunAction::Manufacture { ref units, .. } if units == &vec!["a".to_string()]));
    h.complete_unit("a");
    // Wave 2: b and c.
    let w2 = h.tick();
    match w2.action {
        RunAction::Manufacture { mut units, .. } => {
            units.sort();
            assert_eq!(units, vec!["b".to_string(), "c".to_string()]);
        }
        other => panic!("expected Manufacture, got {other:?}"),
    }
    h.complete_units(&["b", "c"]);
    // Wave 3: d.
    let w3 = h.tick();
    assert!(matches!(w3.action, RunAction::Manufacture { ref units, .. } if units == &vec!["d".to_string()]));
    h.complete_unit("d");
    // All locked → Audit.
    assert!(is_audit(&h.tick().action, "frame"));
}

#[test]
fn wave_chain_resolves_one_unit_per_wave() {
    // a -> b -> c -> d.
    let h = Harness::start("chain");
    h.tick();
    h.tick();
    h.decompose(
        "frame",
        &[("a", &[]), ("b", &["a"]), ("c", &["b"]), ("d", &["c"])],
    );
    for step in ["a", "b", "c", "d"] {
        let t = h.tick();
        match t.action {
            RunAction::Manufacture { units, .. } => assert_eq!(units, vec![step.to_string()]),
            other => panic!("expected Manufacture of {step}, got {other:?}"),
        }
        h.complete_unit(step);
    }
    assert!(is_audit(&h.tick().action, "frame"));
}

#[test]
fn wave_unrelated_station_units_excluded() {
    let h = Harness::start("scoped");
    h.tick();
    h.tick();
    // A unit on a *different* station must not enter frame's wave.
    h.decompose("frame", &[("fu", &[])]);
    h.decompose("specify", &[("su", &[])]);
    let t = h.tick();
    match t.action {
        RunAction::Manufacture { units, .. } => assert_eq!(units, vec!["fu".to_string()]),
        other => panic!("expected only frame unit, got {other:?}"),
    }
}

#[test]
fn wave_blocked_unit_keeps_station_in_manufacture() {
    let h = Harness::start("blk");
    h.tick();
    h.tick();
    // u2 depends on u1 (a real edge — an unresolved/dangling dep would be a
    // UnitsInvalid decomposition error, not a legitimate block).
    h.decompose("frame", &[("u1", &[]), ("u2", &["u1"])]);
    h.tick(); // dispatch the ready wave (u1)
    // u1 is dispatched and in-flight (not yet locked); u2 is blocked behind it.
    h.set_unit_status("u1", Status::InProgress);
    // No wave-ready Pending unit (u1 is in-flight, u2 blocked on incomplete u1)
    // and not all complete → mid-wave noop.
    let pos = derive_position(&h.store, "blk").unwrap();
    assert!(pos.action.is_none());
}

#[test]
fn wave_many_independent_units_all_dispatch() {
    let h = Harness::start("fan");
    h.tick();
    h.tick();
    let units: Vec<(&str, &[&str])> = vec![
        ("u1", &[]),
        ("u2", &[]),
        ("u3", &[]),
        ("u4", &[]),
        ("u5", &[]),
    ];
    h.decompose("frame", &units);
    let t = h.tick();
    match t.action {
        RunAction::Manufacture { units, .. } => assert_eq!(units.len(), 5),
        other => panic!("expected 5-unit wave, got {other:?}"),
    }
}

#[test]
fn wave_partial_completion_redispatches_remaining() {
    let h = Harness::start("partial");
    h.tick();
    h.tick();
    h.decompose("frame", &[("u1", &[]), ("u2", &[]), ("u3", &[])]);
    h.tick(); // wave with all 3
    h.complete_unit("u1");
    let t = h.tick();
    match t.action {
        RunAction::Manufacture { mut units, .. } => {
            units.sort();
            assert_eq!(units, vec!["u2".to_string(), "u3".to_string()]);
        }
        other => panic!("expected remaining 2, got {other:?}"),
    }
}

// ===========================================================================
// Section 6 — Feedback preempts the run track (Track B)
// ===========================================================================

#[test]
fn feedback_preempts_at_start() {
    let h = Harness::start("fb");
    h.file_feedback("fb-1", "pending", "broken");
    assert_eq!(h.position().track, Track::Feedback);
}

#[test]
fn feedback_emits_fix_action() {
    let h = Harness::start("fb");
    h.file_feedback("fb-1", "pending", "broken");
    match h.position().action {
        Some(RunAction::FixFeedback { feedback_id, .. }) => assert_eq!(feedback_id, "fb-1"),
        other => panic!("expected FixFeedback, got {other:?}"),
    }
}

#[test]
fn feedback_fix_targets_active_station() {
    let h = Harness::start("fb");
    h.file_feedback("fb-1", "pending", "broken");
    match h.position().action {
        Some(RunAction::FixFeedback { station, .. }) => assert_eq!(station, "frame"),
        other => panic!("expected FixFeedback, got {other:?}"),
    }
}

#[test]
fn feedback_preempts_mid_station() {
    let h = Harness::start("fb");
    h.tick(); // spec
    h.tick(); // review
    h.file_feedback("fb-9", "pending", "found a problem mid-run");
    let pos = h.position();
    assert_eq!(pos.track, Track::Feedback);
}

#[test]
fn feedback_resolved_returns_control_to_run() {
    let h = Harness::start("fb");
    h.tick();
    h.tick();
    h.file_feedback("fb-9", "pending", "problem");
    assert_eq!(h.position().track, Track::Feedback);
    // Address it (terminal status) → run track resumes.
    h.file_feedback("fb-9", "addressed", "problem");
    assert_eq!(h.position().track, Track::Run);
}

#[test]
fn feedback_closed_is_not_open() {
    let h = Harness::start("fb");
    h.file_feedback("fb-1", "closed", "old");
    assert_eq!(h.position().track, Track::Run);
}

#[test]
fn feedback_rejected_is_not_open() {
    let h = Harness::start("fb");
    h.file_feedback("fb-1", "rejected", "invalid");
    assert_eq!(h.position().track, Track::Run);
}

#[test]
fn feedback_answered_is_not_open() {
    let h = Harness::start("fb");
    h.file_feedback("fb-1", "answered", "replied");
    assert_eq!(h.position().track, Track::Run);
}

#[test]
fn feedback_non_actionable_is_not_open() {
    let h = Harness::start("fb");
    h.file_feedback("fb-1", "non_actionable", "noted");
    assert_eq!(h.position().track, Track::Run);
}

#[test]
fn feedback_fixing_is_still_open() {
    let h = Harness::start("fb");
    h.file_feedback("fb-1", "fixing", "in flight");
    assert_eq!(h.position().track, Track::Feedback);
}

#[test]
fn feedback_no_status_line_treated_open() {
    let h = Harness::start("fb");
    h.store
        .write_feedback_raw("fb", "fb-1", "just a body, no frontmatter\n")
        .unwrap();
    assert_eq!(h.position().track, Track::Feedback);
}

#[test]
fn feedback_first_open_dispatched_when_multiple() {
    let h = Harness::start("fb");
    h.file_feedback("fb-01", "addressed", "done");
    h.file_feedback("fb-02", "pending", "open one");
    match h.position().action {
        Some(RunAction::FixFeedback { feedback_id, .. }) => assert_eq!(feedback_id, "fb-02"),
        other => panic!("expected fb-02, got {other:?}"),
    }
}

#[test]
fn feedback_typed_create_preempts_run() {
    let h = Harness::start("fb");
    darkrun_mcp::feedback::create(&h.store, "fb", "frame", "typed finding", None).unwrap();
    assert_eq!(h.position().track, Track::Feedback);
}

#[test]
fn feedback_typed_resolve_returns_to_run() {
    let h = Harness::start("fb");
    let made = darkrun_mcp::feedback::create(&h.store, "fb", "frame", "typed", None).unwrap();
    darkrun_mcp::feedback::set_status(
        &h.store,
        "fb",
        &made.id,
        darkrun_core::domain::FeedbackStatus::Addressed,
    )
    .unwrap();
    assert_eq!(h.position().track, Track::Run);
}

#[test]
fn feedback_does_not_advance_run_phase() {
    let h = Harness::start("fb");
    h.tick(); // spec -> review
    h.file_feedback("fb-1", "pending", "x");
    let phase_before = h.phase("frame");
    h.tick(); // feedback tick: no phase advance
    assert_eq!(h.phase("frame"), phase_before);
}

// ===========================================================================
// Section 7 — Rejected checkpoint routes rework as feedback
// ===========================================================================

#[test]
fn reject_holds_station_blocked() {
    let h = Harness::start("rej");
    h.walk_station_to_checkpoint("frame", &["u1"]);
    h.decide(false, Some("not good enough"));
    assert_eq!(h.station_status("frame"), Status::Blocked);
}

#[test]
fn reject_routes_to_feedback_track() {
    let h = Harness::start("rej");
    h.walk_station_to_checkpoint("frame", &["u1"]);
    let res = h.decide(false, Some("needs rework"));
    assert_eq!(res.position.track, Track::Feedback);
}

#[test]
fn reject_files_feedback_doc() {
    let h = Harness::start("rej");
    h.walk_station_to_checkpoint("frame", &["u1"]);
    h.decide(false, Some("rework this"));
    let raw = h.store.read_feedback_raw("rej").unwrap();
    assert!(raw.contains_key("fb-checkpoint"));
}

#[test]
fn reject_feedback_body_carries_reason() {
    let h = Harness::start("rej");
    h.walk_station_to_checkpoint("frame", &["u1"]);
    h.decide(false, Some("the spec is wrong"));
    let raw = h.store.read_feedback_raw("rej").unwrap();
    assert!(raw["fb-checkpoint"].contains("the spec is wrong"));
}

#[test]
fn reject_marks_checkpoint_outcome_blocked() {
    let h = Harness::start("rej");
    h.walk_station_to_checkpoint("frame", &["u1"]);
    h.decide(false, Some("no"));
    let cp = h.state().stations["frame"].checkpoint.clone().unwrap();
    assert_eq!(cp.outcome, Some(CheckpointOutcome::Blocked));
}

#[test]
fn reject_does_not_advance_active_station() {
    let h = Harness::start("rej");
    h.walk_station_to_checkpoint("frame", &["u1"]);
    h.decide(false, Some("no"));
    assert_eq!(h.active(), "frame");
}

#[test]
fn reject_then_resolve_feedback_resumes_run() {
    let h = Harness::start("rej");
    h.walk_station_to_checkpoint("frame", &["u1"]);
    h.decide(false, Some("fix it"));
    // The filed feedback preempts; resolve it to return to the run track.
    h.file_feedback("fb-checkpoint", "addressed", "fixed");
    assert_eq!(h.position().track, Track::Run);
}

#[test]
fn reject_without_feedback_still_blocks() {
    let h = Harness::start("rej");
    h.walk_station_to_checkpoint("frame", &["u1"]);
    h.decide(false, None);
    assert_eq!(h.station_status("frame"), Status::Blocked);
}

#[test]
fn reject_fix_action_targets_blocked_station() {
    let h = Harness::start("rej");
    h.walk_station_to_checkpoint("frame", &["u1"]);
    let res = h.decide(false, Some("rework"));
    match res.action {
        RunAction::FixFeedback { feedback_id, .. } => assert_eq!(feedback_id, "fb-checkpoint"),
        other => panic!("expected FixFeedback, got {other:?}"),
    }
}

// ===========================================================================
// Section 8 — run_show / run_list / archive cross-crate flows
// ===========================================================================

#[test]
fn run_list_returns_started_run() {
    let h = Harness::start("listed");
    let runs = darkrun_mcp::runs::list(&h.store, h.repo_root(), true).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].slug, "listed");
}

#[test]
fn run_list_summary_carries_factory() {
    let h = Harness::start("listed");
    let runs = darkrun_mcp::runs::list(&h.store, h.repo_root(), true).unwrap();
    assert_eq!(runs[0].factory, "software");
}

#[test]
fn run_list_summary_active_station_frame() {
    let h = Harness::start("listed");
    let runs = darkrun_mcp::runs::list(&h.store, h.repo_root(), true).unwrap();
    assert_eq!(runs[0].active_station, "frame");
}

#[test]
fn run_list_excludes_archived_by_default() {
    let h = Harness::start("listed");
    darkrun_mcp::runs::set_archived(&h.store, "listed", true).unwrap();
    assert!(darkrun_mcp::runs::list(&h.store, h.repo_root(), false)
        .unwrap()
        .is_empty());
}

#[test]
fn run_list_includes_archived_when_requested() {
    let h = Harness::start("listed");
    darkrun_mcp::runs::set_archived(&h.store, "listed", true).unwrap();
    assert_eq!(
        darkrun_mcp::runs::list(&h.store, h.repo_root(), true)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn run_list_multiple_runs_sorted_by_slug() {
    let dir = tempfile::tempdir().unwrap();
    let store = darkrun_core::StateStore::new(dir.path());
    darkrun_mcp::position::run_start(&store, "bravo", "software", None, darkrun_core::domain::Mode::Solo, "full").unwrap();
    darkrun_mcp::position::run_start(&store, "alpha", "software", None, darkrun_core::domain::Mode::Solo, "full").unwrap();
    let runs = darkrun_mcp::runs::list(&store, dir.path(), true).unwrap();
    assert_eq!(runs[0].slug, "alpha");
    assert_eq!(runs[1].slug, "bravo");
}

#[test]
fn run_show_reflects_active_station_after_advance() {
    let h = Harness::start("show");
    h.complete_station("frame", &["u1"]);
    // run_show is read_run; active_station write-cache updates via state, but
    // run.md frontmatter active_station is the start value. The derived state
    // is authoritative; assert the state pointer moved.
    assert_eq!(h.active(), "specify");
}

#[test]
fn archive_clears_active_pointer() {
    let h = Harness::start("ap");
    h.store.set_active_run("ap").unwrap();
    darkrun_mcp::runs::set_archived(&h.store, "ap", true).unwrap();
    assert_ne!(h.store.active_run().unwrap(), Some("ap".to_string()));
}

#[test]
fn unarchive_restores_to_default_list() {
    let h = Harness::start("ua");
    darkrun_mcp::runs::set_archived(&h.store, "ua", true).unwrap();
    darkrun_mcp::runs::set_archived(&h.store, "ua", false).unwrap();
    assert_eq!(
        darkrun_mcp::runs::list(&h.store, h.repo_root(), false)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn active_run_resolves_started_run() {
    let h = Harness::start("only");
    assert_eq!(h.store.active_run().unwrap(), Some("only".to_string()));
}

// ===========================================================================
// Section 9 — RunAction serde wire shape (cross-crate JSON contract)
// ===========================================================================

#[test]
fn spec_action_serializes_with_tag() {
    let h = Harness::start("ser");
    let t = h.tick();
    assert_eq!(action_tag(&t.action), "spec");
}

#[test]
fn review_action_serializes_with_tag() {
    let h = Harness::start("ser");
    h.tick();
    let t = h.tick();
    assert_eq!(action_tag(&t.action), "review");
}

#[test]
fn manufacture_action_serializes_with_tag() {
    let h = Harness::start("ser");
    h.tick();
    h.tick();
    h.decompose("frame", &[("u1", &[])]);
    let t = h.tick();
    assert_eq!(action_tag(&t.action), "manufacture");
}

#[test]
fn checkpoint_action_serializes_with_tag() {
    let h = Harness::start("ser");
    let actions = h.walk_station_to_checkpoint("frame", &["u1"]);
    assert_eq!(action_tag(actions.last().unwrap()), "checkpoint");
}

#[test]
fn sealed_action_serializes_with_tag() {
    let h = Harness::start("ser");
    h.run_to_seal();
    let pos = h.position();
    assert_eq!(action_tag(&pos.action.unwrap()), "sealed");
}

#[test]
fn fix_feedback_action_serializes_with_tag() {
    let h = Harness::start("ser");
    h.file_feedback("fb-1", "pending", "x");
    let pos = h.position();
    assert_eq!(action_tag(&pos.action.unwrap()), "fix_feedback");
}

#[test]
fn spec_action_carries_kills_field() {
    let h = Harness::start("ser");
    let t = h.tick();
    let v = serde_json::to_value(&t.action).unwrap();
    assert_eq!(v["kills"], "wrong-thing");
}

#[test]
fn checkpoint_action_serializes_kind_snake_case() {
    let h = Harness::start("ser");
    let actions = h.walk_station_to_checkpoint("frame", &["u1"]);
    let v = serde_json::to_value(actions.last().unwrap()).unwrap();
    assert_eq!(v["kind"], "ask");
}

#[test]
fn position_serializes_track_snake_case() {
    let h = Harness::start("ser");
    let pos = h.position();
    let v = serde_json::to_value(&pos).unwrap();
    assert_eq!(v["track"], "run");
}

#[test]
fn tick_result_serializes_run_slug() {
    let h = Harness::start("ser");
    let t = h.tick();
    let v = serde_json::to_value(&t).unwrap();
    assert_eq!(v["run"], "ser");
}

#[test]
fn manufacture_action_units_serialize_as_array() {
    let h = Harness::start("ser");
    h.tick();
    h.tick();
    h.decompose("frame", &[("u1", &[]), ("u2", &[])]);
    let t = h.tick();
    let v = serde_json::to_value(&t.action).unwrap();
    assert!(v["units"].is_array());
}

// ===========================================================================
// Section 10 — Cross-crate: embedded content corpus vs manager factory
// ===========================================================================

#[test]
fn content_software_factory_loads_and_validates() {
    let f = darkrun_content::load_validated("software").unwrap();
    assert_eq!(f.name(), "software");
}

#[test]
fn content_station_order_matches_manager() {
    let f = darkrun_content::load_validated("software").unwrap();
    let names: Vec<&str> = f.stations.iter().map(|s| s.name()).collect();
    assert_eq!(names, STATIONS.to_vec());
}

#[test]
fn content_lists_software_factory() {
    assert!(darkrun_content::list_factories().contains(&"software".to_string()));
}

#[test]
fn content_missing_factory_errors() {
    assert!(darkrun_content::load_factory("ghost").is_err());
}

#[test]
fn content_frame_has_workers() {
    let f = darkrun_content::load_factory("software").unwrap();
    let workers: Vec<&str> = f
        .station("frame")
        .unwrap()
        .workers
        .iter()
        .map(|r| r.name())
        .collect();
    assert_eq!(workers, vec!["framer", "challenger", "distiller"]);
}

#[test]
fn content_frame_workers_match_manager_first_worker() {
    // The manager dispatches the station's first worker as the beat. Confirm
    // the embedded corpus agrees on that first worker for frame.
    let f = darkrun_content::load_factory("software").unwrap();
    let first = f.station("frame").unwrap().workers[0].name().to_string();
    let h = Harness::start("cw");
    h.tick();
    h.tick();
    h.decompose("frame", &[("u1", &[])]);
    let t = h.tick();
    match t.action {
        RunAction::Manufacture { worker, .. } => assert_eq!(worker, first),
        other => panic!("expected Manufacture, got {other:?}"),
    }
}

// ===========================================================================
// Section 11 — Cross-crate: darkrun-api session payload reflects a Run
// ===========================================================================

#[test]
fn api_review_payload_roundtrips_for_a_held_checkpoint() {
    use darkrun_api::session::{ReviewSessionPayload, RunCurrentState, RunPhase, SessionPayload};
    use darkrun_api::{GateType, SessionStatus};

    let h = Harness::start("api");
    h.walk_station_to_checkpoint("frame", &["u1"]);
    // Build a review payload describing the held checkpoint, mirroring what the
    // server would emit from the manager's position.
    let payload = SessionPayload::Review(ReviewSessionPayload {
        session_id: "s-frame".into(),
        status: SessionStatus::Pending,
        run_slug: Some(h.slug.clone()),
        gate_type: Some(GateType::Ask),
        station: Some("frame".into()),
        current_state: Some(RunCurrentState {
            factory: "software".into(),
            station: "frame".into(),
            phase: Some(RunPhase::Checkpoint),
            ..Default::default()
        }),
        ..Default::default()
    });
    let json = serde_json::to_value(&payload).unwrap();
    assert_eq!(json["session_type"], "review");
    assert_eq!(json["station"], "frame");
    let back: SessionPayload = serde_json::from_value(json).unwrap();
    assert_eq!(back.session_type(), "review");
}

#[test]
fn api_gate_type_serializes_snake_case() {
    use darkrun_api::GateType;
    let v = serde_json::to_value(GateType::External).unwrap();
    assert_eq!(v, "external");
}

#[test]
fn api_session_unknown_type_rejected() {
    use darkrun_api::session::SessionPayload;
    let json = serde_json::json!({ "session_type": "telepathy", "session_id": "x" });
    let parsed: Result<SessionPayload, _> = serde_json::from_value(json);
    assert!(parsed.is_err());
}

// ===========================================================================
// Section 12 — Units: create/update over the lifecycle (darkrun-mcp::units)
// ===========================================================================

#[test]
fn units_create_seeds_pending() {
    let h = Harness::start("u");
    let u = darkrun_mcp::units::create(&h.store, "u", "u1", "frame", darkrun_mcp::units::UnitSpec::default()).unwrap();
    assert_eq!(u.frontmatter.status, Status::Pending);
}

#[test]
fn units_create_then_appears_in_wave() {
    let h = Harness::start("u");
    h.tick();
    h.tick();
    darkrun_mcp::units::create(&h.store, "u", "u1", "frame", darkrun_mcp::units::UnitSpec::default()).unwrap();
    h.decide(true, None); // clear the pre-execution operator gate
    let t = h.tick();
    assert!(is_manufacture(&t.action, "frame"));
}

#[test]
fn units_update_completion_unblocks_audit() {
    let h = Harness::start("u");
    h.tick();
    h.tick();
    darkrun_mcp::units::create(&h.store, "u", "u1", "frame", darkrun_mcp::units::UnitSpec::default()).unwrap();
    h.decide(true, None); // clear the pre-execution operator gate
    h.tick(); // dispatch
    darkrun_mcp::units::update(
        &h.store,
        "u",
        "u1",
        darkrun_mcp::units::UnitUpdate {
            status: Some(Status::Completed),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(is_audit(&h.tick().action, "frame"));
}

#[test]
fn units_update_deps_blocked_after_active() {
    let h = Harness::start("u");
    darkrun_mcp::units::create(&h.store, "u", "u1", "frame", darkrun_mcp::units::UnitSpec::default()).unwrap();
    darkrun_mcp::units::update(
        &h.store,
        "u",
        "u1",
        darkrun_mcp::units::UnitUpdate {
            status: Some(Status::Active),
            ..Default::default()
        },
    )
    .unwrap();
    let err = darkrun_mcp::units::update(
        &h.store,
        "u",
        "u1",
        darkrun_mcp::units::UnitUpdate {
            depends_on: Some(vec!["x".into()]),
            ..Default::default()
        },
    );
    assert!(err.is_err());
}

#[test]
fn units_create_duplicate_errors() {
    let h = Harness::start("u");
    darkrun_mcp::units::create(&h.store, "u", "u1", "frame", darkrun_mcp::units::UnitSpec::default()).unwrap();
    assert!(darkrun_mcp::units::create(&h.store, "u", "u1", "frame", darkrun_mcp::units::UnitSpec::default()).is_err());
}

#[test]
fn units_get_missing_errors() {
    let h = Harness::start("u");
    assert!(darkrun_mcp::units::get(&h.store, "u", "ghost").is_err());
}

// ===========================================================================
// Section 13 — Drift preempts feedback and run (Track C)
// ===========================================================================

// ===========================================================================
// Section 14 — Idempotence & re-derivation invariants across full run
// ===========================================================================

#[test]
fn each_phase_position_is_idempotent_across_run() {
    let h = Harness::start("idem");
    // At every cursor stop, deriving twice yields equal positions before the
    // tick that advances. We sample at each station's spec.
    for _ in 0..3 {
        let a = derive_position(&h.store, "idem").unwrap();
        let b = derive_position(&h.store, "idem").unwrap();
        assert_eq!(a, b);
        h.tick();
    }
}

#[test]
fn two_runs_are_isolated() {
    let dir = tempfile::tempdir().unwrap();
    let store = darkrun_core::StateStore::new(dir.path());
    darkrun_mcp::position::run_start(&store, "a", "software", None, darkrun_core::domain::Mode::Solo, "full").unwrap();
    darkrun_mcp::position::run_start(&store, "b", "software", None, darkrun_core::domain::Mode::Solo, "full").unwrap();
    // Advance a; b stays at frame/spec.
    run_tick(&store, "a").unwrap();
    let pb = derive_position(&store, "b").unwrap();
    assert!(is_spec(pb.action.as_ref().unwrap(), "frame"));
}

#[test]
fn sealed_run_stays_sealed_on_retick() {
    let h = Harness::start("seal2");
    h.run_to_seal();
    let t = h.tick();
    assert!(matches!(t.action, RunAction::Sealed { .. }));
}

#[test]
fn full_run_phase_progression_monotone() {
    // Across a single station, the persisted phase only moves forward through
    // the canonical order until checkpoint.
    let h = Harness::start("mono");
    let mut seen = Vec::new();
    seen.push(h.phase("frame")); // spec
    h.tick();
    seen.push(h.phase("frame")); // review
    h.tick();
    seen.push(h.phase("frame")); // user gate (interactive station holds here)
    h.decompose("frame", &[("u1", &[])]); // clears the gate → manufacture
    seen.push(h.phase("frame")); // manufacture
    h.tick();
    h.complete_unit("u1");
    h.tick(); // audit -> reflect
    seen.push(h.phase("frame"));
    h.tick(); // reflect -> checkpoint
    seen.push(h.phase("frame"));
    assert_eq!(
        seen,
        vec![
            StationPhase::Spec,
            StationPhase::Review,
            StationPhase::UserGate,
            StationPhase::Manufacture,
            StationPhase::Reflect,
            StationPhase::Checkpoint,
        ]
    );
}

// ===========================================================================
// Section 15 — Helper to build a raw unit (exercises core directly)
// ===========================================================================

fn raw_unit(slug: &str, station: &str, deps: &[&str]) -> Unit {
    Unit {
        slug: slug.into(),
        frontmatter: UnitFrontmatter {
            status: Status::Pending,
            station: Some(station.into()),
            depends_on: deps.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        },
        title: slug.into(),
        body: String::new(),
    }
}

#[test]
fn core_unit_roundtrips_through_store() {
    let h = Harness::start("core");
    let u = raw_unit("x", "frame", &["dep"]);
    h.store.write_unit("core", &u).unwrap();
    let back = h.store.read_unit("core", "x").unwrap();
    assert_eq!(back.frontmatter.depends_on, vec!["dep".to_string()]);
    assert_eq!(back.station(), "frame");
}

#[test]
fn core_read_units_sorted() {
    let h = Harness::start("core");
    h.store.write_unit("core", &raw_unit("b", "frame", &[])).unwrap();
    h.store.write_unit("core", &raw_unit("a", "frame", &[])).unwrap();
    let units = h.store.read_units("core").unwrap();
    assert_eq!(units[0].slug, "a");
    assert_eq!(units[1].slug, "b");
}
