//! Filesystem state engine.
//!
//! State is filesystem-only (no DB). The layout under a repo root lives
//! under `.darkrun/`:
//!
//! ```text
//! .darkrun/<run>/
//!   run.md                        frontmatter + body for the Run
//!   units/<slug>.md               one markdown doc per Unit
//!   state.json                    derived station/run state snapshot
//!   feedback/*.md                 feedback items (frontmatter + body)
//!   proof.json                    attached objective-evidence proofs, if any
//!   interactive/<station>/*.json  raised operator sessions (question/direction/picker)
//! ```
//!
//! A run being CONFIGURED carries a `setup` block in `run.md`'s frontmatter (its
//! factory/mode/size, elicited via pickers); the block is dropped once the run
//! starts. Interactive sessions are ALSO held in an in-memory registry shared by
//! the in-process MCP + HTTP servers (for live WebSocket updates); the on-disk
//! copies under `interactive/` let an open prompt and its answer survive a
//! restart.
//!
//! [`StateStore`] reads and writes this layout. It does not interpret the
//! manager's walk — it only persists and resolves the durable shapes.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::domain::{
    Mode, Run, RunFrontmatter, Station, StationPhase, Status, Unit, UnitFrontmatter,
};
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
    /// The run's global review mode (team/solo/dark), snapshotted at run start so
    /// the (pure) cursor resolves every station's gate ([`Mode::gate`]) without
    /// re-reading the run frontmatter. `team` opens a per-station PR the human
    /// merges; `solo` asks for local review; `dark` runs without review stops.
    /// The branch hierarchy is universal regardless of mode.
    #[serde(default)]
    pub mode: Mode,
    /// The base branch this run's `darkrun/<slug>/main` forked from, snapshotted
    /// at run start so the run-completion land has a stable target even if
    /// `settings.yml` changes mid-run. Absent on legacy state (resolved live).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_branch: Option<String>,
    /// Per-station derived state, keyed by station name.
    #[serde(default)]
    pub stations: BTreeMap<String, Station>,
    /// Whole-Run reviewer sign-offs, keyed by run-reviewer role (`None` =
    /// unsigned). After the final station locks, the run holds in a run-level
    /// review until every declared run reviewer is stamped — the cross-station
    /// audit of the integrated result before the run seals.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub run_reviews: BTreeMap<String, Option<crate::domain::Stamp>>,
    /// The engine version this run was created with — stamped immutably at run
    /// start, never overwritten. Pure **provenance** — which plugin/engine build
    /// authored the run — distinct from the on-disk shape, which
    /// [`schema_version`](RunState::schema_version) tracks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_with_version: Option<String>,
    /// The on-disk STATE-SHAPE version this run was written in — versioned
    /// **separately from the plugin** so the state format can evolve at its own
    /// cadence. On-read shape migrators key on this, not the plugin version. A
    /// run with no recorded value predates the stamp and is treated as
    /// [`SCHEMA_VERSION_LEGACY`] (the pre-versioning baseline).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<u32>,
}

/// The version stamped onto a run whose on-disk state predates the
/// `created_with_version` field — it was created by an engine old enough not to
/// record one. Distinct from `None` (never read/migrated) and from any real
/// semver (a versioned run).
pub const LEGACY_VERSION: &str = "legacy";

/// The current on-disk state-shape version. Bumped (and a matching migrator
/// added to [`migrate_state`]) whenever the persisted `RunState`/`state.json`
/// shape changes in a way old runs must be carried across. Versioned
/// independently of the plugin/engine semver.
pub const SCHEMA_VERSION: u32 = 1;

/// The shape version assigned to a run whose state predates schema versioning —
/// the pre-1 baseline. Migrators run for any run at a version below
/// [`SCHEMA_VERSION`].
pub const SCHEMA_VERSION_LEGACY: u32 = 0;

/// A single step migrator: transforms the RAW `state.json` value from one schema
/// version to the next-higher one. Running on the JSON (not the typed struct)
/// before deserialization is what lets a step rename/restructure fields an old
/// doc carries that the current `RunState` shape no longer accepts.
type StateMigrator = fn(&mut serde_json::Value);

/// The ordered migrator chain — index `i` migrates schema version `i` → `i+1`.
/// To add a real shape change: bump [`SCHEMA_VERSION`] and append the matching
/// `v<N>_to_v<N+1>` step here. The chain length always equals [`SCHEMA_VERSION`].
const STATE_MIGRATORS: &[StateMigrator] = &[migrate_state_v0_to_v1];

/// v0 → v1: the pre-versioning baseline. A legacy doc carries no engine
/// provenance, so stamp [`LEGACY_VERSION`] (its origin would otherwise be lost).
/// A real, registered step — the template every future shape migrator follows.
fn migrate_state_v0_to_v1(value: &mut serde_json::Value) {
    if let Some(obj) = value.as_object_mut() {
        let needs = obj
            .get("created_with_version")
            .map(|v| v.is_null())
            .unwrap_or(true);
        if needs {
            obj.insert(
                "created_with_version".to_string(),
                serde_json::Value::String(LEGACY_VERSION.to_string()),
            );
        }
    }
}

/// Migrate a raw `state.json` value forward to [`SCHEMA_VERSION`] on read, by
/// walking [`STATE_MIGRATORS`] from the doc's recorded version. Idempotent and
/// pure: a doc already at the current version runs no steps. Stamps the version
/// after migrating so the next read is a no-op.
fn migrate_state_value(value: &mut serde_json::Value) {
    let from = value
        .get("schema_version")
        .and_then(|v| v.as_u64())
        .unwrap_or(SCHEMA_VERSION_LEGACY as u64) as u32;
    // Apply every step whose source version is at-or-after the doc's version,
    // in ascending order — so a doc at version K runs steps K, K+1, … to latest.
    for (i, step) in STATE_MIGRATORS.iter().enumerate() {
        if from <= i as u32 {
            step(value);
        }
    }
    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "schema_version".to_string(),
            serde_json::Value::from(SCHEMA_VERSION),
        );
    }
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
        let ordered = self.ordered_stations(factory_stations);
        // The per-station lifecycle status is the index-relative ordering shared
        // by every surface ([`crate::derive::station_status`]) — completed before
        // the active station, active at it, pending after. (The phase still comes
        // from the recorded snapshot until the engine stamps the per-unit signals
        // the pure phase derivation reads.)
        let active_index = ordered.iter().position(|s| s == &self.active_station);
        ordered
            .iter()
            .enumerate()
            .map(|(i, name)| {
                // Completed-before / Pending-after ordering is the shared
                // index-relative derivation; the ACTIVE station keeps its recorded
                // status so a `Blocked`/`InProgress` nuance isn't flattened to
                // `Active`.
                let status = match crate::derive::station_status(i, active_index) {
                    Status::Active => self
                        .stations
                        .get(name)
                        .map(|st| st.status)
                        .unwrap_or(Status::Active),
                    ordered_status => ordered_status,
                };
                let phase = self
                    .stations
                    .get(name)
                    .map(|st| st.phase)
                    .unwrap_or(StationPhase::Spec);
                StationStatus {
                    station: name.clone(),
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

/// Reduce an arbitrary label (e.g. a station name) to a single safe path
/// component: alphanumerics, `-`, `_` survive; everything else becomes `-`. An
/// empty result falls back to `_`.
fn sanitize_component(s: &str) -> String {
    let out: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    if out.is_empty() {
        "_".to_string()
    } else {
        out
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

    /// Append one line to a run-scoped append-only journal (e.g.
    /// `action-log.jsonl`). Creates the run dir + file as needed. The trailing
    /// newline is added. Used for the audit trail the reflection pass and the
    /// operator read — every resolved action, in order, never rewritten.
    pub fn append_journal(&self, slug: &str, file: &str, line: &str) -> Result<()> {
        use std::io::Write;
        let dir = self.run_dir(slug);
        io(&dir, fs::create_dir_all(&dir))?;
        let path = dir.join(file);
        let mut f = io(&path, fs::OpenOptions::new().create(true).append(true).open(&path))?;
        io(&path, writeln!(f, "{line}"))?;
        Ok(())
    }

    /// Read a run-scoped journal's lines (empty when the file is absent).
    pub fn read_journal(&self, slug: &str, file: &str) -> Vec<String> {
        let path = self.run_dir(slug).join(file);
        fs::read_to_string(&path)
            .map(|s| s.lines().map(str::to_string).collect())
            .unwrap_or_default()
    }

    /// Read a run's in-flight [`RunSetup`] block from its frontmatter, if the
    /// run is still being configured (factory/mode/size not all chosen). `None`
    /// once the run has started, or when the run doc doesn't exist / can't parse.
    pub fn read_run_setup(&self, slug: &str) -> Option<crate::domain::RunSetup> {
        self.read_run(slug).ok()?.frontmatter.setup
    }

    /// Record one setup selection (`factory` / `mode` / `size`) onto a run's
    /// frontmatter setup block. A no-op when the run isn't in setup or the kind
    /// is unknown. Returns the updated block (so a caller can check completeness).
    pub fn set_run_setup_selection(
        &self,
        slug: &str,
        kind: &str,
        value: &str,
    ) -> Result<Option<crate::domain::RunSetup>> {
        let Ok(mut run) = self.read_run(slug) else {
            return Ok(None);
        };
        let Some(setup) = run.frontmatter.setup.as_mut() else {
            return Ok(None);
        };
        match kind {
            "factory" => setup.factory = Some(value.to_string()),
            "mode" => setup.mode = Some(value.to_string()),
            "size" => setup.size = Some(value.to_string()),
            _ => return Ok(Some(setup.clone())),
        }
        let updated = setup.clone();
        self.write_run(&run)?;
        Ok(Some(updated))
    }

    /// The `feedback/` directory for a run.
    pub fn feedback_dir(&self, slug: &str) -> PathBuf {
        self.run_dir(slug).join("feedback")
    }

    /// The `interactive/` root for a run. Operator sessions (questions,
    /// directions, pickers) persist UNDER a per-STATION subdirectory
    /// (`interactive/<station>/<session_id>.json`) so they belong to the
    /// station that raised them: only the active station's open prompt
    /// resurfaces, and a station reset clears its own prompts. Sessions survive
    /// an engine restart and reappear when the desktop reconnects.
    pub fn interactive_dir(&self, slug: &str) -> PathBuf {
        self.run_dir(slug).join("interactive")
    }

    /// The per-station interactive directory (`interactive/<station>/`).
    pub fn interactive_station_dir(&self, slug: &str, station: &str) -> PathBuf {
        self.interactive_dir(slug).join(sanitize_component(station))
    }

    /// The station subdir a given session id already lives under, if any — so a
    /// re-persist (recording the answer) lands in the SAME station it was
    /// raised under, even if the run has since advanced.
    fn interactive_station_of(&self, slug: &str, session_id: &str) -> Option<String> {
        let root = self.interactive_dir(slug);
        let file = format!("{session_id}.json");
        let entries = fs::read_dir(&root).ok()?;
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() && p.join(&file).is_file() {
                return p.file_name().and_then(|n| n.to_str()).map(str::to_string);
            }
        }
        None
    }

    /// Persist an interactive operator session: a [`SessionPayload`] that is a
    /// Question / Direction / Picker carrying a `run_slug`. Both the raise and
    /// the answer flow through here, so the on-disk copy always reflects the
    /// latest state (including the recorded answer). A non-interactive payload,
    /// or one with no run, is a silent no-op.
    ///
    /// The station it lands under is: the one it was already persisted under
    /// (so an answer re-writes in place), else the run's active station, else a
    /// `_run` fallback when no state is readable.
    pub fn write_interactive_session(&self, payload: &darkrun_api::SessionPayload) -> Result<()> {
        let Some(meta) = payload.interactive() else {
            return Ok(());
        };
        let Some(run) = meta.run else { return Ok(()) };
        let station = self
            .interactive_station_of(run, payload.session_id())
            .or_else(|| {
                self.read_state(run)
                    .ok()
                    .flatten()
                    .map(|s| s.active_station)
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or_else(|| "_run".to_string());
        let dir = self.interactive_station_dir(run, &station);
        io(&dir, fs::create_dir_all(&dir))?;
        let path = dir.join(format!("{}.json", payload.session_id()));
        let json = serde_json::to_vec_pretty(payload)?;
        io(&path, fs::write(&path, json))?;
        Ok(())
    }

    /// Read EVERY persisted interactive session for a run across all stations
    /// (newest id first — ids are monotonic `q-NN`/`d-NN`/`p-NN`), plus any
    /// legacy flat `interactive/*.json` files from before per-station scoping.
    /// Used to re-attach all sessions to the registry on reconnect/restart so
    /// each remains answerable. Unparseable files are skipped.
    pub fn list_interactive_sessions(&self, slug: &str) -> Vec<darkrun_api::SessionPayload> {
        let root = self.interactive_dir(slug);
        let Ok(entries) = fs::read_dir(&root) else {
            return Vec::new();
        };
        let mut files: Vec<PathBuf> = Vec::new();
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                // A station subdir: collect its session files.
                if let Ok(inner) = fs::read_dir(&p) {
                    files.extend(
                        inner
                            .flatten()
                            .map(|e| e.path())
                            .filter(|f| f.extension().is_some_and(|x| x == "json")),
                    );
                }
            } else if p.extension().is_some_and(|x| x == "json") {
                // A legacy flat file (pre per-station scoping).
                files.push(p);
            }
        }
        // Sort by file name (the monotonic session id), newest first.
        files.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
        files
            .iter()
            .filter_map(|p| fs::read(p).ok())
            .filter_map(|b| serde_json::from_slice(&b).ok())
            .collect()
    }

    /// The most recent OPEN (still-awaiting) interactive session for a run's
    /// given STATION — the one the desktop surfaces when it opens the run. Also
    /// considers legacy flat files (which carry no station) so a prompt raised
    /// before per-station scoping still resurfaces.
    pub fn latest_open_interactive(
        &self,
        slug: &str,
        station: &str,
    ) -> Option<darkrun_api::SessionPayload> {
        // The station's own sessions, newest first.
        let dir = self.interactive_station_dir(slug, station);
        let mut files: Vec<PathBuf> = fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect();
        // Plus legacy flat files at the interactive root (no station recorded).
        if let Ok(root) = fs::read_dir(self.interactive_dir(slug)) {
            files.extend(
                root.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "json")),
            );
        }
        files.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
        files
            .iter()
            .filter_map(|p| fs::read(p).ok())
            .filter_map(|b| serde_json::from_slice::<darkrun_api::SessionPayload>(&b).ok())
            .find(|p| p.interactive().is_some_and(|m| m.open))
    }

    /// Remove all interactive sessions raised under a station (its prompts go
    /// with the station when it is reset/dropped). Best-effort; a missing dir
    /// is a no-op.
    pub fn clear_station_interactive(&self, slug: &str, station: &str) -> Result<()> {
        let dir = self.interactive_station_dir(slug, station);
        if dir.exists() {
            io(&dir, fs::remove_dir_all(&dir))?;
        }
        Ok(())
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
    ///
    /// Runs the on-read migration: state written before the
    /// [`created_with_version`](RunState::created_with_version) stamp existed is
    /// tagged [`LEGACY_VERSION`] so downstream shape-migrators have a stable
    /// "born-in" version to key on. The migration never persists on its own —
    /// the next `write_state` records it — so a read is still side-effect free.
    pub fn read_state(&self, run: &str) -> Result<Option<RunState>> {
        let path = self.run_dir(run).join("state.json");
        if !path.exists() {
            return Ok(None);
        }
        let raw = io(&path, fs::read_to_string(&path))?;
        // Migrate at the JSON level BEFORE deserialization, so a step can
        // restructure fields the current `RunState` shape no longer accepts.
        let mut value: serde_json::Value = serde_json::from_str(&raw)?;
        migrate_state_value(&mut value);
        let state: RunState = serde_json::from_value(value)?;
        Ok(Some(state))
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

    /// The PROJECT-level knowledge directory — `.darkrun/knowledge/`, a sibling
    /// of the run dirs, NOT scoped to any one run. Explorers persist durable
    /// project knowledge here (constraints, prior art, traps) so it carries
    /// across runs as shared memory the next run's Spec reads as priors.
    pub fn knowledge_dir(&self) -> PathBuf {
        self.root.join("knowledge")
    }

    /// Read every project knowledge document, keyed by topic id (sorted).
    pub fn read_knowledge_raw(&self) -> Result<BTreeMap<String, String>> {
        let dir = self.knowledge_dir();
        let mut out = BTreeMap::new();
        if !dir.exists() {
            return Ok(out);
        }
        for entry in io(&dir, fs::read_dir(&dir))? {
            let path = io(&dir, entry)?.path();
            if path.extension().and_then(|e| e.to_str()) == Some("md") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    let raw = io(&path, fs::read_to_string(&path))?;
                    out.insert(stem.to_string(), raw);
                }
            }
        }
        Ok(out)
    }

    /// Read one project knowledge document by topic id, or `None` when absent.
    pub fn read_knowledge_entry(&self, topic: &str) -> Result<Option<String>> {
        let path = self.knowledge_dir().join(format!("{topic}.md"));
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(io(&path, fs::read_to_string(&path))?))
    }

    /// Write a project knowledge document (`<topic>.md`), overwriting in place
    /// so re-recording a topic updates the shared prior rather than duplicating.
    pub fn write_knowledge_raw(&self, topic: &str, content: &str) -> Result<()> {
        let dir = self.knowledge_dir();
        io(&dir, fs::create_dir_all(&dir))?;
        let path = dir.join(format!("{topic}.md"));
        io(&path, fs::write(&path, content))
    }

    /// The `briefs/` directory for a run — where each station's pre-execution
    /// brief (`<station>-pre.md`, "what I'm going to do", before the review
    /// gate) and closing outcome (`<station>-post.md`, "what the station
    /// produced", before the checkpoint) are persisted.
    pub fn briefs_dir(&self, run: &str) -> PathBuf {
        self.run_dir(run).join("briefs")
    }

    /// Read every brief/outcome document for a run, keyed by id (sorted).
    pub fn read_briefs_raw(&self, run: &str) -> Result<BTreeMap<String, String>> {
        let dir = self.briefs_dir(run);
        let mut out = BTreeMap::new();
        if !dir.exists() {
            return Ok(out);
        }
        for entry in io(&dir, fs::read_dir(&dir))? {
            let path = io(&dir, entry)?.path();
            if path.extension().and_then(|e| e.to_str()) == Some("md") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    let raw = io(&path, fs::read_to_string(&path))?;
                    out.insert(stem.to_string(), raw);
                }
            }
        }
        Ok(out)
    }

    /// Write a raw brief/outcome document (`<id>.md`).
    pub fn write_brief_raw(&self, run: &str, id: &str, content: &str) -> Result<()> {
        let dir = self.briefs_dir(run);
        io(&dir, fs::create_dir_all(&dir))?;
        let path = dir.join(format!("{id}.md"));
        io(&path, fs::write(&path, content))
    }

    /// The `prompts/` directory for a run — where every rendered engine prompt
    /// is persisted for inspection / replay (a durable record of exactly what
    /// the engine handed the agent at each step).
    pub fn prompts_dir(&self, run: &str) -> PathBuf {
        self.run_dir(run).join("prompts")
    }

    /// Persist a rendered prompt under `prompts/<scope>/<label>.md` (a stable
    /// path overwritten each time that action re-renders, so the dir holds the
    /// current prompt per station/phase rather than growing unbounded). `scope`
    /// is the station slug (or `_run` for run-level actions); `label` the action
    /// tag. Both are sanitized to a single safe path component.
    pub fn write_prompt(&self, run: &str, scope: &str, label: &str, body: &str) -> Result<()> {
        let safe = |s: &str| -> String {
            let s: String = s
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
                .collect();
            if s.is_empty() { "_".to_string() } else { s }
        };
        let dir = self.prompts_dir(run).join(safe(scope));
        io(&dir, fs::create_dir_all(&dir))?;
        let path = dir.join(format!("{}.md", safe(label)));
        io(&path, fs::write(&path, body))
    }

    /// Read every persisted prompt for a run, keyed by its `<scope>/<label>`
    /// relative path (sorted). Used by tests + replay tooling.
    pub fn read_prompts(&self, run: &str) -> Result<BTreeMap<String, String>> {
        let root = self.prompts_dir(run);
        let mut out = BTreeMap::new();
        if !root.exists() {
            return Ok(out);
        }
        for scope in io(&root, fs::read_dir(&root))? {
            let scope = io(&root, scope)?.path();
            if !scope.is_dir() {
                continue;
            }
            let Some(scope_name) = scope.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            for entry in io(&scope, fs::read_dir(&scope))? {
                let path = io(&scope, entry)?.path();
                if path.extension().and_then(|e| e.to_str()) == Some("md") {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        let raw = io(&path, fs::read_to_string(&path))?;
                        out.insert(format!("{scope_name}/{stem}"), raw);
                    }
                }
            }
        }
        Ok(out)
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
        }
    }

    fn factory() -> Vec<String> {
        ["frame", "specify", "shape", "build", "prove", "harden"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn reading_legacy_state_stamps_the_legacy_version() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new(dir.path());
        // Write a state.json that predates the version stamp (no field).
        let legacy = RunState {
            factory: "software".into(),
            active_station: "frame".into(),
            ..Default::default()
        };
        store.write_state("r", &legacy).unwrap();
        // Strip the field from disk to model genuine legacy state.
        let path = store.run_dir("r").join("state.json");
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("created_with_version"), "default omits it");

        let back = store.read_state("r").unwrap().unwrap();
        assert_eq!(back.created_with_version.as_deref(), Some(LEGACY_VERSION));
        // Legacy state had no schema version → migrated up to the current shape.
        assert_eq!(back.schema_version, Some(SCHEMA_VERSION));
    }

    #[test]
    fn a_versioned_run_keeps_its_plugin_provenance_on_read() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new(dir.path());
        let state = RunState {
            created_with_version: Some("9.9.9".into()),
            schema_version: Some(SCHEMA_VERSION),
            ..Default::default()
        };
        store.write_state("r", &state).unwrap();
        let back = store.read_state("r").unwrap().unwrap();
        // Plugin provenance is preserved verbatim; schema version is independent.
        assert_eq!(back.created_with_version.as_deref(), Some("9.9.9"));
        assert_eq!(back.schema_version, Some(SCHEMA_VERSION));
    }

    /// Gap #18: the migrator chain runs on the RAW json before deserialization,
    /// applies its registered steps from the doc's version, stamps the result,
    /// and is idempotent.
    #[test]
    fn migrate_state_value_applies_the_chain_and_is_idempotent() {
        use serde_json::json;
        // A legacy doc: no schema_version, no provenance — the v0→v1 step stamps
        // provenance and the chain stamps the version.
        let mut v = json!({ "factory": "software", "active_station": "frame" });
        migrate_state_value(&mut v);
        assert_eq!(v["created_with_version"], json!(LEGACY_VERSION));
        assert_eq!(v["schema_version"], json!(SCHEMA_VERSION));

        // Idempotent: re-running migrates nothing and preserves provenance.
        let before = v.clone();
        migrate_state_value(&mut v);
        assert_eq!(v, before, "a doc already at the current version is unchanged");

        // A doc that already carries provenance keeps it (the step is a no-op).
        let mut keep = json!({ "created_with_version": "7.7.7", "schema_version": 0 });
        migrate_state_value(&mut keep);
        assert_eq!(keep["created_with_version"], json!("7.7.7"));
        assert_eq!(keep["schema_version"], json!(SCHEMA_VERSION));

        // The chain length equals SCHEMA_VERSION — every version has its step.
        assert_eq!(STATE_MIGRATORS.len(), SCHEMA_VERSION as usize);
    }

    /// A raw legacy `state.json` carrying a now-deprecated field still reads:
    /// JSON-level migration runs, the version is stamped, and the doc
    /// deserializes (the value real shape migrators add over typed migration).
    #[test]
    fn read_state_migrates_a_raw_legacy_doc_with_an_unknown_field() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new(dir.path());
        let path = store.run_dir("r").join("state.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // Hand-write a pre-versioning doc with a deprecated key.
        std::fs::write(
            &path,
            r#"{"factory":"software","active_station":"frame","deprecated_legacy_field":true}"#,
        )
        .unwrap();
        let back = store.read_state("r").unwrap().unwrap();
        assert_eq!(back.factory, "software");
        assert_eq!(back.created_with_version.as_deref(), Some(LEGACY_VERSION));
        assert_eq!(back.schema_version, Some(SCHEMA_VERSION));
    }

    #[test]
    fn schema_version_is_independent_of_the_plugin_version() {
        // A run can carry an old plugin provenance but be migrated to the
        // current on-disk schema — the two version axes are separate.
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new(dir.path());
        let state = RunState {
            created_with_version: Some("0.0.1".into()),
            schema_version: None, // pre-schema-versioning
            ..Default::default()
        };
        store.write_state("r", &state).unwrap();
        let back = store.read_state("r").unwrap().unwrap();
        assert_eq!(back.created_with_version.as_deref(), Some("0.0.1"));
        assert_eq!(back.schema_version, Some(SCHEMA_VERSION));
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
        assert_eq!(state.mode, Mode::Solo, "mode defaults to solo");
        assert!(state.base_branch.is_none(), "base_branch defaults to None");

        // A station record without `branch` deserializes with branch = None.
        let st_json = r#"{"station":"build","status":"completed","phase":"checkpoint"}"#;
        let st: Station = serde_json::from_str(st_json).expect("legacy station deserializes");
        assert!(st.branch.is_none());

        // Round-trip the new fields when set.
        let mut state = RunState {
            mode: Mode::Team,
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
        assert_eq!(back.mode, Mode::Team);
        assert_eq!(back.base_branch.as_deref(), Some("trunk"));
        assert_eq!(
            back.stations.get("build").unwrap().branch.as_deref(),
            Some("darkrun/r/build")
        );

        // Optional defaults are skipped on the wire (no migration churn).
        let plain = RunState::default();
        let plain_json = serde_json::to_string(&plain).unwrap();
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

    #[test]
    fn io_helper_tags_errors_with_their_path_and_passes_ok_through() {
        use std::path::Path;
        let err = io::<()>(Path::new("/some/where"), Err(std::io::Error::other("boom"))).unwrap_err();
        assert!(matches!(err, CoreError::Io { .. }));
        assert_eq!(io(Path::new("/x"), Ok(42)).unwrap(), 42);
    }

    #[test]
    fn run_setup_block_round_trips_and_tracks_first_unset() {
        use crate::domain::{Run, RunFrontmatter, RunSetup};
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new(dir.path());

        // No run -> no setup block.
        assert!(store.read_run_setup("r").is_none());

        // A run created with an empty setup block lists immediately.
        store
            .write_run(&Run {
                slug: "r".into(),
                title: "R".into(),
                body: String::new(),
                frontmatter: RunFrontmatter {
                    title: Some("R".into()),
                    setup: Some(RunSetup::default()),
                    ..Default::default()
                },
            })
            .unwrap();
        assert_eq!(store.read_run_setup("r").unwrap().first_unset(), Some("factory"));

        // Selections land in order on the frontmatter; first_unset advances.
        store.set_run_setup_selection("r", "factory", "software").unwrap();
        assert_eq!(store.read_run_setup("r").unwrap().first_unset(), Some("mode"));
        store.set_run_setup_selection("r", "mode", "solo").unwrap();
        assert_eq!(store.read_run_setup("r").unwrap().first_unset(), Some("size"));
        let full = store.set_run_setup_selection("r", "size", "full").unwrap().unwrap();
        assert!(full.is_complete());

        // An unknown kind is a no-op; a run with no setup block returns None.
        store.set_run_setup_selection("r", "bogus", "x").unwrap();
        assert!(store.read_run_setup("r").unwrap().is_complete());
        assert!(store.set_run_setup_selection("gone", "mode", "solo").unwrap().is_none());
    }

    #[test]
    fn interactive_sessions_round_trip_and_track_open_state() {
        use darkrun_api::common::SessionStatus;
        use darkrun_api::{QuestionSessionPayload, SessionPayload};

        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new(dir.path());
        // The run is at the `build` station, so prompts land under build/.
        store
            .write_state(
                "r",
                &RunState { active_station: "build".into(), ..Default::default() },
            )
            .unwrap();

        let q = |id: &str, status| {
            SessionPayload::Question(QuestionSessionPayload {
                session_id: id.into(),
                status,
                run_slug: Some("r".into()),
                prompt: "pick".into(),
                ..Default::default()
            })
        };

        // No sessions yet.
        assert!(store.list_interactive_sessions("r").is_empty());
        assert!(store.latest_open_interactive("r", "build").is_none());

        // Two raised at build, q-02 the newer; both open.
        store.write_interactive_session(&q("q-01", SessionStatus::Pending)).unwrap();
        store.write_interactive_session(&q("q-02", SessionStatus::Pending)).unwrap();
        assert_eq!(store.list_interactive_sessions("r").len(), 2);
        // They live under the station subdir.
        assert!(store.interactive_station_dir("r", "build").join("q-01.json").is_file());
        // Newest open first.
        assert_eq!(
            store.latest_open_interactive("r", "build").unwrap().session_id(),
            "q-02"
        );
        // A DIFFERENT station has no open prompt of its own.
        assert!(store.latest_open_interactive("r", "frame").is_none());

        // Answering q-02 re-writes IN PLACE (same station) and leaves q-01 open
        // — even though the run has since moved to `prove`.
        store
            .write_state(
                "r",
                &RunState { active_station: "prove".into(), ..Default::default() },
            )
            .unwrap();
        store.write_interactive_session(&q("q-02", SessionStatus::Answered)).unwrap();
        assert!(
            store.interactive_station_dir("r", "build").join("q-02.json").is_file(),
            "the answer re-wrote under the original station, not the active one"
        );
        assert_eq!(
            store.latest_open_interactive("r", "build").unwrap().session_id(),
            "q-01",
            "an answered session is no longer the open surface"
        );

        // Resetting the build station takes its prompts with it.
        store.clear_station_interactive("r", "build").unwrap();
        assert!(store.list_interactive_sessions("r").is_empty());

        // A run-less payload is a no-op.
        store
            .write_interactive_session(&SessionPayload::Question(QuestionSessionPayload {
                session_id: "q-09".into(),
                run_slug: None,
                ..Default::default()
            }))
            .unwrap();
        assert!(store.list_interactive_sessions("r").is_empty());
    }
}
