//! Filesystem state engine.
//!
//! State is filesystem-only (no DB). The layout under a repo root lives
//! under `.darkrun/`:
//!
//! ```text
//! .darkrun/<run>/
//!   run.md          frontmatter + body for the Run
//!   units/<slug>.md one markdown doc per Unit
//!   state.json      derived station/run state snapshot
//!   feedback/*.md   feedback items (frontmatter + body)
//!   proof.json      attached objective-evidence proofs, if any
//! ```
//!
//! Interactive sessions (question/direction/picker) are EPHEMERAL and live only
//! in an in-memory registry shared by the in-process MCP + HTTP servers — they
//! are never persisted here.
//!
//! [`StateStore`] reads and writes this layout. It does not interpret the
//! manager's walk — it only persists and resolves the durable shapes.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::domain::{Run, RunFrontmatter, Station, StationPhase, Status, Unit, UnitFrontmatter};
use crate::error::{CoreError, Result};
use crate::frontmatter;

/// The derived state snapshot persisted to `state.json`.
///
/// This is a write-through cache of the run's station/phase position plus
/// per-station derived state — the manager owns its meaning; the store
/// owns its serialization.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RunState {
    /// The factory driving this run.
    #[serde(default)]
    pub factory: String,
    /// The station the run currently sits on.
    #[serde(default)]
    pub active_station: String,
    /// The ordered station plan this run actually walks — a subset of the
    /// factory's stations, chosen by right-sizing at run start. Empty means "the
    /// full factory plan" (the manager falls back to every factory station), so
    /// existing runs and full-size runs need no plan recorded.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plan: Vec<String>,
    /// When `true`, every checkpoint in this run is treated as `auto` — the
    /// right-sized fast path for small work, where the operator opted out of
    /// per-station gates.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub auto_gates: bool,
    /// When `true`, this run is in DISCRETE mode: each station's checkpoint
    /// resolves on a human PR/MR merge rather than in-process. Snapshotted at
    /// run start from the mode so the (pure) cursor can resolve the External
    /// gate without re-parsing the mode string. (The discrete gate-resolution
    /// itself is a later phase; the branch hierarchy is universal regardless.)
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub discrete: bool,
    /// When `true`, this run is in DISCRETE-HYBRID mode: it runs continuous
    /// (in-process gate resolution) within stations EXCEPT those whose factory
    /// checkpoint is `external`, which open a per-station draft PR/MR that the
    /// human merges. Snapshotted at run start alongside `discrete` so the (pure)
    /// cursor resolves each station's gate without re-parsing the mode string.
    /// Only meaningful when `discrete` is also set.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub discrete_hybrid: bool,
    /// The base branch this run's `darkrun/<slug>/main` forked from, snapshotted
    /// at run start so the run-completion land has a stable target even if
    /// `settings.yml` changes mid-run. Absent on legacy state (resolved live).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_branch: Option<String>,
    /// Per-station derived state, keyed by station name.
    #[serde(default)]
    pub stations: BTreeMap<String, Station>,
}

/// The derived position of one station in a run's ordered plan — the strip
/// entry the desktop renders: its name, its lifecycle [`Status`] (done /
/// current / pending), and the [`StationPhase`] it currently sits in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StationStatus {
    /// The station name, in plan order.
    pub station: String,
    /// Lifecycle status: `Completed` (done), `Active`/`InProgress` (current),
    /// `Pending` (not yet reached), or `Blocked`.
    pub status: Status,
    /// The phase within the station — `Spec` for a station not yet entered.
    pub phase: StationPhase,
}

impl RunState {
    /// The ordered station names this run actually walks: its explicit
    /// right-sized [`plan`](RunState::plan), or the factory's full ordered
    /// station list when no plan is recorded (full-size / legacy runs).
    ///
    /// `factory_stations` is the factory's declared station order (from
    /// `FACTORY.md` frontmatter `stations: [...]`); the caller resolves it.
    pub fn ordered_stations(&self, factory_stations: &[String]) -> Vec<String> {
        if self.plan.is_empty() {
            factory_stations.to_vec()
        } else {
            self.plan.clone()
        }
    }

    /// Per-station status + phase for the run's ordered plan — the STATION
    /// strip the desktop renders.
    ///
    /// A station with a recorded entry in [`stations`](RunState::stations)
    /// reports its persisted `status`/`phase`; a station not yet reached
    /// reports `Pending` in the `Spec` phase (the freshly-entered default).
    pub fn station_status_summary(&self, factory_stations: &[String]) -> Vec<StationStatus> {
        self.ordered_stations(factory_stations)
            .into_iter()
            .map(|name| {
                let (status, phase) = self
                    .stations
                    .get(&name)
                    .map(|st| (st.status, st.phase))
                    .unwrap_or((Status::Pending, StationPhase::Spec));
                StationStatus {
                    station: name,
                    status,
                    phase,
                }
            })
            .collect()
    }

    /// The phase of the active station — the live PHASE subheader. Resolves the
    /// [`active_station`](RunState::active_station)'s recorded phase, defaulting
    /// to `Spec` when the station has no entry yet (or none is active).
    pub fn active_phase(&self) -> StationPhase {
        self.stations
            .get(&self.active_station)
            .map(|st| st.phase)
            .unwrap_or(StationPhase::Spec)
    }
}

pub(crate) fn io<T>(path: &Path, r: std::io::Result<T>) -> Result<T> {
    r.map_err(|source| CoreError::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// Reads and writes the `.darkrun/` filesystem state layout.
#[derive(Debug, Clone)]
pub struct StateStore {
    root: PathBuf,
}

impl StateStore {
    /// Create a store rooted at `<repo_root>/.darkrun`.
    pub fn new(repo_root: impl AsRef<Path>) -> Self {
        StateStore {
            root: repo_root.as_ref().join(".darkrun"),
        }
    }

    /// The `.darkrun` root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The directory for a given run slug.
    pub fn run_dir(&self, slug: &str) -> PathBuf {
        self.root.join(slug)
    }

    /// The `units/` directory for a run.
    pub fn units_dir(&self, slug: &str) -> PathBuf {
        self.run_dir(slug).join("units")
    }

    /// The `feedback/` directory for a run.
    pub fn feedback_dir(&self, slug: &str) -> PathBuf {
        self.run_dir(slug).join("feedback")
    }

    /// List the slugs of every run on disk (sorted).
    pub fn list_runs(&self) -> Result<Vec<String>> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }
        let mut slugs = Vec::new();
        for entry in io(&self.root, fs::read_dir(&self.root))? {
            let entry = io(&self.root, entry)?;
            let path = entry.path();
            if path.is_dir() && path.join("run.md").exists() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    slugs.push(name.to_string());
                }
            }
        }
        slugs.sort();
        Ok(slugs)
    }

    // ─── Active-run pointer ──────────────────────────────────────────────

    /// Path to the `.darkrun/active` pointer file. `list_runs` only treats
    /// directories containing `run.md` as runs, so this plain file is never
    /// mistaken for one.
    fn active_pointer(&self) -> PathBuf {
        self.root.join("active")
    }

    /// Record `slug` as the active run — the one the `statusline` and the
    /// bare `run` subcommands resolve to when no slug is given.
    pub fn set_active_run(&self, slug: &str) -> Result<()> {
        io(&self.root, fs::create_dir_all(&self.root))?;
        let path = self.active_pointer();
        io(&path, fs::write(&path, slug))
    }

    /// Clear the active-run pointer. Idempotent.
    pub fn clear_active_run(&self) -> Result<()> {
        let path = self.active_pointer();
        if path.exists() {
            io(&path, fs::remove_file(&path))?;
        }
        Ok(())
    }

    /// Resolve the active run: the `.darkrun/active` pointer when it names a
    /// run that still exists, otherwise the most-recently-started,
    /// non-archived run whose status is `Active`/`InProgress`. `None` when
    /// nothing is active (or there is no `.darkrun/`).
    pub fn active_run(&self) -> Result<Option<String>> {
        let pointer = self.active_pointer();
        if pointer.exists() {
            let slug = io(&pointer, fs::read_to_string(&pointer))?
                .trim()
                .to_string();
            if !slug.is_empty() && self.run_dir(&slug).join("run.md").exists() {
                return Ok(Some(slug));
            }
        }
        // Infer from on-disk runs. RFC3339 start timestamps sort lexically,
        // so the largest `started_at` is the newest; a missing timestamp
        // sorts first and only wins when it is the sole candidate.
        let mut candidates: Vec<(String, String)> = Vec::new();
        for slug in self.list_runs()? {
            let run = match self.read_run(&slug) {
                Ok(r) => r,
                Err(_) => continue,
            };
            if run.frontmatter.archived.unwrap_or(false) {
                continue;
            }
            if matches!(run.frontmatter.status, Status::Active | Status::InProgress) {
                let started = run.frontmatter.started_at.clone().unwrap_or_default();
                candidates.push((started, slug));
            }
        }
        candidates.sort();
        Ok(candidates.pop().map(|(_, slug)| slug))
    }

    // ─── Run document ────────────────────────────────────────────────────

    /// Read and parse `run.md` for a run slug.
    pub fn read_run(&self, slug: &str) -> Result<Run> {
        let path = self.run_dir(slug).join("run.md");
        if !path.exists() {
            return Err(CoreError::RunNotFound(slug.to_string()));
        }
        let raw = io(&path, fs::read_to_string(&path))?;
        let (frontmatter, body) = frontmatter::parse::<RunFrontmatter>(&raw)?;
        let title = frontmatter
            .title
            .clone()
            .or_else(|| frontmatter::first_heading(&body))
            .unwrap_or_else(|| slug.to_string());
        Ok(Run {
            slug: slug.to_string(),
            frontmatter,
            title,
            body,
        })
    }

    /// Write `run.md`, creating the run directory if needed.
    pub fn write_run(&self, run: &Run) -> Result<()> {
        let dir = self.run_dir(&run.slug);
        io(&dir, fs::create_dir_all(&dir))?;
        let path = dir.join("run.md");
        let content = frontmatter::serialize(&run.frontmatter, &run.body)?;
        io(&path, fs::write(&path, content))
    }

    // ─── Unit documents ──────────────────────────────────────────────────

    /// Read and parse a single unit document.
    pub fn read_unit(&self, run: &str, unit_slug: &str) -> Result<Unit> {
        let path = self.units_dir(run).join(format!("{unit_slug}.md"));
        if !path.exists() {
            return Err(CoreError::UnitNotFound(unit_slug.to_string()));
        }
        let raw = io(&path, fs::read_to_string(&path))?;
        let (frontmatter, body) = frontmatter::parse::<UnitFrontmatter>(&raw)?;
        let title = frontmatter
            .name
            .clone()
            .or_else(|| frontmatter::first_heading(&body))
            .unwrap_or_else(|| unit_slug.to_string());
        Ok(Unit {
            slug: unit_slug.to_string(),
            frontmatter,
            title,
            body,
        })
    }

    /// Read every unit document for a run, sorted by slug.
    pub fn read_units(&self, run: &str) -> Result<Vec<Unit>> {
        let dir = self.units_dir(run);
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut slugs = Vec::new();
        for entry in io(&dir, fs::read_dir(&dir))? {
            let entry = io(&dir, entry)?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("md") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    slugs.push(stem.to_string());
                }
            }
        }
        slugs.sort();
        slugs.iter().map(|s| self.read_unit(run, s)).collect()
    }

    /// Write a single unit document.
    pub fn write_unit(&self, run: &str, unit: &Unit) -> Result<()> {
        let dir = self.units_dir(run);
        io(&dir, fs::create_dir_all(&dir))?;
        let path = dir.join(format!("{}.md", unit.slug));
        let content = frontmatter::serialize(&unit.frontmatter, &unit.body)?;
        io(&path, fs::write(&path, content))
    }

    // ─── Derived state (state.json) ──────────────────────────────────────

    /// Read the derived `state.json` snapshot, or `None` when absent.
    pub fn read_state(&self, run: &str) -> Result<Option<RunState>> {
        let path = self.run_dir(run).join("state.json");
        if !path.exists() {
            return Ok(None);
        }
        let raw = io(&path, fs::read_to_string(&path))?;
        Ok(Some(serde_json::from_str(&raw)?))
    }

    /// Write the derived `state.json` snapshot.
    pub fn write_state(&self, run: &str, state: &RunState) -> Result<()> {
        let dir = self.run_dir(run);
        io(&dir, fs::create_dir_all(&dir))?;
        let path = dir.join("state.json");
        let json = serde_json::to_string_pretty(state)?;
        io(&path, fs::write(&path, json))
    }

    // ─── Feedback documents ──────────────────────────────────────────────

    /// Read every raw feedback document body for a run, keyed by file stem.
    pub fn read_feedback_raw(&self, run: &str) -> Result<BTreeMap<String, String>> {
        let dir = self.feedback_dir(run);
        let mut out = BTreeMap::new();
        if !dir.exists() {
            return Ok(out);
        }
        for entry in io(&dir, fs::read_dir(&dir))? {
            let entry = io(&dir, entry)?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("md") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    let raw = io(&path, fs::read_to_string(&path))?;
                    out.insert(stem.to_string(), raw);
                }
            }
        }
        Ok(out)
    }

    /// Write a raw feedback document.
    pub fn write_feedback_raw(&self, run: &str, id: &str, content: &str) -> Result<()> {
        let dir = self.feedback_dir(run);
        io(&dir, fs::create_dir_all(&dir))?;
        let path = dir.join(format!("{id}.md"));
        io(&path, fs::write(&path, content))
    }

    /// The `reflections/` directory for a run — where the Reflect phase's
    /// retrospectives collect.
    pub fn reflections_dir(&self, slug: &str) -> PathBuf {
        self.run_dir(slug).join("reflections")
    }

    /// Read every reflection document for a run, keyed by id (sorted).
    pub fn read_reflections_raw(&self, run: &str) -> Result<BTreeMap<String, String>> {
        let dir = self.reflections_dir(run);
        let mut out = BTreeMap::new();
        if !dir.exists() {
            return Ok(out);
        }
        for entry in io(&dir, fs::read_dir(&dir))? {
            let entry = io(&dir, entry)?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("md") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    let raw = io(&path, fs::read_to_string(&path))?;
                    out.insert(stem.to_string(), raw);
                }
            }
        }
        Ok(out)
    }

    /// Write a raw reflection document.
    pub fn write_reflection_raw(&self, run: &str, id: &str, content: &str) -> Result<()> {
        let dir = self.reflections_dir(run);
        io(&dir, fs::create_dir_all(&dir))?;
        let path = dir.join(format!("{id}.md"));
        io(&path, fs::write(&path, content))
    }

    /// The `witnesses.json` path for a run — the drift-sweep baseline.
    pub fn witnesses_path(&self, slug: &str) -> PathBuf {
        self.run_dir(slug).join("witnesses.json")
    }

    /// Read a run's artifact witnesses (empty when none recorded).
    pub fn read_witnesses(&self, run: &str) -> Result<Vec<crate::witness::Witness>> {
        let path = self.witnesses_path(run);
        if !path.exists() {
            return Ok(Vec::new());
        }
        let raw = io(&path, fs::read_to_string(&path))?;
        Ok(serde_json::from_str(&raw)?)
    }

    /// Overwrite a run's artifact witnesses.
    pub fn write_witnesses(&self, run: &str, witnesses: &[crate::witness::Witness]) -> Result<()> {
        let dir = self.run_dir(run);
        io(&dir, fs::create_dir_all(&dir))?;
        let path = self.witnesses_path(run);
        let raw = serde_json::to_string_pretty(witnesses)?;
        io(&path, fs::write(&path, raw))
    }
}

/// Whether a run is in a terminal (completed) status.
pub fn run_is_complete(run: &Run) -> bool {
    matches!(run.frontmatter.status, Status::Completed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Station, StationPhase, Status};

    fn station(name: &str, status: Status, phase: StationPhase) -> Station {
        Station {
            station: name.to_string(),
            status,
            phase,
            checkpoint: None,
            branch: None,
            pr_ref: None,
            started_at: None,
            completed_at: None,
        }
    }

    fn factory() -> Vec<String> {
        ["frame", "specify", "shape", "build", "prove", "harden"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn ordered_stations_falls_back_to_factory_when_plan_empty() {
        let state = RunState::default();
        assert_eq!(state.ordered_stations(&factory()), factory());
    }

    #[test]
    fn ordered_stations_uses_plan_when_present() {
        let state = RunState {
            plan: vec!["build".to_string(), "prove".to_string()],
            ..Default::default()
        };
        assert_eq!(
            state.ordered_stations(&factory()),
            vec!["build".to_string(), "prove".to_string()]
        );
    }

    #[test]
    fn station_status_summary_derives_done_current_pending() {
        let mut state = RunState {
            active_station: "specify".to_string(),
            ..Default::default()
        };
        state.stations.insert(
            "frame".to_string(),
            station("frame", Status::Completed, StationPhase::Checkpoint),
        );
        state.stations.insert(
            "specify".to_string(),
            station("specify", Status::InProgress, StationPhase::Manufacture),
        );

        let summary = state.station_status_summary(&factory());
        // Preserves the factory's ordering across the full list.
        let order: Vec<&str> = summary.iter().map(|s| s.station.as_str()).collect();
        assert_eq!(
            order,
            vec!["frame", "specify", "shape", "build", "prove", "harden"]
        );

        // Recorded stations report their persisted status/phase…
        assert_eq!(summary[0].status, Status::Completed);
        assert_eq!(summary[0].phase, StationPhase::Checkpoint);
        assert_eq!(summary[1].status, Status::InProgress);
        assert_eq!(summary[1].phase, StationPhase::Manufacture);

        // …and not-yet-reached stations default to Pending/Spec.
        for s in &summary[2..] {
            assert_eq!(s.status, Status::Pending);
            assert_eq!(s.phase, StationPhase::Spec);
        }
    }

    #[test]
    fn new_hierarchy_fields_default_and_round_trip() {
        // Legacy state with none of the new fields still deserializes, with the
        // new fields taking their defaults.
        let legacy = r#"{"factory":"software","active_station":"build","stations":{}}"#;
        let state: RunState = serde_json::from_str(legacy).expect("legacy deserializes");
        assert!(!state.discrete, "discrete defaults to false");
        assert!(state.base_branch.is_none(), "base_branch defaults to None");

        // A station record without `branch` deserializes with branch = None.
        let st_json = r#"{"station":"build","status":"completed","phase":"checkpoint"}"#;
        let st: Station = serde_json::from_str(st_json).expect("legacy station deserializes");
        assert!(st.branch.is_none());

        // Round-trip the new fields when set.
        let mut state = RunState {
            discrete: true,
            base_branch: Some("trunk".into()),
            ..Default::default()
        };
        state.stations.insert(
            "build".into(),
            station("build", Status::InProgress, StationPhase::Manufacture),
        );
        if let Some(b) = state.stations.get_mut("build") {
            b.branch = Some("darkrun/r/build".into());
        }
        let json = serde_json::to_string(&state).unwrap();
        let back: RunState = serde_json::from_str(&json).unwrap();
        assert!(back.discrete);
        assert_eq!(back.base_branch.as_deref(), Some("trunk"));
        assert_eq!(
            back.stations.get("build").unwrap().branch.as_deref(),
            Some("darkrun/r/build")
        );

        // Defaults are skipped on the wire (no migration churn for existing state).
        let plain = RunState::default();
        let plain_json = serde_json::to_string(&plain).unwrap();
        assert!(!plain_json.contains("discrete"));
        assert!(!plain_json.contains("base_branch"));
    }

    #[test]
    fn active_phase_resolves_the_active_station_phase() {
        let mut state = RunState {
            active_station: "shape".to_string(),
            ..Default::default()
        };
        // No entry yet → Spec default.
        assert_eq!(state.active_phase(), StationPhase::Spec);

        state.stations.insert(
            "shape".to_string(),
            station("shape", Status::InProgress, StationPhase::Audit),
        );
        assert_eq!(state.active_phase(), StationPhase::Audit);
    }
}
