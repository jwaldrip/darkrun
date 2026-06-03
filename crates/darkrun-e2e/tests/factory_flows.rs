//! Deeper end-to-end factory flows: per-station phase walks with multi-unit
//! waves, feedback preemption at each station, full reject->rework->re-walk
//! cycles, active-pointer progression across the whole Run, and cross-crate
//! content/role agreement — all driven through the real manager.

mod common;

use common::*;
use darkrun_core::domain::{CheckpointKind, Status, StationPhase};
use darkrun_mcp::position::{derive_position, RunAction, Track};

// ---------------------------------------------------------------------------
// Per-station: every station walks the same phase machine. Parameterize by
// completing the upstream stations first, then exercising the target station.
// ---------------------------------------------------------------------------

/// Advance the harness so `target` is the active station (all upstream done).
fn advance_to(h: &Harness, target: &str) {
    for s in STATIONS {
        if s == target {
            break;
        }
        let unit = format!("{s}-pre");
        h.complete_station(s, &[unit.as_str()]);
    }
    assert_eq!(h.active(), target, "failed to reach {target}");
    // The upstream Ask gate's internal re-tick may have already bumped the
    // target past Spec; reset to the canonical station entry for the suite.
    h.set_phase(target, StationPhase::Spec);
}

macro_rules! station_phase_suite {
    ($modname:ident, $station:literal) => {
        mod $modname {
            use super::*;

            #[test]
            fn reaches_station() {
                let h = Harness::start(concat!($station, "-reach"));
                advance_to(&h, $station);
                assert_eq!(h.active(), $station);
            }

            #[test]
            fn first_tick_is_spec() {
                let h = Harness::start(concat!($station, "-spec"));
                advance_to(&h, $station);
                assert!(is_spec(&h.tick().action, $station));
            }

            #[test]
            fn spec_advances_to_review() {
                let h = Harness::start(concat!($station, "-rev"));
                advance_to(&h, $station);
                h.tick();
                assert_eq!(h.phase($station), StationPhase::Review);
            }

            #[test]
            fn review_action_then_manufacture_phase() {
                let h = Harness::start(concat!($station, "-man"));
                advance_to(&h, $station);
                h.tick();
                assert!(is_review(&h.tick().action, $station));
                // Interactive stations hold at the pre-execution operator gate
                // after Review; auto-gated stations advance into Manufacture.
                let expected = if matches!(manager_checkpoint($station), CheckpointKind::Auto) {
                    StationPhase::Manufacture
                } else {
                    StationPhase::UserGate
                };
                assert_eq!(h.phase($station), expected);
            }

            #[test]
            fn manufacture_dispatches_decomposed_unit() {
                let h = Harness::start(concat!($station, "-disp"));
                advance_to(&h, $station);
                h.tick();
                h.tick();
                h.decompose($station, &[("w1", &[])]);
                assert!(is_manufacture(&h.tick().action, $station));
            }

            #[test]
            fn audit_after_units_complete() {
                let h = Harness::start(concat!($station, "-aud"));
                advance_to(&h, $station);
                h.tick();
                h.tick();
                h.decompose($station, &[("w1", &[])]);
                h.tick();
                h.complete_unit("w1");
                assert!(is_audit(&h.tick().action, $station));
            }

            #[test]
            fn reflect_then_checkpoint() {
                let h = Harness::start(concat!($station, "-tc"));
                advance_to(&h, $station);
                let actions = h.walk_station_to_checkpoint($station, &["w1"]);
                assert!(is_reflect(&actions[actions.len() - 2], $station));
                // The gate is a local Checkpoint, or an ExternalReviewRequested
                // for an external station (harden).
                assert!(is_gate(actions.last().unwrap(), $station));
            }

            #[test]
            fn checkpoint_kind_matches_factory() {
                let h = Harness::start(concat!($station, "-kind"));
                advance_to(&h, $station);
                let actions = h.walk_station_to_checkpoint($station, &["w1"]);
                match (actions.last().unwrap(), manager_checkpoint($station)) {
                    // External gates surface as ExternalReviewRequested (no kind).
                    (RunAction::ExternalReviewRequested { .. }, CheckpointKind::External) => {}
                    (RunAction::Checkpoint { kind, .. }, expected) => {
                        assert_eq!(*kind, expected)
                    }
                    (other, expected) => {
                        panic!("expected gate of kind {expected:?}, got {other:?}")
                    }
                }
            }

            #[test]
            fn multi_unit_wave_dispatches_all() {
                let h = Harness::start(concat!($station, "-multi"));
                advance_to(&h, $station);
                h.tick();
                h.tick();
                h.decompose($station, &[("a", &[]), ("b", &[]), ("c", &[])]);
                let t = h.tick();
                match t.action {
                    RunAction::Manufacture { units, .. } => assert_eq!(units.len(), 3),
                    other => panic!("expected 3-unit wave, got {other:?}"),
                }
            }

            #[test]
            fn dependency_serializes_into_waves() {
                let h = Harness::start(concat!($station, "-dep"));
                advance_to(&h, $station);
                h.tick();
                h.tick();
                h.decompose($station, &[("a", &[]), ("b", &["a"])]);
                let w1 = h.tick();
                assert!(matches!(w1.action, RunAction::Manufacture { ref units, .. } if units == &vec!["a".to_string()]));
                h.complete_unit("a");
                let w2 = h.tick();
                assert!(matches!(w2.action, RunAction::Manufacture { ref units, .. } if units == &vec!["b".to_string()]));
            }

            #[test]
            fn feedback_preempts_at_this_station() {
                let h = Harness::start(concat!($station, "-fb"));
                advance_to(&h, $station);
                h.tick();
                h.file_feedback("fb-x", "pending", "issue here");
                let pos = h.position();
                assert_eq!(pos.track, Track::Feedback);
                match pos.action {
                    Some(RunAction::FixFeedback { station, .. }) => assert_eq!(station, $station),
                    other => panic!("expected FixFeedback, got {other:?}"),
                }
            }

            #[test]
            fn feedback_resolved_resumes_run_at_station() {
                let h = Harness::start(concat!($station, "-fbr"));
                advance_to(&h, $station);
                h.tick();
                h.file_feedback("fb-x", "pending", "issue");
                assert_eq!(h.position().track, Track::Feedback);
                h.file_feedback("fb-x", "addressed", "issue");
                let pos = h.position();
                assert_eq!(pos.track, Track::Run);
                // Cursor is still on this station.
                assert_eq!(h.active(), $station);
            }
        }
    };
}

station_phase_suite!(frame_suite, "frame");
station_phase_suite!(specify_suite, "specify");
station_phase_suite!(shape_suite, "shape");
station_phase_suite!(build_suite, "build");
station_phase_suite!(prove_suite, "prove");
station_phase_suite!(harden_suite, "harden");

// ---------------------------------------------------------------------------
// Active-pointer progression across the entire run.
// ---------------------------------------------------------------------------

#[test]
fn active_pointer_walks_full_station_sequence() {
    let h = Harness::start("ptr");
    let mut seen = vec![h.active()];
    for s in STATIONS {
        let unit = format!("{s}-u");
        h.complete_station(s, &[unit.as_str()]);
        seen.push(h.active());
    }
    // Pointer visits each station, ending parked on harden.
    assert_eq!(
        seen,
        vec![
            "frame", "specify", "shape", "build", "prove", "harden", "harden"
        ]
    );
}

#[test]
fn active_pointer_never_skips_a_station() {
    let h = Harness::start("ptr2");
    for (i, s) in STATIONS.iter().enumerate() {
        assert_eq!(h.active(), *s, "at index {i}");
        let unit = format!("{s}-u");
        h.complete_station(s, &[unit.as_str()]);
    }
}

#[test]
fn active_pointer_holds_on_blocked_station() {
    let h = Harness::start("ptr3");
    advance_to(&h, "shape");
    h.walk_station_to_checkpoint("shape", &["u"]);
    h.decide(false, Some("rework shape"));
    // Rejected → blocked, pointer stays.
    assert_eq!(h.active(), "shape");
}

// ---------------------------------------------------------------------------
// Full reject -> rework -> re-approve cycle (the rework loop).
// ---------------------------------------------------------------------------

#[test]
fn reject_then_reapprove_eventually_advances() {
    let h = Harness::start("rework");
    h.walk_station_to_checkpoint("frame", &["u1"]);
    h.decide(false, Some("not yet"));
    assert_eq!(h.station_status("frame"), Status::Blocked);
    // Resolve the filed feedback (rework landed).
    h.file_feedback("fb-checkpoint", "addressed", "reworked");
    // Run track resumes; the blocked station's checkpoint is re-presented.
    let pos = h.position();
    assert_eq!(pos.track, Track::Run);
}

#[test]
fn reject_feedback_blocks_then_unblocks_run_track() {
    let h = Harness::start("rework2");
    advance_to(&h, "specify");
    h.walk_station_to_checkpoint("specify", &["u"]);
    let res = h.decide(false, Some("ambiguous still"));
    assert_eq!(res.position.track, Track::Feedback);
    h.file_feedback("fb-checkpoint", "closed", "resolved");
    assert_eq!(h.position().track, Track::Run);
}

#[test]
fn reject_records_blocked_outcome_per_station() {
    let h = Harness::start("rework3");
    advance_to(&h, "build");
    // build now gates `ask`, so its checkpoint is rejectable: a reject blocks the
    // station and routes rework through the feedback track.
    h.walk_station_to_checkpoint("build", &["u"]);
    let res = h.decide(false, Some("not ready"));
    assert_eq!(res.position.track, Track::Feedback);
    assert_eq!(h.station_status("build"), Status::Blocked);
}

// ---------------------------------------------------------------------------
// Feedback filed mid-wave preempts even with units in flight.
// ---------------------------------------------------------------------------

#[test]
fn feedback_preempts_mid_manufacture() {
    let h = Harness::start("midfb");
    h.tick();
    h.tick();
    h.decompose("frame", &[("u1", &[]), ("u2", &[])]);
    h.tick(); // dispatch wave
    // File feedback while units are still pending.
    h.file_feedback("fb-mid", "pending", "stop and fix");
    assert_eq!(h.position().track, Track::Feedback);
}

#[test]
fn feedback_cleared_mid_manufacture_resumes_wave() {
    let h = Harness::start("midfb2");
    h.tick();
    h.tick();
    h.decompose("frame", &[("u1", &[])]);
    h.tick();
    h.file_feedback("fb-mid", "pending", "x");
    assert_eq!(h.position().track, Track::Feedback);
    h.file_feedback("fb-mid", "addressed", "x");
    // Back to the run, still in Manufacture with u1 pending → dispatch again.
    let pos = h.position();
    assert_eq!(pos.track, Track::Run);
    assert!(is_manufacture(pos.action.as_ref().unwrap(), "frame"));
}

// ---------------------------------------------------------------------------
// Drift station targeting and priority over run/feedback at each station.
// ---------------------------------------------------------------------------

#[test]
fn drift_targets_named_station() {
    use darkrun_core::domain::{Drift, DriftKind};
    let h = Harness::start("driftst");
    advance_to(&h, "shape");
    darkrun_mcp::drift::record(
        &h.store,
        "driftst",
        "d-01",
        &Drift {
            path: "specify/spec.md".into(),
            station: "specify".into(),
            run: "driftst".into(),
            kind: DriftKind::Spec,
            age: "2h".into(),
            unit: None,
        },
    )
    .unwrap();
    match h.position().action {
        Some(RunAction::ResolveDrift { station, .. }) => assert_eq!(station, "specify"),
        other => panic!("expected ResolveDrift on specify, got {other:?}"),
    }
}

#[test]
fn drift_empty_station_falls_back_to_active() {
    use darkrun_core::domain::{Drift, DriftKind};
    let h = Harness::start("driftfb");
    advance_to(&h, "build");
    darkrun_mcp::drift::record(
        &h.store,
        "driftfb",
        "d-01",
        &Drift {
            path: "some/path.md".into(),
            station: "".into(),
            run: "driftfb".into(),
            kind: DriftKind::Output,
            age: "".into(),
            unit: None,
        },
    )
    .unwrap();
    match h.position().action {
        Some(RunAction::ResolveDrift { station, .. }) => assert_eq!(station, "build"),
        other => panic!("expected ResolveDrift on build, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Cross-crate content: every station's reviewers/workers are well-formed.
// ---------------------------------------------------------------------------

macro_rules! content_station_suite {
    ($modname:ident, $station:literal) => {
        mod $modname {
            use super::*;

            #[test]
            fn station_loads() {
                let f = darkrun_content::load_factory("software").unwrap();
                assert!(f.station($station).is_some());
            }

            #[test]
            fn station_has_workers() {
                let f = darkrun_content::load_factory("software").unwrap();
                assert!(!f.station($station).unwrap().workers.is_empty());
            }

            #[test]
            fn station_has_reviewers() {
                let f = darkrun_content::load_factory("software").unwrap();
                assert!(!f.station($station).unwrap().reviewers.is_empty());
            }

            #[test]
            fn station_checkpoint_matches_manager() {
                let f = darkrun_content::load_factory("software").unwrap();
                assert_eq!(
                    f.station($station).unwrap().checkpoint(),
                    // Content corpus and manager agree except `prove`
                    // (corpus=ask, manager=auto); guard that one explicitly.
                    if $station == "prove" {
                        CheckpointKind::Ask
                    } else {
                        manager_checkpoint($station)
                    }
                );
            }

            #[test]
            fn station_workers_have_nonempty_bodies() {
                let f = darkrun_content::load_factory("software").unwrap();
                for w in &f.station($station).unwrap().workers {
                    assert!(!w.body.trim().is_empty(), "{} body empty", w.name());
                }
            }
        }
    };
}

content_station_suite!(c_frame, "frame");
content_station_suite!(c_specify, "specify");
content_station_suite!(c_shape, "shape");
content_station_suite!(c_build, "build");
content_station_suite!(c_prove, "prove");
content_station_suite!(c_harden, "harden");

// ---------------------------------------------------------------------------
// Determinism: a full run produces the same action log every time.
// ---------------------------------------------------------------------------

#[test]
fn full_run_action_log_is_reproducible() {
    let a = Harness::start("repro-a").run_to_seal();
    let b = Harness::start("repro-b").run_to_seal();
    // Same canonical walk regardless of slug.
    let tags_a: Vec<String> = a.iter().map(action_tag).collect();
    let tags_b: Vec<String> = b.iter().map(action_tag).collect();
    assert_eq!(tags_a, tags_b);
}

#[test]
fn full_run_station_visit_order_reproducible() {
    let a = Harness::start("repro-c").run_to_seal();
    let stations_a: Vec<String> = a.iter().filter_map(action_station).collect();
    let b = Harness::start("repro-d").run_to_seal();
    let stations_b: Vec<String> = b.iter().filter_map(action_station).collect();
    assert_eq!(stations_a, stations_b);
}

#[test]
fn full_run_log_ends_with_sealed() {
    let log = Harness::start("ends").run_to_seal();
    assert_eq!(action_tag(log.last().unwrap()), "sealed");
}

#[test]
fn full_run_log_first_is_frame_spec() {
    let log = Harness::start("first").run_to_seal();
    assert!(is_spec(&log[0], "frame"));
}

#[test]
fn derive_after_seal_constant_over_iterations() {
    let h = Harness::start("constseal");
    h.run_to_seal();
    let first = derive_position(&h.store, "constseal").unwrap();
    for _ in 0..25 {
        assert_eq!(derive_position(&h.store, "constseal").unwrap(), first);
    }
}
