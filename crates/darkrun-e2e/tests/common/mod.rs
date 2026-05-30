//! Shared E2E harness — a thin driver over the darkrun engine crates that
//! lets the lifecycle tests express a full software-factory Run as a sequence
//! of high-level moves (spec a station, decompose units, complete a wave,
//! decide a checkpoint) without re-deriving the manager's plumbing each time.
//!
//! The harness owns a real [`StateStore`] rooted in a `tempfile::TempDir`, so
//! every test drives genuine on-disk `.darkrun/` state through the same
//! `darkrun-mcp` manager (`run_start` -> `run_tick` -> `derive_position`) the
//! production server uses. Nothing here mocks the engine.

#![allow(dead_code)]

use darkrun_core::domain::{CheckpointKind, Status, StationPhase, Unit, UnitFrontmatter};
use darkrun_core::{RunState, StateStore};
use darkrun_mcp::position::{
    checkpoint_decide, derive_position, run_start, run_tick, Position, RunAction, TickResult,
};

/// The software factory's six stations, in cost-of-late-discovery order. The
/// manager's inline `software_factory()` is the authority the Run walks.
pub const STATIONS: [&str; 6] = ["frame", "specify", "shape", "build", "prove", "harden"];

/// The phase order every station walks.
pub const PHASES: [StationPhase; 6] = [
    StationPhase::Spec,
    StationPhase::Review,
    StationPhase::Manufacture,
    StationPhase::Audit,
    StationPhase::Reflect,
    StationPhase::Checkpoint,
];

/// Checkpoint kind per station in the *manager's* inline factory (distinct
/// from the embedded content corpus, where `prove` is `ask`).
pub fn manager_checkpoint(station: &str) -> CheckpointKind {
    match station {
        "frame" | "specify" | "shape" => CheckpointKind::Ask,
        "build" | "prove" => CheckpointKind::Auto,
        "harden" => CheckpointKind::External,
        other => panic!("unknown station {other}"),
    }
}

/// A self-contained Run fixture: a temp dir + the store rooted in it + the run
/// slug. Dropping it tears down all on-disk state.
pub struct Harness {
    _dir: tempfile::TempDir,
    pub store: StateStore,
    pub slug: String,
}

impl Harness {
    /// Start a fresh `software` run with the given slug.
    pub fn start(slug: &str) -> Self {
        Self::start_with(slug, "software", None, "continuous")
    }

    /// Start a run with explicit factory / title / mode.
    pub fn start_with(slug: &str, factory: &str, title: Option<&str>, mode: &str) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = StateStore::new(dir.path());
        run_start(&store, slug, factory, title.map(String::from), mode).expect("run_start");
        Harness {
            _dir: dir,
            store,
            slug: slug.to_string(),
        }
    }

    /// Tick once; return the action.
    pub fn tick(&self) -> TickResult {
        run_tick(&self.store, &self.slug).expect("tick")
    }

    /// Derive the current cursor position without advancing.
    pub fn position(&self) -> Position {
        derive_position(&self.store, &self.slug).expect("derive")
    }

    /// Decide the active checkpoint.
    pub fn decide(&self, approved: bool, feedback: Option<&str>) -> TickResult {
        checkpoint_decide(&self.store, &self.slug, approved, feedback.map(String::from))
            .expect("decide")
    }

    /// Read the persisted state snapshot.
    pub fn state(&self) -> RunState {
        self.store
            .read_state(&self.slug)
            .expect("read_state")
            .expect("state present")
    }

    /// The persisted phase of a station, defaulting to `Spec` when unseen.
    pub fn phase(&self, station: &str) -> StationPhase {
        self.state()
            .stations
            .get(station)
            .map(|s| s.phase)
            .unwrap_or(StationPhase::Spec)
    }

    /// The persisted status of a station, defaulting to `Pending` when unseen.
    pub fn station_status(&self, station: &str) -> Status {
        self.state()
            .stations
            .get(station)
            .map(|s| s.status)
            .unwrap_or(Status::Pending)
    }

    /// The active-station pointer.
    pub fn active(&self) -> String {
        self.state().active_station
    }

    /// Decompose a wave of units on a station, with optional per-unit deps.
    pub fn decompose(&self, station: &str, units: &[(&str, &[&str])]) {
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
            self.store.write_unit(&self.slug, &unit).expect("write_unit");
        }
    }

    /// Mark a single unit completed.
    pub fn complete_unit(&self, unit_slug: &str) {
        let mut u = self.store.read_unit(&self.slug, unit_slug).expect("read_unit");
        u.frontmatter.status = Status::Completed;
        self.store.write_unit(&self.slug, &u).expect("write_unit");
    }

    /// Mark every named unit completed.
    pub fn complete_units(&self, slugs: &[&str]) {
        for s in slugs {
            self.complete_unit(s);
        }
    }

    /// Force a station's persisted phase (test setup — lets a suite place the
    /// cursor at a station's canonical `Spec` entry regardless of how the
    /// upstream gate's internal re-tick left it).
    pub fn set_phase(&self, station: &str, phase: StationPhase) {
        let mut state = self.state();
        if let Some(st) = state.stations.get_mut(station) {
            st.phase = phase;
        }
        self.store.write_state(&self.slug, &state).expect("write_state");
    }

    /// File a feedback document directly (Track B preemption source).
    pub fn file_feedback(&self, id: &str, status: &str, body: &str) {
        let doc = format!("---\nstatus: {status}\n---\n{body}\n");
        self.store
            .write_feedback_raw(&self.slug, id, &doc)
            .expect("write_feedback");
    }

    /// Drive a single station from `Spec` to a held `Checkpoint`, decomposing
    /// `unit_slugs` as its wave during `Manufacture`. Returns the ordered list
    /// of actions emitted across the walk (one per tick). The checkpoint tick
    /// is included as the final action.
    pub fn walk_station_to_checkpoint(&self, station: &str, unit_slugs: &[&str]) -> Vec<RunAction> {
        let mut actions = Vec::new();
        // Spec.
        actions.push(self.tick().action);
        // Review.
        actions.push(self.tick().action);
        // Manufacture: decompose then dispatch one wave, complete it.
        if !unit_slugs.is_empty() {
            let units: Vec<(&str, &[&str])> =
                unit_slugs.iter().map(|s| (*s, &[][..])).collect();
            self.decompose(station, &units);
            actions.push(self.tick().action); // Manufacture dispatch
            self.complete_units(unit_slugs);
        }
        // Audit.
        actions.push(self.tick().action);
        // Tests.
        actions.push(self.tick().action);
        // Checkpoint.
        actions.push(self.tick().action);
        actions
    }

    /// Complete a station fully — walk to its checkpoint, then approve it so the
    /// cursor advances to the next station's `Spec`. For `Auto` gates the
    /// checkpoint tick already advances, so no decide is needed.
    pub fn complete_station(&self, station: &str, unit_slugs: &[&str]) {
        self.walk_station_to_checkpoint(station, unit_slugs);
        if !matches!(manager_checkpoint(station), CheckpointKind::Auto) {
            self.decide(true, None);
        }
    }

    /// Drive the entire run to a sealed state, giving every station one unit.
    /// Returns the full ordered action log — one entry per distinct cursor
    /// stop, in the order the manager surfaced them.
    ///
    /// This is the faithful e2e driver: it reads the manager's next action and
    /// reacts to it (decompose a unit when a station owes Spec, complete the
    /// wave, approve a held gate), looping until the Run is Sealed. Consecutive
    /// duplicate actions — e.g. the Spec the decide re-tick emits and the Spec
    /// the next loop would derive — collapse to a single log entry so the log
    /// reads as the canonical six-station walk.
    pub fn run_to_seal(&self) -> Vec<RunAction> {
        let mut log: Vec<RunAction> = Vec::new();
        let mut guard = 0;
        loop {
            guard += 1;
            assert!(guard < 1000, "run_to_seal failed to converge");
            let action = self.tick().action;
            // Collapse a repeat of the immediately-preceding action.
            if log.last() != Some(&action) {
                log.push(action.clone());
            }
            match &action {
                RunAction::Sealed { .. } => break,
                RunAction::Spec { station, .. } => {
                    // Owe a wave: decompose one unit so Manufacture has work.
                    let unit = format!("{station}-unit");
                    if self.store.read_unit(&self.slug, &unit).is_err() {
                        self.decompose(station, &[(unit.as_str(), &[])]);
                    }
                }
                RunAction::Manufacture { units, .. } => {
                    let owned: Vec<&str> = units.iter().map(|s| s.as_str()).collect();
                    self.complete_units(&owned);
                }
                RunAction::Checkpoint { station, kind, .. }
                    if !matches!(kind, CheckpointKind::Auto) =>
                {
                    let decided = self.decide(true, None);
                    if log.last() != Some(&decided.action) {
                        log.push(decided.action.clone());
                    }
                    // The decide re-tick already advanced the next station;
                    // re-handle its action so the loop stays in sync.
                    if let RunAction::Spec { station: ns, .. } = &decided.action {
                        let unit = format!("{ns}-unit");
                        if self.store.read_unit(&self.slug, &unit).is_err() {
                            self.decompose(ns, &[(unit.as_str(), &[])]);
                        }
                    }
                    let _ = station;
                }
                _ => {}
            }
        }
        log
    }
}

/// Assert an action is a `Spec` on `station`.
pub fn is_spec(a: &RunAction, station: &str) -> bool {
    matches!(a, RunAction::Spec { station: s, .. } if s == station)
}

/// Assert an action is a `Review` on `station`.
pub fn is_review(a: &RunAction, station: &str) -> bool {
    matches!(a, RunAction::Review { station: s, .. } if s == station)
}

/// Assert an action is a `Manufacture` on `station`.
pub fn is_manufacture(a: &RunAction, station: &str) -> bool {
    matches!(a, RunAction::Manufacture { station: s, .. } if s == station)
}

/// Assert an action is an `Audit` on `station`.
pub fn is_audit(a: &RunAction, station: &str) -> bool {
    matches!(a, RunAction::Audit { station: s, .. } if s == station)
}

/// Assert an action is a `Reflect` on `station`.
pub fn is_reflect(a: &RunAction, station: &str) -> bool {
    matches!(a, RunAction::Reflect { station: s, .. } if s == station)
}

/// Assert an action is a `Checkpoint` on `station`.
pub fn is_checkpoint(a: &RunAction, station: &str) -> bool {
    matches!(a, RunAction::Checkpoint { station: s, .. } if s == station)
}

/// The `action` discriminator string serde emits for an action.
pub fn action_tag(a: &RunAction) -> String {
    serde_json::to_value(a).unwrap()["action"]
        .as_str()
        .unwrap()
        .to_string()
}

/// The `station` field of an action, if it carries one.
pub fn action_station(a: &RunAction) -> Option<String> {
    serde_json::to_value(a).unwrap()["station"]
        .as_str()
        .map(String::from)
}

