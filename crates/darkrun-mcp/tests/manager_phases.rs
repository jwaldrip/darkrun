//! Exhaustive manager phase-machine tests for darkrun-mcp.
//!
//! These drive the public manager API (`run_start`, `run_tick`,
//! `derive_position`, `checkpoint_decide`) over a real on-disk `.darkrun/`
//! tree and walk the full station phase machine
//! `Spec -> Review -> Manufacture -> Audit -> Reflect -> Checkpoint`
//! across EVERY software station (frame, specify, shape, build, prove,
//! harden). They cover:
//!
//! - `derive_position` reads at each phase for each station,
//! - `run_tick` write-cache advancement and determinism,
//! - Manufacture wave-looping with 0/1/many units and dependency waves,
//! - mid-wave noop, Sealed when all stations are done,
//! - `ensure_station` seeding via the public surface,
//! - same-disk-same-derive determinism / idempotency of a pure read.

use darkrun_core::domain::{
    CheckpointKind, CheckpointOutcome, Status, StationPhase, Unit, UnitFrontmatter,
};
use darkrun_core::StateStore;
use darkrun_core::domain::Mode;
use darkrun_mcp::position::{
    checkpoint_decide, derive_position, run_start, run_tick, Position, RunAction,
};
use darkrun_mcp::{resolve_factory, Track};
use tempfile::TempDir;

// ───────────────────────── helpers ─────────────────────────

fn store() -> (TempDir, StateStore) {
    let dir = TempDir::new().expect("tmp");
    let store = StateStore::new(dir.path());
    (dir, store)
}

/// Start a fresh software run and return its store (keeping the tempdir alive).
fn fresh(slug: &str) -> (TempDir, StateStore) {
    fresh_with(slug, Mode::Solo)
}

/// Start a fresh software run in a specific mode.
fn fresh_with(slug: &str, mode: Mode) -> (TempDir, StateStore) {
    let (d, store) = store();
    run_start(&store, slug, "software", None, mode, "full").expect("start");
    (d, store)
}

/// The six ordered software stations.
const STATIONS: [&str; 6] = ["frame", "specify", "shape", "build", "prove", "harden"];

/// The phases in canonical order.
const PHASES: [StationPhase; 7] = [
    StationPhase::Spec,
    StationPhase::Review,
    StationPhase::UserGate,
    StationPhase::Manufacture,
    StationPhase::Audit,
    StationPhase::Reflect,
    StationPhase::Checkpoint,
];

fn checkpoint_kind(station: &str) -> CheckpointKind {
    match station {
        "frame" | "specify" | "shape" | "build" | "prove" | "harden" => CheckpointKind::Ask,
        other => panic!("unknown station {other}"),
    }
}

fn next_station(station: &str) -> Option<&'static str> {
    let idx = STATIONS.iter().position(|s| *s == station).unwrap();
    STATIONS.get(idx + 1).copied()
}

fn is_gated(kind: CheckpointKind) -> bool {
    matches!(
        kind,
        CheckpointKind::Ask | CheckpointKind::External | CheckpointKind::Await
    )
}

fn action_name(a: &RunAction) -> &'static str {
    // Delegate to the crate's single source of truth for action → tag.
    darkrun_mcp::position::action_tag(a)
}

fn action_station(a: &RunAction) -> Option<&str> {
    match a {
        RunAction::Spec { station, .. }
        | RunAction::Review { station, .. }
        | RunAction::Manufacture { station, .. }
        | RunAction::Audit { station, .. }
        | RunAction::Reflect { station, .. }
        | RunAction::UserGate { station, .. }
        | RunAction::Checkpoint { station, .. }
        | RunAction::FixFeedback { station, .. }
        | RunAction::FeedbackQuestion { station, .. }
        | RunAction::UnitsInvalid { station, .. }
        | RunAction::Escalate { station, .. }
        | RunAction::BestEffortBoot { station, .. }
        | RunAction::EscalateToUser { station, .. }
        | RunAction::SafeRepair { station, .. }
        | RunAction::ReviseUnitSpecs { station, .. }
        | RunAction::MergeConflict { station, .. }
        | RunAction::ExternalReviewRequested { station, .. } => Some(station),
        RunAction::RunReview { .. }
        | RunAction::PendingSeal { .. }
        | RunAction::Sealed { .. }
        | RunAction::SaveWip { .. }
        | RunAction::Noop { .. } => None,
    }
}

/// Model the whole-run reviewers signing off: if the run is holding in the
/// run-level review, stamp every declared run reviewer so it can seal.
fn sign_run_reviews(store: &StateStore, run: &str) {
    if let Some(RunAction::RunReview { reviewers, .. }) =
        derive_position(store, run).unwrap().action
    {
        for r in reviewers {
            darkrun_mcp::position::run_review_stamp(store, run, &r).expect("run review stamp");
        }
    }
}

/// Seed a unit at a station with a given status and deps. The unit consumes the
/// station's declared inputs so the runtime input-coverage gate is satisfied.
fn seed_unit(store: &StateStore, run: &str, station: &str, slug: &str, status: Status, deps: &[&str]) {
    let inputs = darkrun_mcp::resolve_factory("software")
        .and_then(|f| f.station(station).map(|d| d.inputs.clone()))
        .unwrap_or_default();
    let unit = Unit {
        slug: slug.into(),
        frontmatter: UnitFrontmatter {
            status,
            station: Some(station.into()),
            depends_on: deps.iter().map(|d| d.to_string()).collect(),
            inputs,
            ..Default::default()
        },
        title: slug.into(),
        body: String::new(),
    };
    store.write_unit(run, &unit).expect("write unit");
}

fn complete_unit(store: &StateStore, run: &str, slug: &str) {
    let mut u = store.read_unit(run, slug).expect("read unit");
    u.frontmatter.status = Status::Completed;
    store.write_unit(run, &u).expect("write unit");
}

/// Force the active station's persisted phase directly (write-cache poke).
fn set_phase(store: &StateStore, run: &str, station: &str, phase: StationPhase) {
    let mut state = store.read_state(run).expect("state").expect("some");
    let st = state.stations.get_mut(station).expect("station seeded");
    st.phase = phase;
    // Positioning the cursor mid-station implies the operator-elaboration has
    // happened — solo otherwise holds the Spec until it's sealed.
    st.elaborated = true;
    if !matches!(st.status, Status::Completed) {
        st.status = Status::InProgress;
    }
    store.write_state(run, &state).expect("write state");
}

/// Advance the run cursor so the given `target` station is the current
/// (incomplete) station, sitting in its `Spec` phase, with all prior stations
/// marked `Completed`. The walk drives each prior station to its checkpoint and
/// approves gated gates; afterward the target is normalized to a freshly-seeded
/// `Spec` so callers get a deterministic starting point regardless of how the
/// prior gate resolved (gated decides re-tick once; auto gates self-advance).
fn advance_to_station(store: &StateStore, run: &str, target: &str) {
    for station in STATIONS {
        if station == target {
            break;
        }
        // Drive this station to its checkpoint with a trivial completed unit.
        seed_unit(store, run, station, &format!("{station}-seed"), Status::Completed, &[]);
        let mut reached = false;
        for _ in 0..16 {
            let t = run_tick(store, run).expect("tick");
            match &t.action {
                // Clear the pre-execution operator gate so the walk proceeds.
                RunAction::UserGate { station: s, .. } if s == station => {
                    checkpoint_decide(store, run, true, None).expect("clear gate");
                }
                // Solo holds the Spec until the elaboration is sealed.
                RunAction::Spec { station: s, .. } if s == station => {
                    darkrun_mcp::position::elaborate_seal(store, run, station).expect("seal");
                }
                RunAction::Checkpoint { station: s, .. } if s == station => {
                    reached = true;
                    break;
                }
                // Auto checkpoints complete the station on the same tick and the
                // action reports that station's checkpoint — handled above. If we
                // already moved past, stop.
                RunAction::Spec { station: s, .. } if s == target => {
                    reached = true;
                    break;
                }
                _ => {}
            }
        }
        assert!(reached, "did not reach checkpoint for {station}");
        if is_gated(checkpoint_kind(station)) {
            checkpoint_decide(store, run, true, None).expect("approve");
        }
    }

    // Normalize: ensure the target is the current station seeded at a clean
    // Spec. The target's Station entry already exists (a gated prior station's
    // `checkpoint_decide` re-ticks once into it; an auto prior station's
    // checkpoint tick seeds the next station on completion). Force it to a
    // pristine Spec/Pending so callers get a deterministic starting cursor.
    let factory = resolve_factory("software").expect("factory");
    let mut state = store.read_state(run).expect("state").unwrap_or_default();
    let gate = state.mode.gate();
    for station in STATIONS {
        if station == target {
            break;
        }
        if let Some(st) = state.stations.get_mut(station) {
            st.status = Status::Completed;
        }
    }
    // Ensure a clean target entry.
    let _ = factory.station(target).expect("station def");
    state.stations.insert(
        target.to_string(),
        darkrun_core::domain::Station {
            station: target.to_string(),
            status: Status::Pending,
            phase: StationPhase::Spec,
            elaborated: false,
            checkpoint: Some(darkrun_core::domain::Checkpoint {
                kind: gate,
                entered_at: None,
                outcome: None,
            }),
            branch: None,
            pr_ref: None,
            pr_status: None,
            pr_ready_at: None,
            pr_merged_at: None,
            verifier_nonce: None,
            started_at: None,
            completed_at: None,
        },
    );
    state.active_station = target.to_string();
    store.write_state(run, &state).expect("write state");

    // Sanity: the cursor now sits on the target's Spec.
    let pos = derive_position(store, run).expect("derive");
    let a = pos.action.expect("action");
    assert_eq!(action_name(&a), "spec", "target should start at Spec");
    assert_eq!(action_station(&a), Some(target));
}

/// Put the active station into a precise phase. `advance_to_station` leaves the
/// target seeded at `Spec`; this just stamps the requested phase.
fn at_phase(store: &StateStore, run: &str, station: &str, phase: StationPhase) {
    advance_to_station(store, run, station);
    set_phase(store, run, station, phase);
}

// ─────────────── derive_position per (station, phase) ───────────────
//
// For each station and each phase we assert derive_position returns the
// matching action with the right station. We generate one test per
// (station, phase) pair plus targeted seeding cases. macro to cut boilerplate.

macro_rules! phase_derive_test {
    ($name:ident, $station:expr, $phase:expr, $expect:expr) => {
        #[test]
        fn $name() {
            let (_d, store) = fresh("r");
            at_phase(&store, "r", $station, $phase);
            // For Manufacture, seed a wave-ready unit so the action is Manufacture.
            if matches!($phase, StationPhase::Manufacture) {
                seed_unit(&store, "r", $station, "wu", Status::Pending, &[]);
            }
            let pos = derive_position(&store, "r").expect("pos");
            assert_eq!(pos.track, Track::Run);
            let a = pos.action.expect("action");
            assert_eq!(action_name(&a), $expect, "got {a:?}");
            assert_eq!(action_station(&a), Some($station));
        }
    };
}

// frame
phase_derive_test!(frame_spec_derives_spec, "frame", StationPhase::Spec, "spec");
phase_derive_test!(frame_review_derives_review, "frame", StationPhase::Review, "review");
phase_derive_test!(frame_manufacture_derives_manufacture, "frame", StationPhase::Manufacture, "manufacture");
phase_derive_test!(frame_audit_derives_audit, "frame", StationPhase::Audit, "audit");
phase_derive_test!(frame_reflect_derives_reflect, "frame", StationPhase::Reflect, "reflect");
phase_derive_test!(frame_checkpoint_derives_checkpoint, "frame", StationPhase::Checkpoint, "checkpoint");

// specify
phase_derive_test!(specify_spec_derives_spec, "specify", StationPhase::Spec, "spec");
phase_derive_test!(specify_review_derives_review, "specify", StationPhase::Review, "review");
phase_derive_test!(specify_manufacture_derives_manufacture, "specify", StationPhase::Manufacture, "manufacture");
phase_derive_test!(specify_audit_derives_audit, "specify", StationPhase::Audit, "audit");
phase_derive_test!(specify_reflect_derives_reflect, "specify", StationPhase::Reflect, "reflect");
phase_derive_test!(specify_checkpoint_derives_checkpoint, "specify", StationPhase::Checkpoint, "checkpoint");

// shape
phase_derive_test!(shape_spec_derives_spec, "shape", StationPhase::Spec, "spec");
phase_derive_test!(shape_review_derives_review, "shape", StationPhase::Review, "review");
phase_derive_test!(shape_manufacture_derives_manufacture, "shape", StationPhase::Manufacture, "manufacture");
phase_derive_test!(shape_audit_derives_audit, "shape", StationPhase::Audit, "audit");
phase_derive_test!(shape_reflect_derives_reflect, "shape", StationPhase::Reflect, "reflect");
phase_derive_test!(shape_checkpoint_derives_checkpoint, "shape", StationPhase::Checkpoint, "checkpoint");

// build
phase_derive_test!(build_spec_derives_spec, "build", StationPhase::Spec, "spec");
phase_derive_test!(build_review_derives_review, "build", StationPhase::Review, "review");
phase_derive_test!(build_manufacture_derives_manufacture, "build", StationPhase::Manufacture, "manufacture");
phase_derive_test!(build_audit_derives_audit, "build", StationPhase::Audit, "audit");
phase_derive_test!(build_reflect_derives_reflect, "build", StationPhase::Reflect, "reflect");
phase_derive_test!(build_checkpoint_derives_checkpoint, "build", StationPhase::Checkpoint, "checkpoint");

// prove
phase_derive_test!(prove_spec_derives_spec, "prove", StationPhase::Spec, "spec");
phase_derive_test!(prove_review_derives_review, "prove", StationPhase::Review, "review");
phase_derive_test!(prove_manufacture_derives_manufacture, "prove", StationPhase::Manufacture, "manufacture");
phase_derive_test!(prove_audit_derives_audit, "prove", StationPhase::Audit, "audit");
phase_derive_test!(prove_reflect_derives_reflect, "prove", StationPhase::Reflect, "reflect");
phase_derive_test!(prove_checkpoint_derives_checkpoint, "prove", StationPhase::Checkpoint, "checkpoint");

// harden
phase_derive_test!(harden_spec_derives_spec, "harden", StationPhase::Spec, "spec");
phase_derive_test!(harden_review_derives_review, "harden", StationPhase::Review, "review");
phase_derive_test!(harden_manufacture_derives_manufacture, "harden", StationPhase::Manufacture, "manufacture");
phase_derive_test!(harden_audit_derives_audit, "harden", StationPhase::Audit, "audit");
phase_derive_test!(harden_reflect_derives_reflect, "harden", StationPhase::Reflect, "reflect");
// Every station (harden included) now gates with a local `ask` Checkpoint.
phase_derive_test!(harden_checkpoint_derives_checkpoint, "harden", StationPhase::Checkpoint, "checkpoint");

// ─────────── run_tick write-cache advancement per (station, phase) ───────────
//
// Each tick on a given phase stamps the next phase into state.json (except
// Manufacture which holds, and Checkpoint which depends on the gate). We test
// the deterministic phase->phase transitions for every station.

macro_rules! tick_advances_phase_test {
    ($name:ident, $station:expr, $from:expr, $to:expr) => {
        #[test]
        fn $name() {
            let (_d, store) = fresh("r");
            at_phase(&store, "r", $station, $from);
            run_tick(&store, "r").expect("tick");
            let state = store.read_state("r").unwrap().unwrap();
            assert_eq!(state.stations[$station].phase, $to, "station {}", $station);
        }
    };
}

// Spec -> Review for every station.
tick_advances_phase_test!(frame_spec_to_review, "frame", StationPhase::Spec, StationPhase::Review);
tick_advances_phase_test!(specify_spec_to_review, "specify", StationPhase::Spec, StationPhase::Review);
tick_advances_phase_test!(shape_spec_to_review, "shape", StationPhase::Spec, StationPhase::Review);
tick_advances_phase_test!(build_spec_to_review, "build", StationPhase::Spec, StationPhase::Review);
tick_advances_phase_test!(prove_spec_to_review, "prove", StationPhase::Spec, StationPhase::Review);
tick_advances_phase_test!(harden_spec_to_review, "harden", StationPhase::Spec, StationPhase::Review);

// Review -> next: an INTERACTIVE station (ask/external) holds at the
// pre-execution UserGate; an AUTO-gated station (build/prove) goes straight to
// Manufacture.
tick_advances_phase_test!(frame_review_to_user_gate, "frame", StationPhase::Review, StationPhase::UserGate);
tick_advances_phase_test!(specify_review_to_user_gate, "specify", StationPhase::Review, StationPhase::UserGate);
tick_advances_phase_test!(shape_review_to_user_gate, "shape", StationPhase::Review, StationPhase::UserGate);
tick_advances_phase_test!(build_review_to_user_gate, "build", StationPhase::Review, StationPhase::UserGate);
tick_advances_phase_test!(prove_review_to_user_gate, "prove", StationPhase::Review, StationPhase::UserGate);
tick_advances_phase_test!(harden_review_to_user_gate, "harden", StationPhase::Review, StationPhase::UserGate);

// Audit -> Reflect for every station.
tick_advances_phase_test!(frame_audit_to_reflect, "frame", StationPhase::Audit, StationPhase::Reflect);
tick_advances_phase_test!(specify_audit_to_reflect, "specify", StationPhase::Audit, StationPhase::Reflect);
tick_advances_phase_test!(shape_audit_to_reflect, "shape", StationPhase::Audit, StationPhase::Reflect);
tick_advances_phase_test!(build_audit_to_reflect, "build", StationPhase::Audit, StationPhase::Reflect);
tick_advances_phase_test!(prove_audit_to_reflect, "prove", StationPhase::Audit, StationPhase::Reflect);
tick_advances_phase_test!(harden_audit_to_reflect, "harden", StationPhase::Audit, StationPhase::Reflect);

// Reflect -> Checkpoint for every station.
tick_advances_phase_test!(frame_reflect_to_checkpoint, "frame", StationPhase::Reflect, StationPhase::Checkpoint);
tick_advances_phase_test!(specify_reflect_to_checkpoint, "specify", StationPhase::Reflect, StationPhase::Checkpoint);
tick_advances_phase_test!(shape_reflect_to_checkpoint, "shape", StationPhase::Reflect, StationPhase::Checkpoint);
tick_advances_phase_test!(build_reflect_to_checkpoint, "build", StationPhase::Reflect, StationPhase::Checkpoint);
tick_advances_phase_test!(prove_reflect_to_checkpoint, "prove", StationPhase::Reflect, StationPhase::Checkpoint);
tick_advances_phase_test!(harden_reflect_to_checkpoint, "harden", StationPhase::Reflect, StationPhase::Checkpoint);

// ─────────── Manufacture holds in Manufacture (one wave per tick) ───────────

macro_rules! manufacture_holds_test {
    ($name:ident, $station:expr) => {
        #[test]
        fn $name() {
            let (_d, store) = fresh("r");
            at_phase(&store, "r", $station, StationPhase::Manufacture);
            // A wave-ready pending unit → Manufacture action, phase stays.
            seed_unit(&store, "r", $station, "u1", Status::Pending, &[]);
            let t = run_tick(&store, "r").expect("tick");
            assert!(matches!(t.action, RunAction::Manufacture { .. }), "got {:?}", t.action);
            let state = store.read_state("r").unwrap().unwrap();
            assert_eq!(state.stations[$station].phase, StationPhase::Manufacture);
        }
    };
}
manufacture_holds_test!(frame_manufacture_holds, "frame");
manufacture_holds_test!(specify_manufacture_holds, "specify");
manufacture_holds_test!(shape_manufacture_holds, "shape");
manufacture_holds_test!(build_manufacture_holds, "build");
manufacture_holds_test!(prove_manufacture_holds, "prove");
manufacture_holds_test!(harden_manufacture_holds, "harden");

// ─────── Manufacture with NO units falls back to Spec (still owes Spec) ───────

macro_rules! manufacture_empty_falls_to_spec_test {
    ($name:ident, $station:expr) => {
        #[test]
        fn $name() {
            let (_d, store) = fresh("r");
            at_phase(&store, "r", $station, StationPhase::Manufacture);
            // No station units at all → Manufacture phase yields a Spec action.
            let pos = derive_position(&store, "r").expect("pos");
            let a = pos.action.expect("action");
            assert_eq!(action_name(&a), "spec", "got {a:?}");
            assert_eq!(action_station(&a), Some($station));
        }
    };
}
manufacture_empty_falls_to_spec_test!(frame_manufacture_empty_spec, "frame");
manufacture_empty_falls_to_spec_test!(specify_manufacture_empty_spec, "specify");
manufacture_empty_falls_to_spec_test!(shape_manufacture_empty_spec, "shape");
manufacture_empty_falls_to_spec_test!(build_manufacture_empty_spec, "build");
manufacture_empty_falls_to_spec_test!(prove_manufacture_empty_spec, "prove");
manufacture_empty_falls_to_spec_test!(harden_manufacture_empty_spec, "harden");

// ───── Manufacture: all-units-complete advances to Audit ─────

macro_rules! manufacture_all_complete_audits_test {
    ($name:ident, $station:expr) => {
        #[test]
        fn $name() {
            let (_d, store) = fresh("r");
            at_phase(&store, "r", $station, StationPhase::Manufacture);
            seed_unit(&store, "r", $station, "u1", Status::Completed, &[]);
            seed_unit(&store, "r", $station, "u2", Status::Completed, &[]);
            let pos = derive_position(&store, "r").expect("pos");
            let a = pos.action.expect("action");
            assert_eq!(action_name(&a), "audit", "got {a:?}");
            // A tick at this point stamps Reflect forward (audit folds in tests).
            run_tick(&store, "r").expect("tick");
            let state = store.read_state("r").unwrap().unwrap();
            assert_eq!(state.stations[$station].phase, StationPhase::Reflect);
        }
    };
}
manufacture_all_complete_audits_test!(frame_manufacture_all_complete_audits, "frame");
manufacture_all_complete_audits_test!(specify_manufacture_all_complete_audits, "specify");
manufacture_all_complete_audits_test!(shape_manufacture_all_complete_audits, "shape");
manufacture_all_complete_audits_test!(build_manufacture_all_complete_audits, "build");
manufacture_all_complete_audits_test!(prove_manufacture_all_complete_audits, "prove");
manufacture_all_complete_audits_test!(harden_manufacture_all_complete_audits, "harden");

// ───── Manufacture: mid-wave noop when a unit is in flight ─────

macro_rules! manufacture_midwave_noop_test {
    ($name:ident, $station:expr) => {
        #[test]
        fn $name() {
            let (_d, store) = fresh("r");
            at_phase(&store, "r", $station, StationPhase::Manufacture);
            // A dispatched, in-flight unit (InProgress): not wave-ready (not
            // Pending) and not all complete → null position (mid-wave noop). A
            // dangling dep would instead be a UnitsInvalid decomposition error.
            seed_unit(&store, "r", $station, "u1", Status::InProgress, &[]);
            let pos = derive_position(&store, "r").expect("pos");
            assert_eq!(pos.track, Track::Run);
            assert!(pos.action.is_none(), "expected mid-wave noop, got {:?}", pos.action);
            let t = run_tick(&store, "r").expect("tick");
            assert!(matches!(t.action, RunAction::Noop { .. }), "got {:?}", t.action);
        }
    };
}
manufacture_midwave_noop_test!(frame_midwave_noop, "frame");
manufacture_midwave_noop_test!(specify_midwave_noop, "specify");
manufacture_midwave_noop_test!(shape_midwave_noop, "shape");
manufacture_midwave_noop_test!(build_midwave_noop, "build");
manufacture_midwave_noop_test!(prove_midwave_noop, "prove");
manufacture_midwave_noop_test!(harden_midwave_noop, "harden");

// ───── Manufacture: in-progress (non-pending, non-complete) unit → noop ─────

macro_rules! manufacture_inflight_status_noop_test {
    ($name:ident, $station:expr, $status:expr) => {
        #[test]
        fn $name() {
            let (_d, store) = fresh("r");
            at_phase(&store, "r", $station, StationPhase::Manufacture);
            // A unit that is neither Pending nor Completed: not wave-ready and
            // not "all complete" → mid-wave noop.
            seed_unit(&store, "r", $station, "u1", $status, &[]);
            let pos = derive_position(&store, "r").expect("pos");
            assert!(pos.action.is_none(), "expected noop for {:?}, got {:?}", $status, pos.action);
        }
    };
}
manufacture_inflight_status_noop_test!(frame_inflight_active_noop, "frame", Status::Active);
manufacture_inflight_status_noop_test!(frame_inflight_inprogress_noop, "frame", Status::InProgress);
manufacture_inflight_status_noop_test!(frame_inflight_blocked_noop, "frame", Status::Blocked);
manufacture_inflight_status_noop_test!(build_inflight_active_noop, "build", Status::Active);
manufacture_inflight_status_noop_test!(build_inflight_inprogress_noop, "build", Status::InProgress);
manufacture_inflight_status_noop_test!(build_inflight_blocked_noop, "build", Status::Blocked);
manufacture_inflight_status_noop_test!(harden_inflight_active_noop, "harden", Status::Active);
manufacture_inflight_status_noop_test!(harden_inflight_inprogress_noop, "harden", Status::InProgress);
manufacture_inflight_status_noop_test!(harden_inflight_blocked_noop, "harden", Status::Blocked);

// ───── Checkpoint action carries the right gate kind per station ─────

macro_rules! checkpoint_kind_test {
    ($name:ident, $station:expr, $kind:expr) => {
        #[test]
        fn $name() {
            let (_d, store) = fresh("r");
            at_phase(&store, "r", $station, StationPhase::Checkpoint);
            let pos = derive_position(&store, "r").expect("pos");
            match pos.action.expect("action") {
                RunAction::Checkpoint { kind, station, .. } => {
                    assert_eq!(kind, $kind);
                    assert_eq!(station, $station);
                }
                other => panic!("expected checkpoint, got {other:?}"),
            }
        }
    };
}
checkpoint_kind_test!(frame_checkpoint_is_ask, "frame", CheckpointKind::Ask);
checkpoint_kind_test!(specify_checkpoint_is_ask, "specify", CheckpointKind::Ask);
checkpoint_kind_test!(shape_checkpoint_is_ask, "shape", CheckpointKind::Ask);
checkpoint_kind_test!(build_checkpoint_is_ask, "build", CheckpointKind::Ask);
checkpoint_kind_test!(prove_checkpoint_is_ask, "prove", CheckpointKind::Ask);
checkpoint_kind_test!(harden_checkpoint_is_ask, "harden", CheckpointKind::Ask);

// Every station gates with a local `ask` Checkpoint (the external-review path is
// reserved for discrete mode, which `effective_checkpoint_kind` forces).
#[test]
fn harden_checkpoint_is_ask_local() {
    let (_d, store) = fresh("r");
    at_phase(&store, "r", "harden", StationPhase::Checkpoint);
    let pos = derive_position(&store, "r").expect("pos");
    match pos.action.expect("action") {
        RunAction::Checkpoint { station, kind, .. } => {
            assert_eq!(station, "harden");
            assert_eq!(kind, CheckpointKind::Ask);
        }
        other => panic!("expected Checkpoint, got {other:?}"),
    }
}

// ───── Auto-gated mode still completes the station on the checkpoint tick ─────
//
// Every software station now gates `ask` by default, so auto-advance only
// happens when the run downgrades its gates (quick / auto mode, where
// `effective_checkpoint_kind` forces `Auto`).

#[test]
fn auto_gated_run_checkpoint_tick_completes_and_advances() {
    let (_d, store) = fresh("r");
    at_phase(&store, "r", "frame", StationPhase::Checkpoint);
    // Downgrade the gates (the quick/auto-mode behaviour): `effective_checkpoint_kind`
    // now forces Auto, so the checkpoint tick completes the station with no decide.
    let mut state = store.read_state("r").unwrap().unwrap();
    state.mode = Mode::Dark;
    store.write_state("r", &state).unwrap();
    let t = run_tick(&store, "r").expect("tick");
    assert!(matches!(t.action, RunAction::Checkpoint { kind: CheckpointKind::Auto, .. }));
    let s = store.read_state("r").unwrap().unwrap();
    assert_eq!(s.stations["frame"].status, Status::Completed);
    assert_eq!(s.active_station, "specify");
}

// ───── Gated checkpoint tick HOLDS the station (no advance) ─────

macro_rules! gated_checkpoint_holds_test {
    ($name:ident, $station:expr, $kind:expr) => {
        #[test]
        fn $name() {
            let (_d, store) = fresh("r");
            at_phase(&store, "r", $station, StationPhase::Checkpoint);
            let t = run_tick(&store, "r").expect("tick");
            assert!(matches!(t.action, RunAction::Checkpoint { kind, .. } if kind == $kind));
            let s = store.read_state("r").unwrap().unwrap();
            // Not completed — held for an operator decision.
            assert_ne!(s.stations[$station].status, Status::Completed);
            assert_eq!(s.active_station, $station);
            // The checkpoint record was stamped with entered_at.
            let cp = s.stations[$station].checkpoint.as_ref().expect("cp");
            assert!(cp.entered_at.is_some());
        }
    };
}
gated_checkpoint_holds_test!(frame_gated_holds, "frame", CheckpointKind::Ask);
gated_checkpoint_holds_test!(specify_gated_holds, "specify", CheckpointKind::Ask);
gated_checkpoint_holds_test!(shape_gated_holds, "shape", CheckpointKind::Ask);
gated_checkpoint_holds_test!(build_gated_holds, "build", CheckpointKind::Ask);
gated_checkpoint_holds_test!(prove_gated_holds, "prove", CheckpointKind::Ask);
gated_checkpoint_holds_test!(harden_gated_holds, "harden", CheckpointKind::Ask);

// ───── checkpoint_decide approve → completes & advances to next Spec ─────

macro_rules! decide_approve_advances_test {
    ($name:ident, $station:expr, $next:expr) => {
        #[test]
        fn $name() {
            let (_d, store) = fresh("r");
            at_phase(&store, "r", $station, StationPhase::Checkpoint);
            run_tick(&store, "r").expect("enter checkpoint");
            let decided = checkpoint_decide(&store, "r", true, None).expect("approve");
            match &decided.action {
                RunAction::Spec { station, .. } => assert_eq!(station, $next),
                other => panic!("expected Spec({}), got {other:?}", $next),
            }
            let s = store.read_state("r").unwrap().unwrap();
            assert_eq!(s.stations[$station].status, Status::Completed);
            assert_eq!(s.active_station, $next);
            let cp = s.stations[$station].checkpoint.as_ref().unwrap();
            assert_eq!(cp.outcome, Some(CheckpointOutcome::Advanced));
        }
    };
}
// Gated (ask) stations require an explicit approve to advance.
decide_approve_advances_test!(frame_approve_to_specify, "frame", "specify");
decide_approve_advances_test!(specify_approve_to_shape, "specify", "shape");
decide_approve_advances_test!(shape_approve_to_build, "shape", "build");

// (Every station now gates `ask`; auto-advance-on-tick is exercised by
// `auto_gated_run_checkpoint_tick_completes_and_advances` for the downgraded
// gate mode, and by the per-station `decide_approve_advances_test` below for the
// default ask flow.)
decide_approve_advances_test!(build_approve_to_prove, "build", "prove");
decide_approve_advances_test!(prove_approve_to_harden, "prove", "harden");

#[test]
fn harden_approve_seals_run() {
    let (_d, store) = fresh("r");
    at_phase(&store, "r", "harden", StationPhase::Checkpoint);
    run_tick(&store, "r").expect("enter checkpoint");
    let decided = checkpoint_decide(&store, "r", true, None).expect("approve");
    let final_action = match decided.action {
        RunAction::Sealed { .. } => decided.action,
        RunAction::RunReview { .. } => {
            sign_run_reviews(&store, "r");
            derive_position(&store, "r").unwrap().action.unwrap()
        }
        other => panic!("expected RunReview or Sealed, got {other:?}"),
    };
    assert!(matches!(&final_action, RunAction::Sealed { run } if run == "r"));
    let s = store.read_state("r").unwrap().unwrap();
    assert_eq!(s.stations["harden"].status, Status::Completed);
}

// ───── Station completion advances to the next station's Spec phase ─────

macro_rules! completion_advances_next_spec_test {
    ($name:ident, $station:expr, $next:expr) => {
        #[test]
        fn $name() {
            let (_d, store) = fresh("r");
            advance_to_station(&store, "r", $next);
            // Now derive_position should report the NEXT station's Spec.
            let pos = derive_position(&store, "r").expect("pos");
            let a = pos.action.expect("action");
            assert_eq!(action_name(&a), "spec");
            assert_eq!(action_station(&a), Some($next));
            // And the prior station is completed.
            let s = store.read_state("r").unwrap().unwrap();
            assert_eq!(s.stations[$station].status, Status::Completed);
        }
    };
}
completion_advances_next_spec_test!(frame_completion_to_specify_spec, "frame", "specify");
completion_advances_next_spec_test!(specify_completion_to_shape_spec, "specify", "shape");
completion_advances_next_spec_test!(shape_completion_to_build_spec, "shape", "build");
completion_advances_next_spec_test!(build_completion_to_prove_spec, "build", "prove");
completion_advances_next_spec_test!(prove_completion_to_harden_spec, "prove", "harden");

// ───── ensure_station seeding via the public surface ─────

#[test]
fn run_start_seeds_only_first_station_in_state() {
    let (_d, store) = fresh("r");
    let state = store.read_state("r").unwrap().unwrap();
    assert_eq!(state.factory, "software");
    assert_eq!(state.active_station, "frame");
    assert_eq!(state.stations.len(), 1);
    assert!(state.stations.contains_key("frame"));
    let frame = &state.stations["frame"];
    assert_eq!(frame.phase, StationPhase::Spec);
    assert_eq!(frame.status, Status::Pending);
    // Seeded with the station's checkpoint kind, not yet entered.
    let cp = frame.checkpoint.as_ref().expect("cp seeded");
    assert_eq!(cp.kind, CheckpointKind::Ask);
    assert!(cp.entered_at.is_none());
    assert!(cp.outcome.is_none());
    assert!(frame.started_at.is_none());
    assert!(frame.completed_at.is_none());
}

#[test]
fn spec_tick_seeds_started_at_and_inprogress() {
    let (_d, store) = fresh("r");
    // Solo holds the Spec until sealed; seal so the spec tick advances to Review.
    darkrun_mcp::position::elaborate_seal(&store, "r", "frame").expect("seal");
    run_tick(&store, "r").expect("spec tick");
    let state = store.read_state("r").unwrap().unwrap();
    let frame = &state.stations["frame"];
    assert_eq!(frame.status, Status::InProgress);
    assert_eq!(frame.phase, StationPhase::Review);
    assert!(frame.started_at.is_some(), "started_at stamped on first spec tick");
}

#[test]
fn started_at_is_stable_across_repeat_spec_ticks() {
    let (_d, store) = fresh("r");
    run_tick(&store, "r").expect("spec tick");
    let first = store.read_state("r").unwrap().unwrap().stations["frame"]
        .started_at
        .clone();
    // Reset to spec and re-tick: started_at must not be overwritten.
    set_phase(&store, "r", "frame", StationPhase::Spec);
    run_tick(&store, "r").expect("spec tick 2");
    let second = store.read_state("r").unwrap().unwrap().stations["frame"]
        .started_at
        .clone();
    assert_eq!(first, second, "started_at is set once and preserved");
}

#[test]
fn next_station_seeded_on_completion_with_checkpoint_kind() {
    let (_d, store) = fresh("r");
    advance_to_station(&store, "r", "specify");
    let state = store.read_state("r").unwrap().unwrap();
    let specify = state.stations.get("specify").expect("specify seeded");
    assert_eq!(specify.status, Status::Pending);
    assert_eq!(specify.phase, StationPhase::Spec);
    // specify's gate is Ask.
    assert_eq!(specify.checkpoint.as_ref().unwrap().kind, CheckpointKind::Ask);
}

#[test]
fn build_station_seeded_with_ask_checkpoint_kind() {
    let (_d, store) = fresh("r");
    advance_to_station(&store, "r", "build");
    let state = store.read_state("r").unwrap().unwrap();
    let build = state.stations.get("build").expect("build seeded");
    assert_eq!(build.checkpoint.as_ref().unwrap().kind, CheckpointKind::Ask);
}

#[test]
fn harden_station_seeded_with_ask_checkpoint_kind() {
    let (_d, store) = fresh("r");
    advance_to_station(&store, "r", "harden");
    let state = store.read_state("r").unwrap().unwrap();
    let harden = state.stations.get("harden").expect("harden seeded");
    assert_eq!(harden.checkpoint.as_ref().unwrap().kind, CheckpointKind::Ask);
}

// ───── Sealed when every station is done ─────

#[test]
fn sealed_after_all_six_stations() {
    let (_d, store) = fresh("r");
    for station in STATIONS {
        seed_unit(&store, "r", station, &format!("{station}-u"), Status::Completed, &[]);
        let mut reached = false;
        for _ in 0..14 {
            let t = run_tick(&store, "r").expect("tick");
            // Clear the pre-execution operator gate so the wave releases.
            if matches!(&t.action, RunAction::UserGate { station: s, .. } if s == station) {
                checkpoint_decide(&store, "r", true, None).expect("clear gate");
                continue;
            }
            // Solo holds the Spec until the elaboration is sealed.
            if matches!(&t.action, RunAction::Spec { station: s, .. } if s == station) {
                darkrun_mcp::position::elaborate_seal(&store, "r", station).expect("seal");
                continue;
            }
            // The gate is a local Checkpoint or, for harden, ExternalReviewRequested.
            let at_gate = matches!(&t.action, RunAction::Checkpoint { station: s, .. } if s == station)
                || matches!(&t.action, RunAction::ExternalReviewRequested { station: s, .. } if s == station);
            if at_gate {
                reached = true;
                break;
            }
        }
        assert!(reached, "did not reach {station} checkpoint");
        if is_gated(checkpoint_kind(station)) {
            checkpoint_decide(&store, "r", true, None).expect("approve");
        }
    }
    sign_run_reviews(&store, "r");
    let pos = derive_position(&store, "r").expect("pos");
    assert!(matches!(pos.action, Some(RunAction::Sealed { .. })));
    assert_eq!(pos.track, Track::Run);
}

#[test]
fn sealed_action_carries_run_slug() {
    let (_d, store) = fresh("my-sealed-run");
    // Seed every station entry and mark it completed directly in state.
    let factory = resolve_factory("software").unwrap();
    let mut state = store.read_state("my-sealed-run").unwrap().unwrap();
    for s in &factory.stations {
        state.stations.insert(
            s.name.clone(),
            darkrun_core::domain::Station {
                station: s.name.clone(),
                status: Status::Completed,
                phase: StationPhase::Checkpoint,
            elaborated: false,
                checkpoint: None,
                branch: None,
                pr_ref: None,
                pr_status: None,
                pr_ready_at: None,
                pr_merged_at: None,
                verifier_nonce: None,
                started_at: None,
                completed_at: None,
            },
        );
    }
    store.write_state("my-sealed-run", &state).unwrap();
    sign_run_reviews(&store, "my-sealed-run");
    let pos = derive_position(&store, "my-sealed-run").expect("pos");
    match pos.action {
        Some(RunAction::Sealed { run }) => assert_eq!(run, "my-sealed-run"),
        other => panic!("expected Sealed, got {other:?}"),
    }
}

#[test]
fn sealed_tick_is_noop_for_state_advancement() {
    let (_d, store) = fresh("r");
    // Drive to sealed.
    for station in STATIONS {
        seed_unit(&store, "r", station, &format!("{station}-u"), Status::Completed, &[]);
        for _ in 0..14 {
            let t = run_tick(&store, "r").expect("tick");
            if matches!(&t.action, RunAction::UserGate { station: s, .. } if s == station) {
                checkpoint_decide(&store, "r", true, None).expect("clear gate");
                continue;
            }
            if matches!(&t.action, RunAction::Checkpoint { station: s, .. } if s == station) {
                break;
            }
        }
        if is_gated(checkpoint_kind(station)) {
            checkpoint_decide(&store, "r", true, None).expect("approve");
        }
    }
    sign_run_reviews(&store, "r");
    // A tick on a sealed run yields Sealed and doesn't crash / mutate stations.
    let before = store.read_state("r").unwrap().unwrap();
    let t = run_tick(&store, "r").expect("tick");
    assert!(matches!(t.action, RunAction::Sealed { .. }));
    let after = store.read_state("r").unwrap().unwrap();
    assert_eq!(before.active_station, after.active_station);
    assert_eq!(before.stations.len(), after.stations.len());
}

// ───── Manufacture wave dependency ordering: 0 / 1 / many ─────

#[test]
fn manufacture_zero_units_yields_spec() {
    let (_d, store) = fresh("r");
    at_phase(&store, "r", "frame", StationPhase::Manufacture);
    let pos = derive_position(&store, "r").expect("pos");
    assert_eq!(action_name(&pos.action.unwrap()), "spec");
}

#[test]
fn manufacture_one_pending_unit_dispatches_it() {
    let (_d, store) = fresh("r");
    at_phase(&store, "r", "frame", StationPhase::Manufacture);
    seed_unit(&store, "r", "frame", "only", Status::Pending, &[]);
    let pos = derive_position(&store, "r").expect("pos");
    match pos.action.unwrap() {
        RunAction::Manufacture { units, .. } => assert_eq!(units, vec!["only".to_string()]),
        other => panic!("expected Manufacture, got {other:?}"),
    }
}

#[test]
fn manufacture_many_independent_units_all_ready_in_one_wave() {
    let (_d, store) = fresh("r");
    at_phase(&store, "r", "frame", StationPhase::Manufacture);
    for s in ["a", "b", "c", "d"] {
        seed_unit(&store, "r", "frame", s, Status::Pending, &[]);
    }
    let pos = derive_position(&store, "r").expect("pos");
    match pos.action.unwrap() {
        RunAction::Manufacture { mut units, .. } => {
            units.sort();
            assert_eq!(units, vec!["a", "b", "c", "d"]);
        }
        other => panic!("expected Manufacture, got {other:?}"),
    }
}

#[test]
fn manufacture_dependency_wave_only_exposes_ready_units() {
    let (_d, store) = fresh("r");
    at_phase(&store, "r", "frame", StationPhase::Manufacture);
    // a -> ready; b depends on a -> blocked until a completes.
    seed_unit(&store, "r", "frame", "a", Status::Pending, &[]);
    seed_unit(&store, "r", "frame", "b", Status::Pending, &["a"]);
    let pos = derive_position(&store, "r").expect("pos");
    match pos.action.unwrap() {
        RunAction::Manufacture { units, .. } => assert_eq!(units, vec!["a".to_string()]),
        other => panic!("expected Manufacture only a, got {other:?}"),
    }
    // Complete a → b becomes wave-ready.
    complete_unit(&store, "r", "a");
    let pos = derive_position(&store, "r").expect("pos");
    match pos.action.unwrap() {
        RunAction::Manufacture { units, .. } => assert_eq!(units, vec!["b".to_string()]),
        other => panic!("expected Manufacture b, got {other:?}"),
    }
}

#[test]
fn manufacture_diamond_dependency_waves() {
    let (_d, store) = fresh("r");
    at_phase(&store, "r", "frame", StationPhase::Manufacture);
    // a -> {b, c} -> d   (diamond)
    seed_unit(&store, "r", "frame", "a", Status::Pending, &[]);
    seed_unit(&store, "r", "frame", "b", Status::Pending, &["a"]);
    seed_unit(&store, "r", "frame", "c", Status::Pending, &["a"]);
    seed_unit(&store, "r", "frame", "d", Status::Pending, &["b", "c"]);

    // Wave 1: only a.
    let units = manufacture_units(&store, "r");
    assert_eq!(units, vec!["a".to_string()]);
    complete_unit(&store, "r", "a");

    // Wave 2: b and c (parallel).
    let mut units = manufacture_units(&store, "r");
    units.sort();
    assert_eq!(units, vec!["b".to_string(), "c".to_string()]);
    complete_unit(&store, "r", "b");

    // Still mid-wave: only c ready (d needs both b and c).
    let units = manufacture_units(&store, "r");
    assert_eq!(units, vec!["c".to_string()]);
    complete_unit(&store, "r", "c");

    // Wave 3: d.
    let units = manufacture_units(&store, "r");
    assert_eq!(units, vec!["d".to_string()]);
    complete_unit(&store, "r", "d");

    // All complete → Audit.
    let pos = derive_position(&store, "r").expect("pos");
    assert_eq!(action_name(&pos.action.unwrap()), "audit");
}

fn manufacture_units(store: &StateStore, run: &str) -> Vec<String> {
    let pos = derive_position(store, run).expect("pos");
    match pos.action.expect("action") {
        RunAction::Manufacture { units, .. } => units,
        other => panic!("expected Manufacture, got {other:?}"),
    }
}

#[test]
fn manufacture_chain_dependency_three_waves() {
    let (_d, store) = fresh("r");
    at_phase(&store, "r", "frame", StationPhase::Manufacture);
    seed_unit(&store, "r", "frame", "x", Status::Pending, &[]);
    seed_unit(&store, "r", "frame", "y", Status::Pending, &["x"]);
    seed_unit(&store, "r", "frame", "z", Status::Pending, &["y"]);

    assert_eq!(manufacture_units(&store, "r"), vec!["x".to_string()]);
    complete_unit(&store, "r", "x");
    assert_eq!(manufacture_units(&store, "r"), vec!["y".to_string()]);
    complete_unit(&store, "r", "y");
    assert_eq!(manufacture_units(&store, "r"), vec!["z".to_string()]);
    complete_unit(&store, "r", "z");
    let pos = derive_position(&store, "r").expect("pos");
    assert_eq!(action_name(&pos.action.unwrap()), "audit");
}

#[test]
fn manufacture_units_from_other_stations_are_ignored() {
    let (_d, store) = fresh("r");
    at_phase(&store, "r", "frame", StationPhase::Manufacture);
    // A pending unit on a DIFFERENT station must not appear in frame's wave.
    seed_unit(&store, "r", "specify", "foreign", Status::Pending, &[]);
    // No frame units at all → frame still owes Spec.
    let pos = derive_position(&store, "r").expect("pos");
    assert_eq!(action_name(&pos.action.unwrap()), "spec");
    // Add a frame unit; only it is dispatched.
    seed_unit(&store, "r", "frame", "mine", Status::Pending, &[]);
    let pos = derive_position(&store, "r").expect("pos");
    match pos.action.unwrap() {
        RunAction::Manufacture { units, .. } => assert_eq!(units, vec!["mine".to_string()]),
        other => panic!("got {other:?}"),
    }
}

#[test]
fn manufacture_cross_station_dependency_is_treated_as_unmet() {
    let (_d, store) = fresh("r");
    at_phase(&store, "r", "frame", StationPhase::Manufacture);
    // A frame unit depends on a slug that lives on a DIFFERENT station. Wave
    // readiness is resolved within the active station's own unit slice, so the
    // dependency is not found and is treated as unmet — the unit is NOT ready.
    seed_unit(&store, "r", "specify", "dep", Status::Completed, &[]);
    seed_unit(&store, "r", "frame", "u", Status::Pending, &["dep"]);
    let pos = derive_position(&store, "r").expect("pos");
    assert!(
        pos.action.is_none(),
        "cross-station dep keeps the unit out of the wave (mid-wave noop), got {:?}",
        pos.action
    );

    // Re-seeding the dependency WITHIN the frame station makes the unit ready.
    seed_unit(&store, "r", "frame", "dep", Status::Completed, &[]);
    let pos = derive_position(&store, "r").expect("pos");
    match pos.action.unwrap() {
        RunAction::Manufacture { units, .. } => assert_eq!(units, vec!["u".to_string()]),
        other => panic!("got {other:?}"),
    }
}

// ───── Manufacture worker beat carried from factory ─────

macro_rules! manufacture_worker_test {
    ($name:ident, $station:expr, $worker:expr) => {
        #[test]
        fn $name() {
            let (_d, store) = fresh("r");
            at_phase(&store, "r", $station, StationPhase::Manufacture);
            seed_unit(&store, "r", $station, "u", Status::Pending, &[]);
            let pos = derive_position(&store, "r").expect("pos");
            match pos.action.unwrap() {
                RunAction::Manufacture { worker, .. } => assert_eq!(worker, $worker),
                other => panic!("got {other:?}"),
            }
        }
    };
}
manufacture_worker_test!(frame_first_worker, "frame", "framer");
manufacture_worker_test!(specify_first_worker, "specify", "spec_writer");
manufacture_worker_test!(shape_first_worker, "shape", "designer");
manufacture_worker_test!(build_first_worker, "build", "test_author");
manufacture_worker_test!(prove_first_worker, "prove", "verifier");
manufacture_worker_test!(harden_first_worker, "harden", "hardener");

// ───── Spec action carries the station's `kills` framing ─────

macro_rules! spec_kills_test {
    ($name:ident, $station:expr, $kills:expr) => {
        #[test]
        fn $name() {
            let (_d, store) = fresh("r");
            at_phase(&store, "r", $station, StationPhase::Spec);
            let pos = derive_position(&store, "r").expect("pos");
            match pos.action.unwrap() {
                RunAction::Spec { kills, .. } => assert_eq!(kills, $kills),
                other => panic!("got {other:?}"),
            }
        }
    };
}
spec_kills_test!(frame_kills_wrong_thing, "frame", "wrong-thing");
spec_kills_test!(specify_kills_ambiguity, "specify", "ambiguity");
spec_kills_test!(shape_kills_reversal, "shape", "expensive-structural-reversal");
spec_kills_test!(build_kills_defects, "build", "implementation-defects");
spec_kills_test!(prove_kills_escaped, "prove", "escaped-defects");
spec_kills_test!(harden_kills_prod, "harden", "works-in-dev-dies-in-prod");

// ───── Review/Audit actions carry the station's reviewers ─────

macro_rules! review_reviewers_test {
    ($name:ident, $station:expr, $r0:expr, $r1:expr) => {
        #[test]
        fn $name() {
            let (_d, store) = fresh("r");
            at_phase(&store, "r", $station, StationPhase::Review);
            let pos = derive_position(&store, "r").expect("pos");
            match pos.action.unwrap() {
                RunAction::Review { reviewers, .. } => {
                    assert_eq!(reviewers, vec![$r0.to_string(), $r1.to_string()]);
                }
                other => panic!("got {other:?}"),
            }
        }
    };
}

// Shape carries THREE reviewers (fit / reversibility / simplicity), in both its
// Review and Audit actions — the 2-arg macros can't express it.
macro_rules! shape_three_reviewers {
    ($name:ident, $variant:ident) => {
        #[test]
        fn $name() {
            let (_d, store) = fresh("r");
            at_phase(&store, "r", "shape", StationPhase::$variant);
            let pos = derive_position(&store, "r").expect("pos");
            match pos.action.unwrap() {
                RunAction::$variant { reviewers, .. } => {
                    assert_eq!(reviewers, vec!["fit", "reversibility", "simplicity"]);
                }
                other => panic!("got {other:?}"),
            }
        }
    };
}

review_reviewers_test!(frame_reviewers, "frame", "value", "feasibility");
review_reviewers_test!(specify_reviewers, "specify", "testability", "completeness");
shape_three_reviewers!(shape_reviewers, Review);
review_reviewers_test!(build_reviewers, "build", "correctness", "maintainability");
review_reviewers_test!(prove_reviewers, "prove", "evidence", "coverage");
review_reviewers_test!(harden_reviewers, "harden", "security", "readiness");

macro_rules! audit_reviewers_test {
    ($name:ident, $station:expr, $r0:expr, $r1:expr) => {
        #[test]
        fn $name() {
            let (_d, store) = fresh("r");
            at_phase(&store, "r", $station, StationPhase::Audit);
            let pos = derive_position(&store, "r").expect("pos");
            match pos.action.unwrap() {
                RunAction::Audit { reviewers, .. } => {
                    assert_eq!(reviewers, vec![$r0.to_string(), $r1.to_string()]);
                }
                other => panic!("got {other:?}"),
            }
        }
    };
}
audit_reviewers_test!(frame_audit_reviewers, "frame", "value", "feasibility");
audit_reviewers_test!(specify_audit_reviewers, "specify", "testability", "completeness");
shape_three_reviewers!(shape_audit_reviewers, Audit);
audit_reviewers_test!(build_audit_reviewers, "build", "correctness", "maintainability");
audit_reviewers_test!(prove_audit_reviewers, "prove", "evidence", "coverage");
audit_reviewers_test!(harden_audit_reviewers, "harden", "security", "readiness");

// ───── Reflect action carries run + station only ─────

macro_rules! reflect_action_test {
    ($name:ident, $station:expr) => {
        #[test]
        fn $name() {
            let (_d, store) = fresh("r");
            at_phase(&store, "r", $station, StationPhase::Reflect);
            let pos = derive_position(&store, "r").expect("pos");
            match pos.action.unwrap() {
                RunAction::Reflect { run, station } => {
                    assert_eq!(run, "r");
                    assert_eq!(station, $station);
                }
                other => panic!("got {other:?}"),
            }
        }
    };
}
reflect_action_test!(frame_reflect_action, "frame");
reflect_action_test!(specify_reflect_action, "specify");
reflect_action_test!(shape_reflect_action, "shape");
reflect_action_test!(build_reflect_action, "build");
reflect_action_test!(prove_reflect_action, "prove");
reflect_action_test!(harden_reflect_action, "harden");

// ───── Determinism: same disk → same derive ─────

macro_rules! determinism_phase_test {
    ($name:ident, $station:expr, $phase:expr) => {
        #[test]
        fn $name() {
            let (_d, store) = fresh("r");
            at_phase(&store, "r", $station, $phase);
            if matches!($phase, StationPhase::Manufacture) {
                seed_unit(&store, "r", $station, "u", Status::Pending, &[]);
            }
            let p1 = derive_position(&store, "r").expect("p1");
            let p2 = derive_position(&store, "r").expect("p2");
            let p3 = derive_position(&store, "r").expect("p3");
            assert_eq!(p1, p2);
            assert_eq!(p2, p3);
        }
    };
}
determinism_phase_test!(determinism_frame_spec, "frame", StationPhase::Spec);
determinism_phase_test!(determinism_frame_review, "frame", StationPhase::Review);
determinism_phase_test!(determinism_frame_manufacture, "frame", StationPhase::Manufacture);
determinism_phase_test!(determinism_frame_audit, "frame", StationPhase::Audit);
determinism_phase_test!(determinism_frame_reflect, "frame", StationPhase::Reflect);
determinism_phase_test!(determinism_frame_checkpoint, "frame", StationPhase::Checkpoint);
determinism_phase_test!(determinism_build_manufacture, "build", StationPhase::Manufacture);
determinism_phase_test!(determinism_build_checkpoint, "build", StationPhase::Checkpoint);
determinism_phase_test!(determinism_harden_checkpoint, "harden", StationPhase::Checkpoint);
determinism_phase_test!(determinism_specify_reflect, "specify", StationPhase::Reflect);
determinism_phase_test!(determinism_shape_review, "shape", StationPhase::Review);
determinism_phase_test!(determinism_prove_audit, "prove", StationPhase::Audit);

#[test]
fn derive_does_not_mutate_disk() {
    let (_d, store) = fresh("r");
    at_phase(&store, "r", "frame", StationPhase::Review);
    let before = store.read_state("r").unwrap().unwrap();
    let _ = derive_position(&store, "r").unwrap();
    let _ = derive_position(&store, "r").unwrap();
    let after = store.read_state("r").unwrap().unwrap();
    assert_eq!(before.active_station, after.active_station);
    assert_eq!(before.stations["frame"].phase, after.stations["frame"].phase);
    assert_eq!(before.stations["frame"].status, after.stations["frame"].status);
}

#[test]
fn two_stores_same_disk_same_derive() {
    let dir = TempDir::new().unwrap();
    let a = StateStore::new(dir.path());
    run_start(&a, "r", "software", None, Mode::Solo, "full").unwrap();
    at_phase(&a, "r", "frame", StationPhase::Manufacture);
    seed_unit(&a, "r", "frame", "u", Status::Pending, &[]);
    // A second store rooted at the same disk derives identically.
    let b = StateStore::new(dir.path());
    let pa = derive_position(&a, "r").unwrap();
    let pb = derive_position(&b, "r").unwrap();
    assert_eq!(pa, pb);
}

#[test]
fn manufacture_wave_dispatch_is_order_independent_of_query() {
    let (_d, store) = fresh("r");
    at_phase(&store, "r", "frame", StationPhase::Manufacture);
    seed_unit(&store, "r", "frame", "m", Status::Pending, &[]);
    seed_unit(&store, "r", "frame", "n", Status::Pending, &[]);
    seed_unit(&store, "r", "frame", "o", Status::Pending, &[]);
    let p1 = manufacture_units(&store, "r");
    let p2 = manufacture_units(&store, "r");
    assert_eq!(p1, p2, "wave membership stable across repeated derives");
}

// ───── run_tick on Manufacture does not lose units / re-stamps Manufacture ─────

#[test]
fn run_tick_manufacture_keeps_phase_and_redispatches_remaining() {
    let (_d, store) = fresh("r");
    at_phase(&store, "r", "frame", StationPhase::Manufacture);
    seed_unit(&store, "r", "frame", "a", Status::Pending, &[]);
    seed_unit(&store, "r", "frame", "b", Status::Pending, &["a"]);

    // Tick 1: dispatches a, phase stays Manufacture.
    let t1 = run_tick(&store, "r").expect("t1");
    assert!(matches!(&t1.action, RunAction::Manufacture { units, .. } if units == &vec!["a".to_string()]));
    assert_eq!(
        store.read_state("r").unwrap().unwrap().stations["frame"].phase,
        StationPhase::Manufacture
    );

    // Caller completes a; next tick dispatches b.
    complete_unit(&store, "r", "a");
    let t2 = run_tick(&store, "r").expect("t2");
    assert!(matches!(&t2.action, RunAction::Manufacture { units, .. } if units == &vec!["b".to_string()]));

    // Complete b; next tick audits.
    complete_unit(&store, "r", "b");
    let t3 = run_tick(&store, "r").expect("t3");
    assert!(matches!(t3.action, RunAction::Audit { .. }));
    assert_eq!(
        store.read_state("r").unwrap().unwrap().stations["frame"].phase,
        StationPhase::Reflect
    );
}

// ───── Full per-station phase walk (Spec..Checkpoint), generated ─────

macro_rules! full_station_walk_test {
    ($name:ident, $station:expr) => {
        #[test]
        fn $name() {
            let (_d, store) = fresh("r");
            advance_to_station(&store, "r", $station);
            // Spec — solo holds it until the elaboration is sealed.
            let t = run_tick(&store, "r").unwrap();
            assert_eq!(action_name(&t.action), "spec");
            assert_eq!(action_station(&t.action), Some($station));
            darkrun_mcp::position::elaborate_seal(&store, "r", $station).unwrap();
            run_tick(&store, "r").unwrap(); // advance past the sealed Spec
            // Review
            let t = run_tick(&store, "r").unwrap();
            assert_eq!(action_name(&t.action), "review");
            // Decompose a unit. An interactive station (ask/external) holds at
            // the pre-execution operator gate before manufacture; an auto-gated
            // station (build/prove) releases the wave straight away.
            seed_unit(&store, "r", $station, "u1", Status::Pending, &[]);
            let t = run_tick(&store, "r").unwrap();
            let t = if action_name(&t.action) == "user_gate" {
                checkpoint_decide(&store, "r", true, None).unwrap();
                run_tick(&store, "r").unwrap()
            } else {
                t
            };
            assert_eq!(action_name(&t.action), "manufacture");
            complete_unit(&store, "r", "u1");
            // Audit (folds in the old tests phase)
            let t = run_tick(&store, "r").unwrap();
            assert_eq!(action_name(&t.action), "audit");
            // Reflect
            let t = run_tick(&store, "r").unwrap();
            assert_eq!(action_name(&t.action), "reflect");
            // The gate — a local checkpoint, or an external review for an
            // external station (harden).
            let t = run_tick(&store, "r").unwrap();
            let gate = action_name(&t.action);
            assert!(
                gate == "checkpoint" || gate == "external_review_requested",
                "expected a gate action, got {gate}"
            );
        }
    };
}
full_station_walk_test!(full_walk_frame, "frame");
full_station_walk_test!(full_walk_specify, "specify");
full_station_walk_test!(full_walk_shape, "shape");
full_station_walk_test!(full_walk_build, "build");
full_station_walk_test!(full_walk_prove, "prove");
full_station_walk_test!(full_walk_harden, "harden");

// ───── Phase ordering sanity over the canonical sequence ─────

#[test]
fn phases_const_is_canonical_order() {
    assert_eq!(
        PHASES,
        [
            StationPhase::Spec,
            StationPhase::Review,
            StationPhase::UserGate,
            StationPhase::Manufacture,
            StationPhase::Audit,
            StationPhase::Reflect,
            StationPhase::Checkpoint,
        ]
    );
}

#[test]
fn stations_const_matches_factory_order() {
    let f = resolve_factory("software").unwrap();
    assert_eq!(f.station_names(), STATIONS.to_vec());
}

#[test]
fn next_station_helper_matches_factory() {
    let f = resolve_factory("software").unwrap();
    for s in STATIONS {
        assert_eq!(
            f.next_station(s).map(|d| d.name.as_str()),
            next_station(s),
            "station {s}"
        );
    }
}

// ───── Generated: every phase-pair derive for every station via loop ─────
//
// These iterate the static tables to assert derive_position emits the matching
// action for each of the 36 (station, phase) combinations in a single test.

#[test]
fn all_station_phase_pairs_derive_expected_action() {
    let expected = |_station: &str, p: StationPhase| -> &'static str {
        match p {
            StationPhase::Spec => "spec",
            StationPhase::Review => "review",
            StationPhase::Manufacture => "manufacture",
            StationPhase::Audit => "audit",
            StationPhase::Reflect => "reflect",
            StationPhase::UserGate => "user_gate",
            StationPhase::Checkpoint => "checkpoint",
        }
    };
    for station in STATIONS {
        for phase in PHASES {
            let (_d, store) = fresh("r");
            at_phase(&store, "r", station, phase);
            if matches!(phase, StationPhase::Manufacture) {
                seed_unit(&store, "r", station, "u", Status::Pending, &[]);
            }
            let pos = derive_position(&store, "r").expect("pos");
            let a = pos.action.expect("action");
            assert_eq!(action_name(&a), expected(station, phase), "station {station} phase {phase:?}");
            assert_eq!(action_station(&a), Some(station));
        }
    }
}

// ───── Idempotency: re-deriving after a tick reflects the new phase ─────

#[test]
fn tick_then_derive_reflects_advanced_phase() {
    let (_d, store) = fresh("r");
    // Solo holds the Spec until sealed; seal so the tick advances to Review.
    darkrun_mcp::position::elaborate_seal(&store, "r", "frame").unwrap();
    // Spec → after tick, derive should now report Review.
    run_tick(&store, "r").unwrap();
    let pos = derive_position(&store, "r").unwrap();
    assert_eq!(action_name(&pos.action.unwrap()), "review");
    // Review → after tick, an interactive (continuous) station holds at the
    // pre-execution operator gate before manufacture.
    run_tick(&store, "r").unwrap();
    let pos = derive_position(&store, "r").unwrap();
    assert_eq!(action_name(&pos.action.unwrap()), "user_gate");
}

// ───── unknown factory / unknown run error paths ─────

#[test]
fn derive_unknown_run_errors() {
    let (_d, store) = store();
    assert!(derive_position(&store, "nope").is_err());
}

#[test]
fn run_tick_unknown_run_errors() {
    let (_d, store) = store();
    assert!(run_tick(&store, "nope").is_err());
}

#[test]
fn checkpoint_decide_unknown_run_errors() {
    let (_d, store) = store();
    assert!(checkpoint_decide(&store, "nope", true, None).is_err());
}

// ───── reject path holds station + routes feedback (per gated station) ─────

macro_rules! reject_holds_test {
    ($name:ident, $station:expr) => {
        #[test]
        fn $name() {
            let (_d, store) = fresh("r");
            at_phase(&store, "r", $station, StationPhase::Checkpoint);
            run_tick(&store, "r").expect("enter checkpoint");
            let res = checkpoint_decide(&store, "r", false, Some("rework".into())).expect("reject");
            assert_eq!(res.position.track, Track::Feedback);
            assert!(matches!(res.action, RunAction::FixFeedback { .. }));
            let s = store.read_state("r").unwrap().unwrap();
            assert_eq!(s.stations[$station].status, Status::Blocked);
            let cp = s.stations[$station].checkpoint.as_ref().unwrap();
            assert_eq!(cp.outcome, Some(CheckpointOutcome::Blocked));
        }
    };
}
reject_holds_test!(frame_reject_holds, "frame");
reject_holds_test!(specify_reject_holds, "specify");
reject_holds_test!(shape_reject_holds, "shape");
reject_holds_test!(harden_reject_holds, "harden");

// ───── Position serialization round-trips the action shape ─────

#[test]
fn position_serializes_with_track_and_action_tag() {
    let (_d, store) = fresh("r");
    let pos = derive_position(&store, "r").unwrap();
    let json = serde_json::to_value(&pos).unwrap();
    assert_eq!(json["track"], "run");
    assert_eq!(json["action"]["action"], "spec");
    assert_eq!(json["action"]["station"], "frame");
}

#[test]
fn manufacture_position_serializes_units_array() {
    let (_d, store) = fresh("r");
    at_phase(&store, "r", "frame", StationPhase::Manufacture);
    seed_unit(&store, "r", "frame", "u1", Status::Pending, &[]);
    seed_unit(&store, "r", "frame", "u2", Status::Pending, &[]);
    let pos = derive_position(&store, "r").unwrap();
    let json = serde_json::to_value(&pos).unwrap();
    assert_eq!(json["action"]["action"], "manufacture");
    assert!(json["action"]["units"].is_array());
    assert_eq!(json["action"]["units"].as_array().unwrap().len(), 2);
}

#[test]
fn checkpoint_position_serializes_kind() {
    let (_d, store) = fresh("r");
    at_phase(&store, "r", "build", StationPhase::Checkpoint);
    let pos = derive_position(&store, "r").unwrap();
    let json = serde_json::to_value(&pos).unwrap();
    assert_eq!(json["action"]["action"], "checkpoint");
    assert_eq!(json["action"]["kind"], "ask");
}

#[test]
fn sealed_position_serializes() {
    let (_d, store) = fresh("r");
    for station in STATIONS {
        seed_unit(&store, "r", station, &format!("{station}-u"), Status::Completed, &[]);
        for _ in 0..14 {
            let t = run_tick(&store, "r").unwrap();
            if matches!(&t.action, RunAction::UserGate { station: s, .. } if s == station) {
                checkpoint_decide(&store, "r", true, None).unwrap();
                continue;
            }
            if matches!(&t.action, RunAction::Checkpoint { station: s, .. } if s == station) {
                break;
            }
        }
        if is_gated(checkpoint_kind(station)) {
            checkpoint_decide(&store, "r", true, None).unwrap();
        }
    }
    sign_run_reviews(&store, "r");
    let pos = derive_position(&store, "r").unwrap();
    let json = serde_json::to_value(&pos).unwrap();
    assert_eq!(json["action"]["action"], "sealed");
    assert_eq!(json["action"]["run"], "r");
}

// ───── TickResult contains both position and action ─────

#[test]
fn tick_result_action_matches_position_when_present() {
    let (_d, store) = fresh("r");
    let t = run_tick(&store, "r").unwrap();
    assert_eq!(Some(&t.action), t.position.action.as_ref());
    assert_eq!(t.run, "r");
}

#[test]
fn tick_result_noop_when_position_null() {
    let (_d, store) = fresh("r");
    at_phase(&store, "r", "frame", StationPhase::Manufacture);
    // A dispatched, in-flight unit → null position (mid-wave noop). A dangling
    // dep would instead be a UnitsInvalid decomposition error.
    seed_unit(&store, "r", "frame", "u", Status::InProgress, &[]);
    let t = run_tick(&store, "r").unwrap();
    assert!(t.position.action.is_none());
    assert!(matches!(t.action, RunAction::Noop { .. }));
}

// ───── Active station pointer follows the cursor ─────

macro_rules! active_station_pointer_test {
    ($name:ident, $station:expr) => {
        #[test]
        fn $name() {
            let (_d, store) = fresh("r");
            advance_to_station(&store, "r", $station);
            run_tick(&store, "r").unwrap(); // spec tick stamps active_station
            let s = store.read_state("r").unwrap().unwrap();
            assert_eq!(s.active_station, $station);
        }
    };
}
active_station_pointer_test!(active_pointer_frame, "frame");
active_station_pointer_test!(active_pointer_specify, "specify");
active_station_pointer_test!(active_pointer_shape, "shape");
active_station_pointer_test!(active_pointer_build, "build");
active_station_pointer_test!(active_pointer_prove, "prove");
active_station_pointer_test!(active_pointer_harden, "harden");

// ───── Completed station is skipped by current_station ─────

#[test]
fn completed_frame_is_not_current_station() {
    let (_d, store) = fresh("r");
    advance_to_station(&store, "r", "specify");
    // frame is completed; the cursor sits on specify.
    let pos = derive_position(&store, "r").unwrap();
    assert_eq!(action_station(&pos.action.unwrap()), Some("specify"));
}

#[test]
fn unseeded_later_stations_default_to_spec_phase_in_derive() {
    let (_d, store) = fresh("r");
    // Complete frame in-state but DON'T seed specify, then derive.
    advance_to_station(&store, "r", "specify");
    // Manually wipe specify's seeded entry to simulate an unseeded station.
    let mut state = store.read_state("r").unwrap().unwrap();
    state.stations.remove("specify");
    store.write_state("r", &state).unwrap();
    // current_station now returns specify (no entry → not completed) and the
    // phase defaults to Spec.
    let pos = derive_position(&store, "r").unwrap();
    let a = pos.action.unwrap();
    assert_eq!(action_name(&a), "spec");
    assert_eq!(action_station(&a), Some("specify"));
}

// ───── Many-unit large wave stress (one wave) ─────

#[test]
fn manufacture_large_independent_wave() {
    let (_d, store) = fresh("r");
    at_phase(&store, "r", "frame", StationPhase::Manufacture);
    let slugs: Vec<String> = (0..25).map(|i| format!("u{i:02}")).collect();
    for s in &slugs {
        seed_unit(&store, "r", "frame", s, Status::Pending, &[]);
    }
    let mut units = manufacture_units(&store, "r");
    units.sort();
    let mut expected = slugs.clone();
    expected.sort();
    assert_eq!(units, expected);
}

#[test]
fn manufacture_large_chain_collapses_to_single_per_wave() {
    let (_d, store) = fresh("r");
    at_phase(&store, "r", "frame", StationPhase::Manufacture);
    // Linear chain c0 <- c1 <- ... <- c9.
    for i in 0..10 {
        let dep = if i == 0 { vec![] } else { vec![format!("c{}", i - 1)] };
        let dep_refs: Vec<&str> = dep.iter().map(|s| s.as_str()).collect();
        seed_unit(&store, "r", "frame", &format!("c{i}"), Status::Pending, &dep_refs);
    }
    // Each wave exposes exactly one ready unit.
    for i in 0..10 {
        let units = manufacture_units(&store, "r");
        assert_eq!(units, vec![format!("c{i}")], "wave {i}");
        complete_unit(&store, "r", &format!("c{i}"));
    }
    let pos = derive_position(&store, "r").unwrap();
    assert_eq!(action_name(&pos.action.unwrap()), "audit");
}

// ───── Mixed pending/completed: only pending-ready dispatched ─────

#[test]
fn manufacture_mixed_statuses_dispatches_only_ready_pending() {
    let (_d, store) = fresh("r");
    at_phase(&store, "r", "frame", StationPhase::Manufacture);
    seed_unit(&store, "r", "frame", "done", Status::Completed, &[]);
    seed_unit(&store, "r", "frame", "ready", Status::Pending, &[]);
    // `blocked` waits on the still-pending `ready` (a real edge — a dangling
    // dep would be a UnitsInvalid decomposition error).
    seed_unit(&store, "r", "frame", "blocked", Status::Pending, &["ready"]);
    let units = manufacture_units(&store, "r");
    assert_eq!(units, vec!["ready".to_string()]);
}

#[test]
fn manufacture_completes_when_last_pending_blocked_dep_resolves() {
    let (_d, store) = fresh("r");
    at_phase(&store, "r", "frame", StationPhase::Manufacture);
    seed_unit(&store, "r", "frame", "base", Status::Completed, &[]);
    seed_unit(&store, "r", "frame", "leaf", Status::Pending, &["base"]);
    // leaf is ready (base complete).
    assert_eq!(manufacture_units(&store, "r"), vec!["leaf".to_string()]);
    complete_unit(&store, "r", "leaf");
    let pos = derive_position(&store, "r").unwrap();
    assert_eq!(action_name(&pos.action.unwrap()), "audit");
}

// ───── Distinct runs are isolated on disk ─────

#[test]
fn two_runs_in_same_store_are_independent() {
    let (_d, store) = store();
    run_start(&store, "alpha", "software", None, Mode::Solo, "full").unwrap();
    run_start(&store, "beta", "software", None, Mode::Solo, "full").unwrap();
    // Solo holds alpha's Spec until sealed; seal so it can advance past it.
    darkrun_mcp::position::elaborate_seal(&store, "alpha", "frame").unwrap();
    // Advance alpha; beta stays at frame/spec.
    run_tick(&store, "alpha").unwrap();
    run_tick(&store, "alpha").unwrap();
    let a = store.read_state("alpha").unwrap().unwrap();
    let b = store.read_state("beta").unwrap().unwrap();
    // alpha: spec → review → pre-execution user gate (frame is interactive).
    assert_eq!(a.stations["frame"].phase, StationPhase::UserGate);
    assert_eq!(b.stations["frame"].phase, StationPhase::Spec);
    let pa = derive_position(&store, "alpha").unwrap();
    let pb = derive_position(&store, "beta").unwrap();
    assert_ne!(pa, pb);
}

// ───── Re-running checkpoint phase repeatedly (gated) re-enters cleanly ─────

#[test]
fn gated_checkpoint_re_tick_is_stable() {
    let (_d, store) = fresh("r");
    at_phase(&store, "r", "frame", StationPhase::Checkpoint);
    let t1 = run_tick(&store, "r").unwrap();
    let t2 = run_tick(&store, "r").unwrap();
    // Still on frame checkpoint both times (held).
    assert!(matches!(t1.action, RunAction::Checkpoint { .. }));
    assert!(matches!(t2.action, RunAction::Checkpoint { .. }));
    let s = store.read_state("r").unwrap().unwrap();
    assert_eq!(s.active_station, "frame");
    assert_ne!(s.stations["frame"].status, Status::Completed);
}

// ───── completed_at stamped on station completion ─────

macro_rules! completed_at_test {
    ($name:ident, $station:expr) => {
        #[test]
        fn $name() {
            let (_d, store) = fresh("r");
            at_phase(&store, "r", $station, StationPhase::Checkpoint);
            run_tick(&store, "r").unwrap(); // enter checkpoint
            if is_gated(checkpoint_kind($station)) {
                checkpoint_decide(&store, "r", true, None).unwrap();
            }
            let s = store.read_state("r").unwrap().unwrap();
            assert!(s.stations[$station].completed_at.is_some());
        }
    };
}
completed_at_test!(frame_completed_at, "frame");
completed_at_test!(specify_completed_at, "specify");
completed_at_test!(shape_completed_at, "shape");
completed_at_test!(build_completed_at, "build");
completed_at_test!(prove_completed_at, "prove");
completed_at_test!(harden_completed_at, "harden");

// ───── derive on a freshly started run yields frame/spec/run-track ─────

#[test]
fn fresh_run_derives_frame_spec_on_run_track() {
    let (_d, store) = fresh("r");
    let pos = derive_position(&store, "r").unwrap();
    assert_eq!(pos.track, Track::Run);
    let a = pos.action.unwrap();
    assert_eq!(action_name(&a), "spec");
    assert_eq!(action_station(&a), Some("frame"));
    match a {
        RunAction::Spec { run, .. } => assert_eq!(run, "r"),
        other => panic!("got {other:?}"),
    }
}

// ───── Helper invariant: Position equality is structural ─────

#[test]
fn position_equality_is_structural() {
    let p1 = Position {
        track: Track::Run,
        action: Some(RunAction::Reflect {
            run: "r".into(),
            station: "frame".into(),
        }),
    };
    let p2 = Position {
        track: Track::Run,
        action: Some(RunAction::Reflect {
            run: "r".into(),
            station: "frame".into(),
        }),
    };
    let p3 = Position {
        track: Track::Run,
        action: Some(RunAction::Reflect {
            run: "r".into(),
            station: "build".into(),
        }),
    };
    assert_eq!(p1, p2);
    assert_ne!(p1, p3);
}

// ───── Determinism across EVERY (station, phase) — full grid ─────

macro_rules! determinism_grid_test {
    ($name:ident, $station:expr, $phase:expr) => {
        #[test]
        fn $name() {
            let (_d, store) = fresh("r");
            at_phase(&store, "r", $station, $phase);
            if matches!($phase, StationPhase::Manufacture) {
                seed_unit(&store, "r", $station, "u", Status::Pending, &[]);
            }
            let a = derive_position(&store, "r").unwrap();
            let b = derive_position(&store, "r").unwrap();
            assert_eq!(a, b, "{} {:?} not deterministic", $station, $phase);
            // Serialized form is identical too.
            assert_eq!(
                serde_json::to_string(&a).unwrap(),
                serde_json::to_string(&b).unwrap()
            );
        }
    };
}
determinism_grid_test!(detgrid_specify_spec, "specify", StationPhase::Spec);
determinism_grid_test!(detgrid_specify_review, "specify", StationPhase::Review);
determinism_grid_test!(detgrid_specify_manufacture, "specify", StationPhase::Manufacture);
determinism_grid_test!(detgrid_specify_audit, "specify", StationPhase::Audit);
determinism_grid_test!(detgrid_specify_reflect, "specify", StationPhase::Reflect);
determinism_grid_test!(detgrid_specify_checkpoint, "specify", StationPhase::Checkpoint);
determinism_grid_test!(detgrid_shape_spec, "shape", StationPhase::Spec);
determinism_grid_test!(detgrid_shape_review, "shape", StationPhase::Review);
determinism_grid_test!(detgrid_shape_manufacture, "shape", StationPhase::Manufacture);
determinism_grid_test!(detgrid_shape_audit, "shape", StationPhase::Audit);
determinism_grid_test!(detgrid_shape_reflect, "shape", StationPhase::Reflect);
determinism_grid_test!(detgrid_shape_checkpoint, "shape", StationPhase::Checkpoint);
determinism_grid_test!(detgrid_build_spec, "build", StationPhase::Spec);
determinism_grid_test!(detgrid_build_review, "build", StationPhase::Review);
determinism_grid_test!(detgrid_build_audit, "build", StationPhase::Audit);
determinism_grid_test!(detgrid_build_reflect, "build", StationPhase::Reflect);
determinism_grid_test!(detgrid_prove_spec, "prove", StationPhase::Spec);
determinism_grid_test!(detgrid_prove_review, "prove", StationPhase::Review);
determinism_grid_test!(detgrid_prove_manufacture, "prove", StationPhase::Manufacture);
determinism_grid_test!(detgrid_prove_reflect, "prove", StationPhase::Reflect);
determinism_grid_test!(detgrid_prove_checkpoint, "prove", StationPhase::Checkpoint);
determinism_grid_test!(detgrid_harden_spec, "harden", StationPhase::Spec);
determinism_grid_test!(detgrid_harden_review, "harden", StationPhase::Review);
determinism_grid_test!(detgrid_harden_manufacture, "harden", StationPhase::Manufacture);
determinism_grid_test!(detgrid_harden_audit, "harden", StationPhase::Audit);
determinism_grid_test!(detgrid_harden_reflect, "harden", StationPhase::Reflect);

// ───── Idempotency: deriving N times never advances the cursor ─────

macro_rules! repeated_derive_stable_test {
    ($name:ident, $station:expr, $phase:expr) => {
        #[test]
        fn $name() {
            let (_d, store) = fresh("r");
            at_phase(&store, "r", $station, $phase);
            let before = store.read_state("r").unwrap().unwrap();
            for _ in 0..5 {
                let _ = derive_position(&store, "r").unwrap();
            }
            let after = store.read_state("r").unwrap().unwrap();
            assert_eq!(before.active_station, after.active_station);
            assert_eq!(
                before.stations[$station].phase,
                after.stations[$station].phase
            );
            assert_eq!(
                before.stations[$station].status,
                after.stations[$station].status
            );
        }
    };
}
repeated_derive_stable_test!(repeat_frame_spec, "frame", StationPhase::Spec);
repeated_derive_stable_test!(repeat_frame_review, "frame", StationPhase::Review);
repeated_derive_stable_test!(repeat_frame_audit, "frame", StationPhase::Audit);
repeated_derive_stable_test!(repeat_frame_reflect, "frame", StationPhase::Reflect);
repeated_derive_stable_test!(repeat_frame_checkpoint, "frame", StationPhase::Checkpoint);
repeated_derive_stable_test!(repeat_specify_review, "specify", StationPhase::Review);
repeated_derive_stable_test!(repeat_shape_audit, "shape", StationPhase::Audit);
repeated_derive_stable_test!(repeat_build_reflect, "build", StationPhase::Reflect);
repeated_derive_stable_test!(repeat_prove_review, "prove", StationPhase::Review);
repeated_derive_stable_test!(repeat_harden_spec, "harden", StationPhase::Spec);

// ───── Manufacture two-unit dependency per station (wave correctness) ─────

macro_rules! two_unit_wave_test {
    ($name:ident, $station:expr) => {
        #[test]
        fn $name() {
            let (_d, store) = fresh("r");
            at_phase(&store, "r", $station, StationPhase::Manufacture);
            seed_unit(&store, "r", $station, "first", Status::Pending, &[]);
            seed_unit(&store, "r", $station, "second", Status::Pending, &["first"]);
            // Wave 1: only `first`.
            assert_eq!(manufacture_units(&store, "r"), vec!["first".to_string()]);
            complete_unit(&store, "r", "first");
            // Wave 2: `second`.
            assert_eq!(manufacture_units(&store, "r"), vec!["second".to_string()]);
            complete_unit(&store, "r", "second");
            // Done → Audit.
            let pos = derive_position(&store, "r").unwrap();
            assert_eq!(action_name(&pos.action.unwrap()), "audit");
        }
    };
}
two_unit_wave_test!(two_wave_frame, "frame");
two_unit_wave_test!(two_wave_specify, "specify");
two_unit_wave_test!(two_wave_shape, "shape");
two_unit_wave_test!(two_wave_build, "build");
two_unit_wave_test!(two_wave_prove, "prove");
two_unit_wave_test!(two_wave_harden, "harden");

// ───── In-flight (non-ready) status keeps Manufacture from advancing ─────
// One case per station for each non-terminal status.

macro_rules! inflight_status_block_test {
    ($name:ident, $station:expr, $status:expr) => {
        #[test]
        fn $name() {
            let (_d, store) = fresh("r");
            at_phase(&store, "r", $station, StationPhase::Manufacture);
            seed_unit(&store, "r", $station, "x", $status, &[]);
            let pos = derive_position(&store, "r").unwrap();
            assert!(
                pos.action.is_none(),
                "{:?} on {} should hold mid-wave, got {:?}",
                $status,
                $station,
                pos.action
            );
        }
    };
}
inflight_status_block_test!(inflight_specify_active, "specify", Status::Active);
inflight_status_block_test!(inflight_specify_inprogress, "specify", Status::InProgress);
inflight_status_block_test!(inflight_specify_blocked, "specify", Status::Blocked);
inflight_status_block_test!(inflight_shape_active, "shape", Status::Active);
inflight_status_block_test!(inflight_shape_inprogress, "shape", Status::InProgress);
inflight_status_block_test!(inflight_shape_blocked, "shape", Status::Blocked);
inflight_status_block_test!(inflight_prove_active, "prove", Status::Active);
inflight_status_block_test!(inflight_prove_inprogress, "prove", Status::InProgress);
inflight_status_block_test!(inflight_prove_blocked, "prove", Status::Blocked);

// ───── Spec/Review/Audit/Reflect actions all carry the run slug ─────

macro_rules! action_run_slug_test {
    ($name:ident, $station:expr, $phase:expr, $expect:expr) => {
        #[test]
        fn $name() {
            let (_d, store) = fresh("slug-x");
            at_phase(&store, "slug-x", $station, $phase);
            if matches!($phase, StationPhase::Manufacture) {
                seed_unit(&store, "slug-x", $station, "u", Status::Pending, &[]);
            }
            let pos = derive_position(&store, "slug-x").unwrap();
            let a = pos.action.unwrap();
            assert_eq!(action_name(&a), $expect);
            let run = match &a {
                RunAction::Spec { run, .. }
                | RunAction::Review { run, .. }
                | RunAction::Manufacture { run, .. }
                | RunAction::Audit { run, .. }
                | RunAction::Reflect { run, .. }
                | RunAction::Checkpoint { run, .. }
                | RunAction::ExternalReviewRequested { run, .. } => run.as_str(),
                other => panic!("unexpected {other:?}"),
            };
            assert_eq!(run, "slug-x");
        }
    };
}
action_run_slug_test!(slug_frame_spec, "frame", StationPhase::Spec, "spec");
action_run_slug_test!(slug_frame_review, "frame", StationPhase::Review, "review");
action_run_slug_test!(slug_frame_manufacture, "frame", StationPhase::Manufacture, "manufacture");
action_run_slug_test!(slug_frame_audit, "frame", StationPhase::Audit, "audit");
action_run_slug_test!(slug_frame_reflect, "frame", StationPhase::Reflect, "reflect");
action_run_slug_test!(slug_frame_checkpoint, "frame", StationPhase::Checkpoint, "checkpoint");
action_run_slug_test!(slug_harden_spec, "harden", StationPhase::Spec, "spec");
action_run_slug_test!(slug_harden_review, "harden", StationPhase::Review, "review");
action_run_slug_test!(slug_harden_audit, "harden", StationPhase::Audit, "audit");
action_run_slug_test!(slug_harden_reflect, "harden", StationPhase::Reflect, "reflect");
action_run_slug_test!(slug_harden_checkpoint, "harden", StationPhase::Checkpoint, "checkpoint");

// ───── Serialization tag per action variant (snake_case) ─────

macro_rules! action_serializes_tag_test {
    ($name:ident, $station:expr, $phase:expr, $tag:expr) => {
        #[test]
        fn $name() {
            let (_d, store) = fresh("r");
            at_phase(&store, "r", $station, $phase);
            if matches!($phase, StationPhase::Manufacture) {
                seed_unit(&store, "r", $station, "u", Status::Pending, &[]);
            }
            let pos = derive_position(&store, "r").unwrap();
            let json = serde_json::to_value(&pos).unwrap();
            assert_eq!(json["action"]["action"], $tag);
            assert_eq!(json["action"]["station"], $station);
        }
    };
}
action_serializes_tag_test!(ser_specify_spec, "specify", StationPhase::Spec, "spec");
action_serializes_tag_test!(ser_specify_review, "specify", StationPhase::Review, "review");
action_serializes_tag_test!(ser_shape_audit, "shape", StationPhase::Audit, "audit");
action_serializes_tag_test!(ser_build_reflect, "build", StationPhase::Reflect, "reflect");
action_serializes_tag_test!(ser_prove_manufacture, "prove", StationPhase::Manufacture, "manufacture");
action_serializes_tag_test!(ser_harden_checkpoint, "harden", StationPhase::Checkpoint, "checkpoint");

// ───── Tick advances phase exactly one step per call (Spec→Review→…) ─────

macro_rules! single_step_tick_test {
    ($name:ident, $station:expr, $from:expr, $to:expr) => {
        #[test]
        fn $name() {
            let (_d, store) = fresh("r");
            at_phase(&store, "r", $station, $from);
            // Manufacture needs a wave-ready unit to emit a Manufacture action
            // (otherwise it falls back to Spec and re-stamps Review).
            if matches!($from, StationPhase::Manufacture) {
                seed_unit(&store, "r", $station, "u", Status::Pending, &[]);
            }
            run_tick(&store, "r").unwrap();
            assert_eq!(store.read_state("r").unwrap().unwrap().stations[$station].phase, $to);
        }
    };
}
// Manufacture with a ready wave holds in Manufacture.
single_step_tick_test!(step_frame_mfg_holds, "frame", StationPhase::Manufacture, StationPhase::Manufacture);
single_step_tick_test!(step_specify_mfg_holds, "specify", StationPhase::Manufacture, StationPhase::Manufacture);
single_step_tick_test!(step_shape_mfg_holds, "shape", StationPhase::Manufacture, StationPhase::Manufacture);
single_step_tick_test!(step_build_mfg_holds, "build", StationPhase::Manufacture, StationPhase::Manufacture);
single_step_tick_test!(step_prove_mfg_holds, "prove", StationPhase::Manufacture, StationPhase::Manufacture);
single_step_tick_test!(step_harden_mfg_holds, "harden", StationPhase::Manufacture, StationPhase::Manufacture);

// ───── checkpoint_decide reject files feedback whose body is dispatched ─────

macro_rules! reject_body_dispatched_test {
    ($name:ident, $station:expr) => {
        #[test]
        fn $name() {
            let (_d, store) = fresh("r");
            at_phase(&store, "r", $station, StationPhase::Checkpoint);
            run_tick(&store, "r").unwrap();
            let body = format!("rework for {}", $station);
            let res = checkpoint_decide(&store, "r", false, Some(body.clone())).unwrap();
            match &res.action {
                RunAction::FixFeedback { station, .. } => assert_eq!(station, $station),
                other => panic!("expected FixFeedback, got {other:?}"),
            }
            // The feedback persists with the rework body (the reject path writes
            // a minimal status:pending doc; the station field is left blank).
            let all = darkrun_mcp::feedback::list(&store, "r").unwrap();
            assert_eq!(all.len(), 1);
            assert!(all[0].body.contains(&body));
            assert_eq!(all[0].status, darkrun_core::domain::FeedbackStatus::Pending);
        }
    };
}
reject_body_dispatched_test!(reject_body_frame, "frame");
reject_body_dispatched_test!(reject_body_specify, "specify");
reject_body_dispatched_test!(reject_body_shape, "shape");
reject_body_dispatched_test!(reject_body_harden, "harden");

// ───── checkpoint entered_at + outcome lifecycle per gated station ─────

macro_rules! checkpoint_lifecycle_test {
    ($name:ident, $station:expr) => {
        #[test]
        fn $name() {
            let (_d, store) = fresh("r");
            at_phase(&store, "r", $station, StationPhase::Checkpoint);
            // Enter the gate.
            run_tick(&store, "r").unwrap();
            let s = store.read_state("r").unwrap().unwrap();
            let cp = s.stations[$station].checkpoint.as_ref().unwrap();
            assert!(cp.entered_at.is_some(), "entered_at stamped");
            assert!(cp.outcome.is_none(), "no outcome until decided");
            // Approve → Advanced outcome.
            checkpoint_decide(&store, "r", true, None).unwrap();
            let s = store.read_state("r").unwrap().unwrap();
            let cp = s.stations[$station].checkpoint.as_ref().unwrap();
            assert_eq!(cp.outcome, Some(CheckpointOutcome::Advanced));
        }
    };
}
checkpoint_lifecycle_test!(cp_lifecycle_frame, "frame");
checkpoint_lifecycle_test!(cp_lifecycle_specify, "specify");
checkpoint_lifecycle_test!(cp_lifecycle_shape, "shape");
checkpoint_lifecycle_test!(cp_lifecycle_harden, "harden");

// ───── Whole-run phase-name sequence is the canonical six per station ─────

macro_rules! station_phase_sequence_test {
    ($name:ident, $station:expr) => {
        #[test]
        fn $name() {
            let (_d, store) = fresh("r");
            advance_to_station(&store, "r", $station);
            let mut seen = Vec::new();
            // Spec — solo holds it until the elaboration is sealed.
            seen.push(action_name(&run_tick(&store, "r").unwrap().action));
            darkrun_mcp::position::elaborate_seal(&store, "r", $station).unwrap();
            run_tick(&store, "r").unwrap(); // advance past the sealed Spec
            // Review.
            seen.push(action_name(&run_tick(&store, "r").unwrap().action));
            // Manufacture (after decomposing a unit). An interactive station
            // first holds at the pre-execution operator gate; clear it so the
            // wave releases. An auto-gated station skips straight to Manufacture.
            seed_unit(&store, "r", $station, "u1", Status::Pending, &[]);
            let mfg = run_tick(&store, "r").unwrap();
            let mfg = if action_name(&mfg.action) == "user_gate" {
                checkpoint_decide(&store, "r", true, None).unwrap();
                run_tick(&store, "r").unwrap()
            } else {
                mfg
            };
            seen.push(action_name(&mfg.action));
            complete_unit(&store, "r", "u1");
            // Audit (folds in the old tests phase).
            seen.push(action_name(&run_tick(&store, "r").unwrap().action));
            // Reflect.
            seen.push(action_name(&run_tick(&store, "r").unwrap().action));
            // The gate (local checkpoint, or external review for harden).
            let gate = action_name(&run_tick(&store, "r").unwrap().action);
            assert_eq!(
                seen,
                vec!["spec", "review", "manufacture", "audit", "reflect"]
            );
            assert!(
                gate == "checkpoint" || gate == "external_review_requested",
                "expected a gate action, got {gate}"
            );
        }
    };
}
station_phase_sequence_test!(seq_frame, "frame");
station_phase_sequence_test!(seq_specify, "specify");
station_phase_sequence_test!(seq_shape, "shape");
station_phase_sequence_test!(seq_build, "build");
station_phase_sequence_test!(seq_prove, "prove");
station_phase_sequence_test!(seq_harden, "harden");

// ───── Manufacture re-derive after partial completion drops finished units ─────

#[test]
fn manufacture_rederive_drops_completed_units_from_wave() {
    let (_d, store) = fresh("r");
    at_phase(&store, "r", "frame", StationPhase::Manufacture);
    seed_unit(&store, "r", "frame", "p", Status::Pending, &[]);
    seed_unit(&store, "r", "frame", "q", Status::Pending, &[]);
    let mut w1 = manufacture_units(&store, "r");
    w1.sort();
    assert_eq!(w1, vec!["p".to_string(), "q".to_string()]);
    // Complete p; q remains the only wave member.
    complete_unit(&store, "r", "p");
    assert_eq!(manufacture_units(&store, "r"), vec!["q".to_string()]);
}

// ───── run_start frontmatter and active_station ─────

#[test]
fn run_start_sets_active_station_and_status() {
    let (_d, store) = store();
    let run = run_start(&store, "r", "software", Some("Title".into()), Mode::Solo, "full").unwrap();
    assert_eq!(run.frontmatter.active_station, "frame");
    assert_eq!(run.frontmatter.factory, "software");
    assert_eq!(run.frontmatter.mode, Mode::Solo);
    assert_eq!(run.frontmatter.status, Status::Active);
    assert!(run.frontmatter.started_at.is_some());
    assert_eq!(run.title, "Title");
}

#[test]
fn run_start_unknown_factory_errors() {
    let (_d, store) = store();
    assert!(run_start(&store, "r", "nonexistent", None, Mode::Solo, "full").is_err());
}

#[test]
fn run_start_title_defaults_to_slug() {
    let (_d, store) = store();
    let run = run_start(&store, "my-slug", "software", None, Mode::Solo, "full").unwrap();
    assert_eq!(run.title, "my-slug");
}

// ───── current_station skips ALL completed prefixes ─────

#[test]
fn current_station_skips_multiple_completed() {
    let (_d, store) = fresh("r");
    advance_to_station(&store, "r", "build");
    // frame, specify, shape are completed; build is current.
    let s = store.read_state("r").unwrap().unwrap();
    assert_eq!(s.stations["frame"].status, Status::Completed);
    assert_eq!(s.stations["specify"].status, Status::Completed);
    assert_eq!(s.stations["shape"].status, Status::Completed);
    let pos = derive_position(&store, "r").unwrap();
    assert_eq!(action_station(&pos.action.unwrap()), Some("build"));
}

// ───── started_at stamped on each station's first Spec tick ─────

macro_rules! started_at_stamped_test {
    ($name:ident, $station:expr) => {
        #[test]
        fn $name() {
            let (_d, store) = fresh("r");
            advance_to_station(&store, "r", $station);
            // Before the Spec tick, started_at is unset on a fresh station.
            assert!(store.read_state("r").unwrap().unwrap().stations[$station]
                .started_at
                .is_none());
            run_tick(&store, "r").unwrap(); // Spec tick.
            let st = store.read_state("r").unwrap().unwrap();
            assert!(st.stations[$station].started_at.is_some());
            assert_eq!(st.stations[$station].status, Status::InProgress);
        }
    };
}
started_at_stamped_test!(started_at_frame, "frame");
started_at_stamped_test!(started_at_specify, "specify");
started_at_stamped_test!(started_at_shape, "shape");
started_at_stamped_test!(started_at_build, "build");
started_at_stamped_test!(started_at_prove, "prove");
started_at_stamped_test!(started_at_harden, "harden");

// ───── Manufacture worker carried on a MULTI-unit wave per station ─────

macro_rules! worker_on_multi_wave_test {
    ($name:ident, $station:expr, $worker:expr) => {
        #[test]
        fn $name() {
            let (_d, store) = fresh("r");
            at_phase(&store, "r", $station, StationPhase::Manufacture);
            seed_unit(&store, "r", $station, "a", Status::Pending, &[]);
            seed_unit(&store, "r", $station, "b", Status::Pending, &[]);
            let pos = derive_position(&store, "r").unwrap();
            match pos.action.unwrap() {
                RunAction::Manufacture { worker, units, .. } => {
                    assert_eq!(worker, $worker);
                    assert_eq!(units.len(), 2);
                }
                other => panic!("got {other:?}"),
            }
        }
    };
}
worker_on_multi_wave_test!(multi_worker_frame, "frame", "framer");
worker_on_multi_wave_test!(multi_worker_specify, "specify", "spec_writer");
worker_on_multi_wave_test!(multi_worker_shape, "shape", "designer");
worker_on_multi_wave_test!(multi_worker_build, "build", "test_author");
worker_on_multi_wave_test!(multi_worker_prove, "prove", "verifier");
worker_on_multi_wave_test!(multi_worker_harden, "harden", "hardener");

// ───── Checkpoint kind survives a re-derive (no mutation) per station ─────

macro_rules! checkpoint_kind_stable_test {
    ($name:ident, $station:expr, $kind:expr) => {
        #[test]
        fn $name() {
            let (_d, store) = fresh("r");
            at_phase(&store, "r", $station, StationPhase::Checkpoint);
            for _ in 0..3 {
                let pos = derive_position(&store, "r").unwrap();
                match pos.action.unwrap() {
                    RunAction::Checkpoint { kind, .. } => assert_eq!(kind, $kind),
                    other => panic!("got {other:?}"),
                }
            }
        }
    };
}
checkpoint_kind_stable_test!(cp_stable_frame, "frame", CheckpointKind::Ask);
checkpoint_kind_stable_test!(cp_stable_specify, "specify", CheckpointKind::Ask);
checkpoint_kind_stable_test!(cp_stable_shape, "shape", CheckpointKind::Ask);
checkpoint_kind_stable_test!(cp_stable_build, "build", CheckpointKind::Ask);
checkpoint_kind_stable_test!(cp_stable_prove, "prove", CheckpointKind::Ask);

// harden's local `ask` gate re-derives stably as a Checkpoint.
#[test]
fn cp_stable_harden() {
    let (_d, store) = fresh("r");
    at_phase(&store, "r", "harden", StationPhase::Checkpoint);
    for _ in 0..3 {
        let pos = derive_position(&store, "r").unwrap();
        match pos.action.unwrap() {
            RunAction::Checkpoint { station, kind, .. } => {
                assert_eq!(station, "harden");
                assert_eq!(kind, CheckpointKind::Ask);
            }
            other => panic!("got {other:?}"),
        }
    }
}

// ───── Empty-units Manufacture re-stamps Review (Spec fallback loop) ─────
// At Manufacture with no units, derive yields Spec; a tick re-runs Spec's
// advancement, stamping Review — proving the station re-specs rather than
// silently advancing.

#[test]
fn manufacture_empty_tick_restamps_review() {
    let (_d, store) = fresh("r");
    at_phase(&store, "r", "frame", StationPhase::Manufacture);
    let t = run_tick(&store, "r").unwrap();
    assert_eq!(action_name(&t.action), "spec");
    assert_eq!(
        store.read_state("r").unwrap().unwrap().stations["frame"].phase,
        StationPhase::Review
    );
}
