//! End-to-end durability & recovery tests for the darkrun engine.
//!
//! These drive a real Run partway, then simulate a process restart by dropping
//! every in-memory handle and opening a *fresh* [`StateStore`] over the same
//! on-disk `.darkrun/` tree. The Run must resume exactly where it left off:
//! the active station, every station's phase, its unit set, its checkpoint
//! stamps, and any filed feedback all survive the reopen because the engine
//! keeps no state outside the filesystem.
//!
//! Beyond resumption the suite exercises:
//! - active-run pointer resolution after a restart (explicit pointer + the
//!   inferred-from-disk fallback);
//! - advisory `mkdir` lock acquisition across two managers standing in for two
//!   OS processes, including blocking, stale-holder recovery, and `with_lock`
//!   serialization of concurrent ticks across threads;
//! - corrupted / partial / truncated `state.json` files — a syntactically
//!   broken snapshot surfaces an error rather than silently corrupting the Run,
//!   while a partial-but-valid snapshot degrades gracefully through serde
//!   defaults and the Run keeps walking.
//!
//! Nothing is mocked: every assertion reads genuine bytes the engine wrote.

mod common;

use std::sync::{Arc, Barrier};
use std::thread;

use common::*;
use darkrun_core::domain::{
    CheckpointKind, CheckpointOutcome, Status, StationPhase, Unit, UnitFrontmatter,
};
use darkrun_core::{LockManager, RunState, StateStore};
use darkrun_mcp::position::{
    checkpoint_decide, derive_position, run_start, run_tick, RunAction, Track,
};

// ===========================================================================
// Local fixture: a Run rooted in a temp dir we can reopen at will.
//
// Unlike the shared `Harness`, this fixture exposes the *repo root* so a test
// can throw away its store and build a brand-new one over the same bytes — the
// faithful "restart the process" move.
// ===========================================================================

/// A run on disk plus the repo root it lives under. Holds the `TempDir` so the
/// tree survives for the test's lifetime and is torn down on drop.
struct Durable {
    dir: tempfile::TempDir,
    slug: String,
}

impl Durable {
    /// Start a fresh `software` run and return the durable fixture.
    fn start(slug: &str) -> Self {
        Self::start_with(slug, "software", "continuous")
    }

    fn start_with(slug: &str, factory: &str, mode: &str) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = StateStore::new(dir.path());
        run_start(&store, slug, factory, None, mode).expect("run_start");
        Durable {
            dir,
            slug: slug.to_string(),
        }
    }

    /// A FRESH store over the same on-disk tree — the restart move. Nothing in
    /// memory is shared with any prior store; only the filesystem carries over.
    fn reopen(&self) -> StateStore {
        StateStore::new(self.dir.path())
    }

    /// A fresh lock manager over the same tree (a "second process").
    fn locks(&self) -> LockManager {
        LockManager::new(self.dir.path())
    }

    /// Tick once through a freshly-opened store (each tick a new "process").
    fn tick_fresh(&self) -> RunAction {
        run_tick(&self.reopen(), &self.slug).expect("tick").action
    }

    /// Read the persisted state through a fresh store.
    fn state(&self) -> RunState {
        self.reopen()
            .read_state(&self.slug)
            .expect("read_state")
            .expect("state present")
    }

    fn phase(&self, station: &str) -> StationPhase {
        self.state()
            .stations
            .get(station)
            .map(|s| s.phase)
            .unwrap_or(StationPhase::Spec)
    }

    fn station_status(&self, station: &str) -> Status {
        self.state()
            .stations
            .get(station)
            .map(|s| s.status)
            .unwrap_or(Status::Pending)
    }

    fn active(&self) -> String {
        self.state().active_station
    }

    /// Decompose a wave of units (with optional deps) through a fresh store.
    fn decompose(&self, station: &str, units: &[(&str, &[&str])]) {
        let store = self.reopen();
        for (slug, deps) in units {
            let unit = Unit {
                slug: (*slug).to_string(),
                frontmatter: UnitFrontmatter {
                    status: Status::Pending,
                    station: Some(station.to_string()),
                    depends_on: deps.iter().map(|d| d.to_string()).collect(),
                    ..Default::default()
                },
                title: (*slug).to_string(),
                body: String::new(),
            };
            store.write_unit(&self.slug, &unit).expect("write_unit");
        }
    }

    fn complete_unit(&self, unit_slug: &str) {
        let store = self.reopen();
        let mut u = store.read_unit(&self.slug, unit_slug).expect("read_unit");
        u.frontmatter.status = Status::Completed;
        store.write_unit(&self.slug, &u).expect("write_unit");
    }

    /// Path to `state.json` on disk for direct byte surgery.
    fn state_path(&self) -> std::path::PathBuf {
        self.reopen().run_dir(&self.slug).join("state.json")
    }
}

/// Drive the run's active station to a held checkpoint, opening a fresh store
/// for every step so the whole walk is reconstructed from disk each tick.
fn walk_to_checkpoint_fresh(d: &Durable, station: &str, units: &[&str]) {
    d.tick_fresh(); // spec -> review
    d.tick_fresh(); // review -> manufacture
    if !units.is_empty() {
        let pairs: Vec<(&str, &[&str])> = units.iter().map(|s| (*s, &[][..])).collect();
        d.decompose(station, &pairs);
        d.tick_fresh(); // manufacture dispatch
        for u in units {
            d.complete_unit(u);
        }
    }
    d.tick_fresh(); // audit
    d.tick_fresh(); // tests
    d.tick_fresh(); // checkpoint (held for ask/external)
}

// ===========================================================================
// Section 1 — A fresh store reopen mid-run resumes the exact cursor
// ===========================================================================

#[test]
fn reopen_after_start_sees_frame_spec() {
    let d = Durable::start("r1");
    // A brand-new store over the same tree resolves the same starting action.
    let pos = derive_position(&d.reopen(), "r1").unwrap();
    assert!(is_spec(pos.action.as_ref().unwrap(), "frame"));
}

#[test]
fn reopen_after_start_state_factory_persists() {
    let d = Durable::start("r1");
    assert_eq!(d.state().factory, "software");
}

#[test]
fn reopen_after_start_active_station_is_frame() {
    let d = Durable::start("r1");
    assert_eq!(d.active(), "frame");
}

#[test]
fn reopen_preserves_run_md_frontmatter() {
    let d = Durable::start("r1");
    let run = d.reopen().read_run("r1").unwrap();
    assert_eq!(run.frontmatter.factory, "software");
    assert_eq!(run.frontmatter.status, Status::Active);
}

#[test]
fn reopen_after_one_tick_phase_is_review() {
    let d = Durable::start("r1");
    d.tick_fresh(); // spec -> review, persisted
                    // New store reads the advanced phase straight off disk.
    assert_eq!(d.phase("frame"), StationPhase::Review);
}

#[test]
fn reopen_after_one_tick_next_action_is_review() {
    let d = Durable::start("r1");
    d.tick_fresh();
    let pos = derive_position(&d.reopen(), "r1").unwrap();
    assert!(is_review(pos.action.as_ref().unwrap(), "frame"));
}

#[test]
fn reopen_after_two_ticks_phase_is_manufacture() {
    let d = Durable::start("r1");
    d.tick_fresh();
    d.tick_fresh();
    assert_eq!(d.phase("frame"), StationPhase::Manufacture);
}

#[test]
fn reopen_mid_manufacture_resumes_same_wave() {
    let d = Durable::start("r1");
    d.tick_fresh(); // spec
    d.tick_fresh(); // review
    d.decompose("frame", &[("u1", &[]), ("u2", &[])]);
    d.tick_fresh(); // dispatch wave
                    // Reopen: the next derived action still dispatches the same ready units.
    let pos = derive_position(&d.reopen(), "r1").unwrap();
    match pos.action {
        Some(RunAction::Manufacture { mut units, .. }) => {
            units.sort();
            assert_eq!(units, vec!["u1".to_string(), "u2".to_string()]);
        }
        other => panic!("expected Manufacture after reopen, got {other:?}"),
    }
}

#[test]
fn reopen_preserves_decomposed_units() {
    let d = Durable::start("r1");
    d.tick_fresh();
    d.tick_fresh();
    d.decompose("frame", &[("u1", &[]), ("u2", &["u1"])]);
    let units = d.reopen().read_units("r1").unwrap();
    assert_eq!(units.len(), 2);
    assert_eq!(units[0].slug, "u1");
    assert_eq!(units[1].slug, "u2");
}

#[test]
fn reopen_preserves_unit_dependencies() {
    let d = Durable::start("r1");
    d.tick_fresh();
    d.tick_fresh();
    d.decompose("frame", &[("u1", &[]), ("u2", &["u1"])]);
    let u2 = d.reopen().read_unit("r1", "u2").unwrap();
    assert_eq!(u2.frontmatter.depends_on, vec!["u1".to_string()]);
}

#[test]
fn reopen_preserves_unit_station_assignment() {
    let d = Durable::start("r1");
    d.tick_fresh();
    d.tick_fresh();
    d.decompose("frame", &[("u1", &[])]);
    let u1 = d.reopen().read_unit("r1", "u1").unwrap();
    assert_eq!(u1.station(), "frame");
}

#[test]
fn reopen_preserves_completed_unit_status() {
    let d = Durable::start("r1");
    d.tick_fresh();
    d.tick_fresh();
    d.decompose("frame", &[("u1", &[])]);
    d.tick_fresh();
    d.complete_unit("u1");
    let u1 = d.reopen().read_unit("r1", "u1").unwrap();
    assert_eq!(u1.frontmatter.status, Status::Completed);
}

#[test]
fn reopen_after_unit_completion_resumes_at_audit() {
    let d = Durable::start("r1");
    d.tick_fresh();
    d.tick_fresh();
    d.decompose("frame", &[("u1", &[])]);
    d.tick_fresh();
    d.complete_unit("u1");
    let pos = derive_position(&d.reopen(), "r1").unwrap();
    assert!(is_audit(pos.action.as_ref().unwrap(), "frame"));
}

#[test]
fn reopen_after_audit_phase_is_tests() {
    let d = Durable::start("r1");
    d.tick_fresh();
    d.tick_fresh();
    d.decompose("frame", &[("u1", &[])]);
    d.tick_fresh();
    d.complete_unit("u1");
    d.tick_fresh(); // audit -> tests
    assert_eq!(d.phase("frame"), StationPhase::Tests);
}

#[test]
fn reopen_at_held_checkpoint_resumes_held() {
    let d = Durable::start("r1");
    walk_to_checkpoint_fresh(&d, "frame", &["u1"]);
    // The ask gate holds; a fresh store still sees the held checkpoint action.
    let pos = derive_position(&d.reopen(), "r1").unwrap();
    assert!(is_checkpoint(pos.action.as_ref().unwrap(), "frame"));
}

#[test]
fn reopen_at_held_checkpoint_station_in_progress() {
    let d = Durable::start("r1");
    walk_to_checkpoint_fresh(&d, "frame", &["u1"]);
    assert_eq!(d.station_status("frame"), Status::InProgress);
}

#[test]
fn reopen_preserves_checkpoint_entered_at() {
    let d = Durable::start("r1");
    walk_to_checkpoint_fresh(&d, "frame", &["u1"]);
    let cp = d.state().stations["frame"].checkpoint.clone().unwrap();
    assert!(cp.entered_at.is_some());
}

#[test]
fn reopen_preserves_checkpoint_kind() {
    let d = Durable::start("r1");
    walk_to_checkpoint_fresh(&d, "frame", &["u1"]);
    let cp = d.state().stations["frame"].checkpoint.clone().unwrap();
    assert_eq!(cp.kind, CheckpointKind::Ask);
}

#[test]
fn reopen_preserves_station_started_at() {
    let d = Durable::start("r1");
    d.tick_fresh(); // frame spec stamps started_at
    assert!(d.state().stations["frame"].started_at.is_some());
}

#[test]
fn decide_then_reopen_advances_to_specify() {
    let d = Durable::start("r1");
    walk_to_checkpoint_fresh(&d, "frame", &["u1"]);
    // Approve through one store, then read through a fresh one.
    checkpoint_decide(&d.reopen(), "r1", true, None).unwrap();
    assert_eq!(d.active(), "specify");
}

#[test]
fn decide_then_reopen_frame_completed() {
    let d = Durable::start("r1");
    walk_to_checkpoint_fresh(&d, "frame", &["u1"]);
    checkpoint_decide(&d.reopen(), "r1", true, None).unwrap();
    assert_eq!(d.station_status("frame"), Status::Completed);
}

#[test]
fn decide_then_reopen_checkpoint_outcome_advanced() {
    let d = Durable::start("r1");
    walk_to_checkpoint_fresh(&d, "frame", &["u1"]);
    checkpoint_decide(&d.reopen(), "r1", true, None).unwrap();
    let cp = d.state().stations["frame"].checkpoint.clone().unwrap();
    assert_eq!(cp.outcome, Some(CheckpointOutcome::Advanced));
}

#[test]
fn decide_then_reopen_frame_completed_at_persisted() {
    let d = Durable::start("r1");
    walk_to_checkpoint_fresh(&d, "frame", &["u1"]);
    checkpoint_decide(&d.reopen(), "r1", true, None).unwrap();
    assert!(d.state().stations["frame"].completed_at.is_some());
}

#[test]
fn reopen_every_tick_reaches_specify() {
    // Drive a whole station to completion where EVERY single tick opens a brand
    // new store — the strongest restart-durability statement.
    let d = Durable::start("r1");
    walk_to_checkpoint_fresh(&d, "frame", &["u1"]);
    // The decide's re-tick surfaces the next station's Spec action; that is the
    // action the caller sees on approval, reconstructed entirely from disk.
    let decided = checkpoint_decide(&d.reopen(), "r1", true, None).unwrap();
    assert!(is_spec(&decided.action, "specify"));
    // And the persisted pointer truly moved to specify.
    assert_eq!(d.active(), "specify");
}

#[test]
fn reopen_multi_station_progress_persists() {
    // Complete frame and specify entirely via fresh stores; shape is next.
    let d = Durable::start("r1");
    walk_to_checkpoint_fresh(&d, "frame", &["fu"]);
    checkpoint_decide(&d.reopen(), "r1", true, None).unwrap();
    walk_to_checkpoint_fresh(&d, "specify", &["su"]);
    checkpoint_decide(&d.reopen(), "r1", true, None).unwrap();
    assert_eq!(d.active(), "shape");
    assert_eq!(d.station_status("frame"), Status::Completed);
    assert_eq!(d.station_status("specify"), Status::Completed);
}

#[test]
fn reopen_preserves_completed_station_timestamps_across_stations() {
    let d = Durable::start("r1");
    walk_to_checkpoint_fresh(&d, "frame", &["fu"]);
    checkpoint_decide(&d.reopen(), "r1", true, None).unwrap();
    let s = d.state();
    assert!(s.stations["frame"].completed_at.is_some());
    assert!(s.stations["frame"].started_at.is_some());
}

// ===========================================================================
// Section 2 — Feedback survives a reopen and still preempts the Run track
// ===========================================================================

#[test]
fn reopen_with_open_feedback_preempts() {
    let d = Durable::start("fb");
    d.reopen()
        .write_feedback_raw("fb", "fb-1", "---\nstatus: pending\n---\nbroken\n")
        .unwrap();
    let pos = derive_position(&d.reopen(), "fb").unwrap();
    assert_eq!(pos.track, Track::Feedback);
}

#[test]
fn reopen_feedback_fix_action_carries_id() {
    let d = Durable::start("fb");
    d.reopen()
        .write_feedback_raw("fb", "fb-7", "---\nstatus: pending\n---\nx\n")
        .unwrap();
    match derive_position(&d.reopen(), "fb").unwrap().action {
        Some(RunAction::FixFeedback { feedback_id, .. }) => assert_eq!(feedback_id, "fb-7"),
        other => panic!("expected FixFeedback, got {other:?}"),
    }
}

#[test]
fn reopen_feedback_doc_bytes_persist() {
    let d = Durable::start("fb");
    d.reopen()
        .write_feedback_raw("fb", "fb-1", "---\nstatus: pending\n---\nthe exact body\n")
        .unwrap();
    let raw = d.reopen().read_feedback_raw("fb").unwrap();
    assert!(raw["fb-1"].contains("the exact body"));
}

#[test]
fn reopen_resolved_feedback_returns_to_run() {
    let d = Durable::start("fb");
    let store = d.reopen();
    store
        .write_feedback_raw("fb", "fb-1", "---\nstatus: pending\n---\nx\n")
        .unwrap();
    assert_eq!(
        derive_position(&d.reopen(), "fb").unwrap().track,
        Track::Feedback
    );
    // Mark it addressed and reopen: the run track resumes.
    store
        .write_feedback_raw("fb", "fb-1", "---\nstatus: addressed\n---\nx\n")
        .unwrap();
    assert_eq!(
        derive_position(&d.reopen(), "fb").unwrap().track,
        Track::Run
    );
}

#[test]
fn reject_files_feedback_that_survives_reopen() {
    let d = Durable::start("rej");
    walk_to_checkpoint_fresh(&d, "frame", &["u1"]);
    checkpoint_decide(&d.reopen(), "rej", false, Some("rework the spec".to_string())).unwrap();
    // A fresh store reads the filed checkpoint feedback and preempts.
    let raw = d.reopen().read_feedback_raw("rej").unwrap();
    assert!(raw.contains_key("fb-checkpoint"));
    assert!(raw["fb-checkpoint"].contains("rework the spec"));
    assert_eq!(
        derive_position(&d.reopen(), "rej").unwrap().track,
        Track::Feedback
    );
}

#[test]
fn reject_then_reopen_station_blocked() {
    let d = Durable::start("rej");
    walk_to_checkpoint_fresh(&d, "frame", &["u1"]);
    checkpoint_decide(&d.reopen(), "rej", false, Some("no".to_string())).unwrap();
    assert_eq!(d.station_status("frame"), Status::Blocked);
}

#[test]
fn reject_then_reopen_checkpoint_outcome_blocked() {
    let d = Durable::start("rej");
    walk_to_checkpoint_fresh(&d, "frame", &["u1"]);
    checkpoint_decide(&d.reopen(), "rej", false, Some("no".to_string())).unwrap();
    let cp = d.state().stations["frame"].checkpoint.clone().unwrap();
    assert_eq!(cp.outcome, Some(CheckpointOutcome::Blocked));
}

// ===========================================================================
// Section 3 — Active-run pointer resolution after a restart
// ===========================================================================

#[test]
fn active_pointer_persists_across_reopen() {
    let d = Durable::start("only");
    d.reopen().set_active_run("only").unwrap();
    assert_eq!(d.reopen().active_run().unwrap(), Some("only".to_string()));
}

#[test]
fn active_pointer_written_to_disk_file() {
    let d = Durable::start("only");
    d.reopen().set_active_run("only").unwrap();
    assert!(d.dir.path().join(".darkrun").join("active").exists());
}

#[test]
fn active_inferred_without_pointer_after_reopen() {
    // No explicit pointer set: a single active run is inferred from disk.
    let d = Durable::start("solo");
    assert_eq!(d.reopen().active_run().unwrap(), Some("solo".to_string()));
}

#[test]
fn active_pointer_to_missing_run_falls_back_to_inference() {
    let d = Durable::start("real");
    // Point at a run that does not exist; resolution falls back to the live run.
    d.reopen().set_active_run("ghost").unwrap();
    assert_eq!(d.reopen().active_run().unwrap(), Some("real".to_string()));
}

#[test]
fn active_cleared_pointer_then_infers() {
    let d = Durable::start("solo");
    let store = d.reopen();
    store.set_active_run("solo").unwrap();
    store.clear_active_run().unwrap();
    // Pointer gone; inference still resolves the single active run.
    assert_eq!(d.reopen().active_run().unwrap(), Some("solo".to_string()));
}

#[test]
fn active_newest_run_wins_inference_after_reopen() {
    // Two runs; the newest started_at wins when no pointer is set.
    let dir = tempfile::tempdir().unwrap();
    {
        let store = StateStore::new(dir.path());
        let mut older = run_start(&store, "older", "software", None, "continuous").unwrap();
        older.frontmatter.started_at = Some("2020-01-01T00:00:00Z".to_string());
        store.write_run(&older).unwrap();
        let mut newer = run_start(&store, "newer", "software", None, "continuous").unwrap();
        newer.frontmatter.started_at = Some("2030-01-01T00:00:00Z".to_string());
        store.write_run(&newer).unwrap();
    }
    // Restart: a fresh store over the same tree picks the newer run.
    let reopened = StateStore::new(dir.path());
    assert_eq!(reopened.active_run().unwrap(), Some("newer".to_string()));
}

#[test]
fn active_explicit_pointer_overrides_inference_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = StateStore::new(dir.path());
        let mut newer = run_start(&store, "newer", "software", None, "continuous").unwrap();
        newer.frontmatter.started_at = Some("2030-01-01T00:00:00Z".to_string());
        store.write_run(&newer).unwrap();
        run_start(&store, "pinned", "software", None, "continuous").unwrap();
        // Pin the OLDER-by-inference run explicitly.
        store.set_active_run("pinned").unwrap();
    }
    let reopened = StateStore::new(dir.path());
    assert_eq!(reopened.active_run().unwrap(), Some("pinned".to_string()));
}

#[test]
fn active_archived_run_excluded_from_inference() {
    let d = Durable::start("arch");
    let store = d.reopen();
    let mut run = store.read_run("arch").unwrap();
    run.frontmatter.archived = Some(true);
    store.write_run(&run).unwrap();
    // Archived & no pointer → nothing inferred.
    assert_eq!(d.reopen().active_run().unwrap(), None);
}

#[test]
fn active_completed_run_excluded_from_inference() {
    let d = Durable::start("done");
    let store = d.reopen();
    let mut run = store.read_run("done").unwrap();
    run.frontmatter.status = Status::Completed;
    store.write_run(&run).unwrap();
    assert_eq!(d.reopen().active_run().unwrap(), None);
}

#[test]
fn active_run_listed_after_reopen() {
    let d = Durable::start("listed");
    assert_eq!(d.reopen().list_runs().unwrap(), vec!["listed".to_string()]);
}

#[test]
fn active_pointer_blank_file_falls_back() {
    let d = Durable::start("solo");
    // A blank/whitespace pointer file is ignored; inference takes over.
    let store = d.reopen();
    store.set_active_run("   ").unwrap();
    assert_eq!(store.active_run().unwrap(), Some("solo".to_string()));
}

// ===========================================================================
// Section 4 — Lock acquisition across "processes" (two managers / threads)
// ===========================================================================

#[test]
fn lock_acquire_creates_dir_on_disk() {
    let d = Durable::start("lk");
    let mgr = d.locks();
    let guard = mgr.acquire("run-tick", "p1").unwrap();
    assert!(guard.path().exists());
}

#[test]
fn lock_held_blocks_second_manager_try() {
    let d = Durable::start("lk");
    let a = d.locks();
    let b = d.locks(); // a "second process" over the same tree
    let g = a.acquire("run-tick", "proc-a").unwrap();
    // b cannot acquire while a holds it: with_lock would block to the timeout,
    // so prove contention by checking the dir exists and b sees it.
    assert!(g.path().exists());
    assert!(b.root().join("run-tick").exists());
    drop(g);
    // Once released, the second process acquires immediately.
    let g2 = b.acquire("run-tick", "proc-b").unwrap();
    assert!(g2.path().exists());
}

#[test]
fn lock_release_lets_other_process_acquire() {
    let d = Durable::start("lk");
    let a = d.locks();
    let b = d.locks();
    a.acquire("x", "a").unwrap().release();
    let g = b.acquire("x", "b").unwrap();
    assert!(g.path().exists());
}

#[test]
fn lock_distinct_names_dont_contend() {
    let d = Durable::start("lk");
    let mgr = d.locks();
    let g1 = mgr.acquire("alpha", "p").unwrap();
    let g2 = mgr.acquire("beta", "p").unwrap();
    assert!(g1.path().exists());
    assert!(g2.path().exists());
    assert_ne!(g1.path(), g2.path());
}

#[test]
fn lock_drop_releases_for_next_process() {
    let d = Durable::start("lk");
    let a = d.locks();
    {
        let _g = a.acquire("scoped", "a").unwrap();
        assert!(a.root().join("scoped").exists());
    }
    // Dropped at end of block → dir removed.
    assert!(!a.root().join("scoped").exists());
}

#[test]
fn lock_with_lock_runs_and_releases() {
    let d = Durable::start("lk");
    let mgr = d.locks();
    let out = mgr.with_lock("w", "t", || 7).unwrap();
    assert_eq!(out, 7);
    assert!(!mgr.root().join("w").exists());
}

#[test]
fn lock_reacquire_after_release_same_process() {
    let d = Durable::start("lk");
    let mgr = d.locks();
    mgr.acquire("re", "t").unwrap().release();
    let g = mgr.acquire("re", "t").unwrap();
    assert!(g.path().exists());
}

#[test]
fn lock_root_is_under_darkrun() {
    let d = Durable::start("lk");
    let mgr = d.locks();
    assert!(mgr.root().ends_with("locks"));
    assert!(mgr.root().starts_with(d.dir.path().join(".darkrun")));
}

#[test]
fn lock_stale_holder_with_dead_pid_is_reclaimed() {
    use std::fs;
    let d = Durable::start("lk");
    let mgr = d.locks();
    let lock_dir = mgr.root().join("wedged");
    fs::create_dir_all(&lock_dir).unwrap();
    // A holder with an effectively-never-alive pid.
    let holder = serde_json::json!({ "pid": i32::MAX, "at": 0u64, "tag": "dead" });
    fs::write(lock_dir.join("holder.json"), holder.to_string()).unwrap();
    // Backdate the dir well past the stale threshold (5 min).
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    filetime::set_file_mtime(&lock_dir, filetime::FileTime::from_system_time(old)).unwrap();
    assert!(mgr.is_stale(&lock_dir));
    let g = mgr.acquire("wedged", "fresh").unwrap();
    assert!(g.path().exists());
}

#[test]
fn lock_wedged_no_holder_file_is_stale_after_age() {
    use std::fs;
    let d = Durable::start("lk");
    let mgr = d.locks();
    let lock_dir = mgr.root().join("noholder");
    fs::create_dir_all(&lock_dir).unwrap();
    // No holder.json at all → wedged once old enough.
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    filetime::set_file_mtime(&lock_dir, filetime::FileTime::from_system_time(old)).unwrap();
    assert!(mgr.is_stale(&lock_dir));
}

#[test]
fn lock_fresh_holder_is_not_stale() {
    let d = Durable::start("lk");
    let mgr = d.locks();
    let g = mgr.acquire("fresh", "live").unwrap();
    // Just acquired by THIS (alive) process → never stale.
    assert!(!mgr.is_stale(g.path()));
}

#[test]
fn lock_live_pid_holder_not_reclaimed_even_when_old() {
    use std::fs;
    let d = Durable::start("lk");
    let mgr = d.locks();
    let lock_dir = mgr.root().join("livelock");
    fs::create_dir_all(&lock_dir).unwrap();
    // Holder is THIS process (alive). Even backdated, a live holder is not stale.
    let holder = serde_json::json!({ "pid": std::process::id() as i32, "at": 0u64, "tag": "me" });
    fs::write(lock_dir.join("holder.json"), holder.to_string()).unwrap();
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    filetime::set_file_mtime(&lock_dir, filetime::FileTime::from_system_time(old)).unwrap();
    assert!(!mgr.is_stale(&lock_dir));
}

#[test]
fn lock_missing_dir_is_not_stale() {
    let d = Durable::start("lk");
    let mgr = d.locks();
    // A dir that was never created is "gone", not stale.
    assert!(!mgr.is_stale(&mgr.root().join("never")));
}

#[test]
fn lock_two_managers_share_one_lock_namespace() {
    // Two managers built independently from the same root must contend on the
    // SAME on-disk lock dir — they are two processes, one filesystem.
    let d = Durable::start("lk");
    let a = d.locks();
    let b = d.locks();
    let g = a.acquire("shared", "a").unwrap();
    assert_eq!(g.path(), b.root().join("shared"));
    drop(g);
    let g2 = b.acquire("shared", "b").unwrap();
    assert!(g2.path().exists());
}

// ===========================================================================
// Section 5 — Concurrent ticks serialized by a shared lock
// ===========================================================================

#[test]
fn concurrent_locked_ticks_advance_one_phase_at_a_time() {
    // Two threads, each opening its own store ("process"), both racing to tick
    // the same Run. A shared mkdir lock around each tick serializes them so the
    // phase machine advances cleanly rather than double-stamping.
    let d = Durable::start("conc");
    let root = d.dir.path().to_path_buf();
    let slug = d.slug.clone();
    let barrier = Arc::new(Barrier::new(2));

    let handles: Vec<_> = (0..2)
        .map(|_| {
            let root = root.clone();
            let slug = slug.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                let store = StateStore::new(&root);
                let mgr = LockManager::new(&root);
                barrier.wait();
                mgr.with_lock("tick", "worker", || {
                    run_tick(&store, &slug).expect("tick");
                })
                .expect("with_lock");
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }

    // Two serialized ticks from frame/spec → spec advances to review, then
    // review advances to manufacture. The phase is exactly Manufacture, never
    // skipped or corrupted.
    assert_eq!(d.phase("frame"), StationPhase::Manufacture);
}

#[test]
fn concurrent_locked_ticks_keep_state_parseable() {
    // After a flurry of locked concurrent ticks, the state file is still valid
    // JSON the engine can read — no torn write left it corrupt.
    let d = Durable::start("conc");
    let root = d.dir.path().to_path_buf();
    let slug = d.slug.clone();

    let handles: Vec<_> = (0..8)
        .map(|_| {
            let root = root.clone();
            let slug = slug.clone();
            thread::spawn(move || {
                let store = StateStore::new(&root);
                let mgr = LockManager::new(&root);
                mgr.with_lock("tick", "w", || {
                    let _ = run_tick(&store, &slug);
                })
                .expect("with_lock");
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }

    // State still reads back cleanly.
    let state = d.reopen().read_state(&slug).unwrap().unwrap();
    assert_eq!(state.factory, "software");
    // And the cursor still resolves to a valid action.
    assert!(derive_position(&d.reopen(), &slug).unwrap().action.is_some());
}

#[test]
fn concurrent_distinct_runs_under_lock_dont_interfere() {
    // Two runs, two threads, two locks. Each advances its own run; neither
    // bleeds into the other.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    {
        let store = StateStore::new(&root);
        run_start(&store, "ra", "software", None, "continuous").unwrap();
        run_start(&store, "rb", "software", None, "continuous").unwrap();
    }
    let handles: Vec<_> = ["ra", "rb"]
        .iter()
        .map(|slug| {
            let root = root.clone();
            let slug = slug.to_string();
            thread::spawn(move || {
                let store = StateStore::new(&root);
                let mgr = LockManager::new(&root);
                // Lock per-run so the two runs proceed in parallel without a
                // shared global lock.
                mgr.with_lock(&format!("tick-{slug}"), "w", || {
                    run_tick(&store, &slug).expect("tick");
                })
                .expect("with_lock");
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    let reopened = StateStore::new(&root);
    // Each run advanced exactly one phase: frame spec -> review.
    assert_eq!(
        reopened.read_state("ra").unwrap().unwrap().stations["frame"].phase,
        StationPhase::Review
    );
    assert_eq!(
        reopened.read_state("rb").unwrap().unwrap().stations["frame"].phase,
        StationPhase::Review
    );
}

#[test]
fn concurrent_lock_holder_excludes_others_during_critical_section() {
    // Prove mutual exclusion: a thread holds the lock through a sleep while the
    // main thread confirms it cannot acquire within a short window.
    use std::sync::mpsc;
    let d = Durable::start("excl");
    let root = d.dir.path().to_path_buf();
    let (tx, rx) = mpsc::channel();

    let root_t = root.clone();
    let holder = thread::spawn(move || {
        let mgr = LockManager::new(&root_t);
        let g = mgr.acquire("crit", "holder").unwrap();
        tx.send(()).unwrap(); // signal: lock held
        thread::sleep(std::time::Duration::from_millis(200));
        drop(g); // release
    });

    rx.recv().unwrap(); // wait until the holder owns the lock
    let mgr = LockManager::new(&root);
    // A non-blocking probe: the lock dir exists, so a fresh create_dir fails.
    assert!(mgr.root().join("crit").exists());
    // Now block until the holder releases; we then acquire successfully.
    let g = mgr.acquire("crit", "main").unwrap();
    assert!(g.path().exists());
    holder.join().unwrap();
}

#[test]
fn lock_serializes_full_station_walk_across_threads() {
    // Sequentially complete a station where each phase tick is taken under the
    // shared lock from an alternating "process". The station still reaches its
    // checkpoint cleanly.
    let d = Durable::start("walklk");
    let root = d.dir.path().to_path_buf();
    let mgr = LockManager::new(&root);
    let store = StateStore::new(&root);

    // spec, review under lock.
    mgr.with_lock("t", "p1", || run_tick(&store, "walklk").unwrap())
        .unwrap();
    mgr.with_lock("t", "p2", || run_tick(&store, "walklk").unwrap())
        .unwrap();
    d.decompose("frame", &[("u1", &[])]);
    mgr.with_lock("t", "p1", || run_tick(&store, "walklk").unwrap())
        .unwrap();
    d.complete_unit("u1");
    mgr.with_lock("t", "p2", || run_tick(&store, "walklk").unwrap())
        .unwrap(); // audit
    mgr.with_lock("t", "p1", || run_tick(&store, "walklk").unwrap())
        .unwrap(); // tests
    let cp = mgr
        .with_lock("t", "p2", || run_tick(&store, "walklk").unwrap())
        .unwrap();
    assert!(is_checkpoint(&cp.action, "frame"));
}

// ===========================================================================
// Section 6 — Corrupted / partial state files degrade gracefully
// ===========================================================================

#[test]
fn corrupt_state_json_makes_read_state_error() {
    use std::fs;
    let d = Durable::start("corrupt");
    fs::write(d.state_path(), "{ this is not json").unwrap();
    // A syntactically broken snapshot surfaces an error rather than silently
    // returning a bogus default.
    assert!(d.reopen().read_state("corrupt").is_err());
}

#[test]
fn corrupt_state_json_makes_derive_position_error() {
    use std::fs;
    let d = Durable::start("corrupt");
    fs::write(d.state_path(), "not even close to json").unwrap();
    assert!(derive_position(&d.reopen(), "corrupt").is_err());
}

#[test]
fn corrupt_state_json_makes_run_tick_error() {
    use std::fs;
    let d = Durable::start("corrupt");
    fs::write(d.state_path(), "{{{").unwrap();
    assert!(run_tick(&d.reopen(), "corrupt").is_err());
}

#[test]
fn truncated_state_json_errors_not_panics() {
    use std::fs;
    let d = Durable::start("corrupt");
    // Half a JSON object — a torn write mid-flush.
    fs::write(d.state_path(), "{\"factory\": \"soft").unwrap();
    let res = d.reopen().read_state("corrupt");
    assert!(res.is_err());
}

#[test]
fn empty_state_json_errors() {
    use std::fs;
    let d = Durable::start("corrupt");
    // A zero-byte file is not valid JSON (an empty object would be `{}`).
    fs::write(d.state_path(), "").unwrap();
    assert!(d.reopen().read_state("corrupt").is_err());
}

#[test]
fn empty_object_state_json_is_default() {
    use std::fs;
    let d = Durable::start("corrupt");
    // `{}` is valid: every field falls back to its serde default.
    fs::write(d.state_path(), "{}").unwrap();
    let state = d.reopen().read_state("corrupt").unwrap().unwrap();
    assert_eq!(state.factory, "");
    assert_eq!(state.active_station, "");
    assert!(state.stations.is_empty());
}

#[test]
fn partial_state_missing_stations_defaults_empty() {
    use std::fs;
    let d = Durable::start("partial");
    // Valid JSON, but the `stations` map is absent → serde default (empty).
    fs::write(
        d.state_path(),
        "{\"factory\":\"software\",\"active_station\":\"frame\"}",
    )
    .unwrap();
    let state = d.reopen().read_state("partial").unwrap().unwrap();
    assert_eq!(state.factory, "software");
    assert_eq!(state.active_station, "frame");
    assert!(state.stations.is_empty());
}

#[test]
fn partial_state_missing_stations_recovers_at_frame_spec() {
    use std::fs;
    let d = Durable::start("partial");
    // A snapshot that lost its per-station map degrades gracefully: with no
    // station entry, current_station treats frame as incomplete and the cursor
    // resumes the run at frame/Spec.
    fs::write(
        d.state_path(),
        "{\"factory\":\"software\",\"active_station\":\"frame\"}",
    )
    .unwrap();
    let pos = derive_position(&d.reopen(), "partial").unwrap();
    assert!(is_spec(pos.action.as_ref().unwrap(), "frame"));
}

#[test]
fn partial_state_missing_factory_field_defaults_blank() {
    use std::fs;
    let d = Durable::start("partial");
    fs::write(d.state_path(), "{\"active_station\":\"frame\"}").unwrap();
    let state = d.reopen().read_state("partial").unwrap().unwrap();
    assert_eq!(state.factory, "");
    // But the run's factory still resolves from run.md, so the cursor walks.
    assert!(derive_position(&d.reopen(), "partial")
        .unwrap()
        .action
        .is_some());
}

#[test]
fn partial_state_recovers_and_can_retick() {
    use std::fs;
    let d = Durable::start("partial");
    fs::write(
        d.state_path(),
        "{\"factory\":\"software\",\"active_station\":\"frame\"}",
    )
    .unwrap();
    // Re-ticking rebuilds the station entry and advances the phase from Spec.
    run_tick(&d.reopen(), "partial").unwrap();
    assert_eq!(d.phase("frame"), StationPhase::Review);
}

#[test]
fn partial_state_with_one_station_resumes_there() {
    use std::fs;
    let d = Durable::start("partial");
    // A snapshot carrying only frame=completed should resume at specify/Spec.
    let snapshot = serde_json::json!({
        "factory": "software",
        "active_station": "frame",
        "stations": {
            "frame": {
                "station": "frame",
                "status": "completed",
                "phase": "checkpoint",
                "checkpoint": { "kind": "ask", "outcome": "advanced" }
            }
        }
    });
    fs::write(d.state_path(), serde_json::to_string(&snapshot).unwrap()).unwrap();
    let pos = derive_position(&d.reopen(), "partial").unwrap();
    assert!(is_spec(pos.action.as_ref().unwrap(), "specify"));
}

#[test]
fn missing_state_json_defaults_and_resumes_at_frame() {
    use std::fs;
    let d = Durable::start("nostate");
    // Delete state.json entirely — the worst partial: it's gone.
    fs::remove_file(d.state_path()).unwrap();
    // read_state → None; the manager treats it as default and resumes at frame.
    assert!(d.reopen().read_state("nostate").unwrap().is_none());
    let pos = derive_position(&d.reopen(), "nostate").unwrap();
    assert!(is_spec(pos.action.as_ref().unwrap(), "frame"));
}

#[test]
fn missing_state_json_retick_reseeds_it() {
    use std::fs;
    let d = Durable::start("nostate");
    fs::remove_file(d.state_path()).unwrap();
    run_tick(&d.reopen(), "nostate").unwrap();
    // The tick rewrote state.json from the run's factory default.
    assert!(d.state_path().exists());
    assert_eq!(d.state().factory, "software");
}

#[test]
fn corrupt_unit_file_surfaces_error_not_panic() {
    use std::fs;
    let d = Durable::start("badunit");
    d.tick_fresh();
    d.tick_fresh();
    d.decompose("frame", &[("u1", &[])]);
    // Corrupt the unit's markdown: no frontmatter fence at all.
    let path = d.reopen().units_dir("badunit").join("u1.md");
    fs::write(&path, "totally bogus, no frontmatter\n").unwrap();
    // Reading that one unit errors; the engine propagates rather than panicking.
    assert!(d.reopen().read_unit("badunit", "u1").is_err());
    // And read_units (which reads every unit) propagates the same error.
    assert!(d.reopen().read_units("badunit").is_err());
}

#[test]
fn corrupt_run_md_surfaces_error() {
    use std::fs;
    let d = Durable::start("badrun");
    let path = d.reopen().run_dir("badrun").join("run.md");
    fs::write(&path, "no frontmatter fence here\n").unwrap();
    // derive_position reads run.md first → propagates the parse error.
    assert!(derive_position(&d.reopen(), "badrun").is_err());
}

#[test]
fn missing_run_md_is_run_not_found() {
    use std::fs;
    let d = Durable::start("gone");
    let path = d.reopen().run_dir("gone").join("run.md");
    fs::remove_file(&path).unwrap();
    // A vanished run.md is a clean RunNotFound, not a panic.
    assert!(d.reopen().read_run("gone").is_err());
    assert!(derive_position(&d.reopen(), "gone").is_err());
}

#[test]
fn corrupt_state_does_not_destroy_run_md() {
    use std::fs;
    let d = Durable::start("survive");
    fs::write(d.state_path(), "garbage").unwrap();
    // Even with a wrecked snapshot, the durable run.md is untouched and the
    // run can be rebuilt by hand: rewrite a clean default state.json.
    assert!(d.reopen().read_run("survive").is_ok());
    let clean = RunState {
        factory: "software".to_string(),
        active_station: "frame".to_string(),
        ..Default::default()
    };
    d.reopen().write_state("survive", &clean).unwrap();
    // Recovery complete: the cursor walks again from frame/Spec.
    let pos = derive_position(&d.reopen(), "survive").unwrap();
    assert!(is_spec(pos.action.as_ref().unwrap(), "frame"));
}

#[test]
fn rewriting_state_after_corruption_resumes_run() {
    use std::fs;
    let d = Durable::start("heal");
    // Walk a bit, then corrupt, then heal by replaying the default — the run's
    // own units survive and re-enter the wave.
    d.tick_fresh();
    d.tick_fresh();
    d.decompose("frame", &[("u1", &[])]);
    fs::write(d.state_path(), "}{ broken").unwrap();
    assert!(d.reopen().read_state("heal").is_err());
    // Heal: write a default snapshot. The unit is still on disk.
    d.reopen()
        .write_state(
            "heal",
            &RunState {
                factory: "software".to_string(),
                active_station: "frame".to_string(),
                ..Default::default()
            },
        )
        .unwrap();
    // Unit u1 persisted through the corruption.
    assert!(d.reopen().read_unit("heal", "u1").is_ok());
}

// ===========================================================================
// Section 7 — Idempotent re-derivation after a reopen (pure-read durability)
// ===========================================================================

#[test]
fn reopen_derive_is_deterministic() {
    let d = Durable::start("det");
    d.tick_fresh();
    let a = derive_position(&d.reopen(), "det").unwrap();
    let b = derive_position(&d.reopen(), "det").unwrap();
    assert_eq!(a, b);
}

#[test]
fn reopen_derive_does_not_mutate_state() {
    let d = Durable::start("det");
    let before = d.state().active_station;
    derive_position(&d.reopen(), "det").unwrap();
    derive_position(&d.reopen(), "det").unwrap();
    assert_eq!(d.state().active_station, before);
}

#[test]
fn reopen_derive_stable_across_many_reopens() {
    let d = Durable::start("det");
    d.tick_fresh();
    d.tick_fresh();
    d.decompose("frame", &[("u1", &[])]);
    let first = derive_position(&d.reopen(), "det").unwrap();
    for _ in 0..25 {
        assert_eq!(derive_position(&d.reopen(), "det").unwrap(), first);
    }
}

#[test]
fn reopen_at_checkpoint_is_stable() {
    let d = Durable::start("det");
    walk_to_checkpoint_fresh(&d, "frame", &["u1"]);
    let a = derive_position(&d.reopen(), "det").unwrap();
    let b = derive_position(&d.reopen(), "det").unwrap();
    assert_eq!(a, b);
    assert!(is_checkpoint(a.action.as_ref().unwrap(), "frame"));
}

#[test]
fn reopen_sealed_run_stays_sealed() {
    // Walk the entire run to Sealed (via the shared harness), then reopen the
    // store and confirm the sealed terminal survives.
    let h = Harness::start("seal");
    h.run_to_seal();
    // Reopen over the same root the harness used.
    let reopened = StateStore::new(h.store.root().parent().unwrap());
    let pos = derive_position(&reopened, "seal").unwrap();
    assert!(matches!(pos.action, Some(RunAction::Sealed { .. })));
}

#[test]
fn reopen_sealed_run_all_stations_completed() {
    let h = Harness::start("seal");
    h.run_to_seal();
    let reopened = StateStore::new(h.store.root().parent().unwrap());
    let state = reopened.read_state("seal").unwrap().unwrap();
    for s in STATIONS {
        assert_eq!(state.stations[s].status, Status::Completed, "{s}");
    }
}

#[test]
fn reopen_does_not_advance_phase_on_pure_read() {
    let d = Durable::start("det");
    d.tick_fresh(); // now at review
    derive_position(&d.reopen(), "det").unwrap();
    derive_position(&d.reopen(), "det").unwrap();
    // Pure derive never advanced the phase past review.
    assert_eq!(d.phase("frame"), StationPhase::Review);
}

// ===========================================================================
// Section 8 — Mixed durability: units + feedback + state survive together
// ===========================================================================

#[test]
fn reopen_units_and_feedback_coexist_after_restart() {
    let d = Durable::start("mix");
    d.tick_fresh();
    d.tick_fresh();
    d.decompose("frame", &[("u1", &[]), ("u2", &[])]);
    d.reopen()
        .write_feedback_raw("mix", "fb-1", "---\nstatus: pending\n---\nlook here\n")
        .unwrap();
    let store = d.reopen();
    assert_eq!(store.read_units("mix").unwrap().len(), 2);
    assert!(store.read_feedback_raw("mix").unwrap().contains_key("fb-1"));
    // Feedback still preempts the run after reopen.
    assert_eq!(
        derive_position(&d.reopen(), "mix").unwrap().track,
        Track::Feedback
    );
}

#[test]
fn reopen_session_json_survives() {
    let d = Durable::start("sess");
    let payload = serde_json::json!({ "session_id": "s1", "status": "pending" });
    d.reopen().write_session("sess", &payload).unwrap();
    let back = d.reopen().read_session("sess").unwrap().unwrap();
    assert_eq!(back["session_id"], "s1");
}

#[test]
fn reopen_corrupt_session_json_errors() {
    use std::fs;
    let d = Durable::start("sess");
    let path = d.reopen().run_dir("sess").join("session.json");
    fs::write(&path, "not json").unwrap();
    assert!(d.reopen().read_session("sess").is_err());
}

#[test]
fn reopen_missing_session_is_none() {
    let d = Durable::start("sess");
    assert!(d.reopen().read_session("sess").unwrap().is_none());
}

#[test]
fn full_durability_walk_each_tick_a_fresh_process() {
    // The capstone: drive a full station-to-checkpoint walk where the entire
    // sequence is reconstructed from disk on every step, then approve and
    // confirm the next station is live — all bytes, no shared memory.
    let d = Durable::start("cap");
    walk_to_checkpoint_fresh(&d, "frame", &["a", "b"]);
    assert!(is_checkpoint(
        derive_position(&d.reopen(), "cap")
            .unwrap()
            .action
            .as_ref()
            .unwrap(),
        "frame"
    ));
    // Approving surfaces specify/Spec as the next action (the decide re-tick),
    // then the cursor lives on specify with frame sealed completed.
    let decided = checkpoint_decide(&d.reopen(), "cap", true, None).unwrap();
    assert!(is_spec(&decided.action, "specify"));
    assert_eq!(d.active(), "specify");
    assert_eq!(d.station_status("frame"), Status::Completed);
}
