//! The branch + worktree lifecycle the manager hooks into.
//!
//! darkrun drives a **universal** branch hierarchy — the same for every mode
//! (continuous, autopilot, quick, bugfix, refactor, full, and discrete):
//!
//! ```text
//! <base> < darkrun/<slug>/main < darkrun/<slug>/<station> < darkrun/<slug>/units/<station>/<unit>
//!                                                         \ darkrun/<slug>/fixes/<station>/<id>
//! ```
//!
//! - **run-main** (`darkrun/<slug>/main`) is the stable per-run base. It only
//!   ever accumulates *fully-verified* stations.
//! - **station branches** (`darkrun/<slug>/<station>`) fork off run-main when a
//!   station is entered, and carry that station's work on an isolated worktree.
//! - **unit branches** (`darkrun/<slug>/units/<station>/<unit>`) fork off the
//!   station branch when a unit's Pass-loop begins, isolate one unit's diff on
//!   its own worktree, and land back onto the station branch when the unit locks.
//! - **fix branches** (`darkrun/<slug>/fixes/<station>/<id>`) do the same for a
//!   drift/feedback repair, so a fix's diff never tangles with in-flight units.
//! - On station completion the station branch lands onto run-main through the
//!   [`engine_protected_merge`](darkrun_git::engine_protected_merge), and the
//!   station worktree + branch are removed.
//! - At run completion run-main lands onto the repo base (the run-completion
//!   PR/merge).
//!
//! Units and fixes live in `units/`/`fixes/` namespaces *parallel* to the station
//! branches rather than nested beneath them — a station branch is a leaf git ref,
//! so a child ref directly under it would hit git's directory/file ref conflict.
//!
//! The MODE only changes HOW the per-station checkpoint gate resolves (in-process
//! for the non-discrete modes here; a human PR merge for discrete — a later
//! phase). Per-station branching itself is always on.
//!
//! Every operation is **best-effort and non-fatal** — git failures never crash
//! the manager. Each fn returns a structured [`LifecycleOutcome`] and is a clean
//! no-op outside a git repo (the manager runs in plain tempdirs under test, and
//! many in-process desktop flows are not git-backed).

use std::path::{Path, PathBuf};

use darkrun_core::StateStore;
use darkrun_git::{engine_protected_merge, has_no_merge_debt, GitBackend, Git};

/// The branch prefix the engine forks run work onto (`darkrun/...`).
pub const BRANCH_PREFIX: &str = "darkrun";

/// The run's stable base branch: `darkrun/<slug>/main`.
pub fn run_main_branch(slug: &str) -> String {
    format!("{BRANCH_PREFIX}/{slug}/main")
}

/// A station's working branch: `darkrun/<slug>/<station>`.
pub fn station_branch(slug: &str, station: &str) -> String {
    format!("{BRANCH_PREFIX}/{slug}/{station}")
}

/// The on-disk worktree path for a station's branch, kept out of the way under
/// `<repo>/.darkrun/worktrees/<slug>/<station>`. (`.darkrun/` is gitignored, so
/// the nested worktree never pollutes the run's own state tree.)
pub fn station_worktree_path(repo_root: &Path, slug: &str, station: &str) -> PathBuf {
    repo_root
        .join(".darkrun")
        .join("worktrees")
        .join(slug)
        .join(station)
}

/// A unit's working branch: `darkrun/<slug>/units/<station>/<unit>`. A unit forks
/// off its station branch, carries one unit's Pass-loop on an isolated worktree,
/// and lands back onto the station branch when the unit locks — the same
/// fork/isolate/land discipline a station has against run-main, one level down.
///
/// Units live in a `units/` namespace *parallel* to the station branches, not
/// nested beneath `darkrun/<slug>/<station>`: a station branch is a leaf git ref,
/// so a child ref under it would hit git's directory/file ref conflict. The
/// parallel namespace keeps the readable hierarchy without that collision (no
/// station is ever named `units`).
pub fn unit_branch(slug: &str, station: &str, unit: &str) -> String {
    format!("{BRANCH_PREFIX}/{slug}/units/{station}/{unit}")
}

/// The on-disk worktree path for a unit's branch:
/// `<repo>/.darkrun/worktrees/<slug>/units/<station>/<unit>`.
pub fn unit_worktree_path(repo_root: &Path, slug: &str, station: &str, unit: &str) -> PathBuf {
    repo_root
        .join(".darkrun")
        .join("worktrees")
        .join(slug)
        .join("units")
        .join(station)
        .join(unit)
}

/// A fix-worker's working branch: `darkrun/<slug>/fixes/<station>/<id>`. A drift
/// or feedback repair forks off its station branch onto its own worktree so the
/// fix's diff is isolated from the station's in-flight work, then lands back onto
/// the station branch — the same isolation a unit gets, for a repair instead of a
/// fresh unit. Fixes live in a `fixes/` namespace parallel to the stations for
/// the same directory/file ref reason as [`unit_branch`].
pub fn fix_branch(slug: &str, station: &str, fix_id: &str) -> String {
    format!("{BRANCH_PREFIX}/{slug}/fixes/{}/{fix_id}", fix_station_component(station))
}

/// The station sentinel for a RUN-SCOPE feedback — a closeout / cross-station
/// finding that belongs to the run as a whole, fixed on run-main rather than a
/// station branch.
pub const RUN_SCOPE_STATION: &str = "_run";

/// Whether `station` denotes a run-scope fix (the `_run` sentinel, or empty).
pub fn is_run_scope_station(station: &str) -> bool {
    station.is_empty() || station == RUN_SCOPE_STATION
}

/// The path component for a fix's station — the station slug, or `_run` for a
/// run-scope feedback.
fn fix_station_component(station: &str) -> &str {
    if is_run_scope_station(station) {
        RUN_SCOPE_STATION
    } else {
        station
    }
}

/// The on-disk worktree path for a fix's branch:
/// `<repo>/.darkrun/worktrees/<slug>/fixes/<station|_run>/<id>`.
pub fn fix_worktree_path(repo_root: &Path, slug: &str, station: &str, fix_id: &str) -> PathBuf {
    repo_root
        .join(".darkrun")
        .join("worktrees")
        .join(slug)
        .join("fixes")
        .join(fix_station_component(station))
        .join(fix_id)
}

/// Resolve the base branch a run forks from — `default_branch` out of
/// `.darkrun/settings.yml`, defaulting to `main`. Parsed line-wise so this needs
/// no YAML dependency. The shared helper (also used by `runs::base_branch`).
pub fn resolve_base_branch(store: &StateStore) -> String {
    let raw = std::fs::read_to_string(store.root().join("settings.yml")).unwrap_or_default();
    for line in raw.lines() {
        if let Some(value) = line.trim().strip_prefix("default_branch:") {
            let value = value.trim().trim_matches(['"', '\'']).trim();
            if !value.is_empty() {
                return value.to_string();
            }
        }
    }
    "main".to_string()
}

/// Where a run's stable base branch (`darkrun/<slug>/main`) stands relative to
/// the repo's default branch (G4b). The run accumulates verified stations onto
/// run-main; this is whether that work has reached the default branch yet, and
/// whether the default branch has moved on underneath it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunMainStatus {
    /// run-main hasn't been forked yet (no run work branched), or the repo isn't
    /// git-backed.
    NotForked,
    /// run-main and the default branch are at the same commit — nothing to land.
    UpToDate,
    /// run-main has verified work the default branch doesn't have yet (the normal
    /// in-progress state, awaiting the run-completion land).
    Ahead,
    /// run-main is fully contained in the default branch and the default branch
    /// has advanced past it — the run's work has landed.
    Merged,
    /// run-main and the default branch each have commits the other lacks — they
    /// diverged; a downstream sync is needed before landing.
    Diverged,
}

/// Compute [`RunMainStatus`] for a run — where its `darkrun/<slug>/main` stands
/// against the default branch. Best-effort: returns `NotForked` outside a git
/// repo or when run-main doesn't exist. The base is the run's snapshotted
/// `base_branch` (falling back to the resolved default).
pub fn run_main_status(store: &StateStore, slug: &str) -> RunMainStatus {
    let Some((git, _root)) = open_git(store) else {
        return RunMainStatus::NotForked;
    };
    let run_main = run_main_branch(slug);
    if !git.branch_exists(&run_main).unwrap_or(false) {
        return RunMainStatus::NotForked;
    }
    let base = store
        .read_state(slug)
        .ok()
        .flatten()
        .and_then(|s| s.base_branch)
        .unwrap_or_else(|| resolve_base_branch(store));
    if !git.branch_exists(&base).unwrap_or(false) {
        return RunMainStatus::NotForked;
    }
    let base_in_main = git.is_ancestor(&base, &run_main).unwrap_or(false);
    let main_in_base = git.is_ancestor(&run_main, &base).unwrap_or(false);
    match (base_in_main, main_in_base) {
        (true, true) => RunMainStatus::UpToDate,
        (true, false) => RunMainStatus::Ahead,
        (false, true) => RunMainStatus::Merged,
        (false, false) => RunMainStatus::Diverged,
    }
}

/// Which downstream-sync step (mechanic #5) surfaced a conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncConflictStep {
    /// base/mainline -> run-main (step 1).
    MainlineToRunMain,
    /// run-main -> the active station branch (step 2).
    RunMainToStation,
}

impl SyncConflictStep {
    /// The stable tag used in notes / actions.
    pub fn as_str(&self) -> &'static str {
        match self {
            SyncConflictStep::MainlineToRunMain => "mainline_to_run_main",
            SyncConflictStep::RunMainToStation => "run_main_to_station",
        }
    }
}

/// The structured result of a lifecycle operation. Never an error from the
/// manager's perspective — `note` carries why a step was a no-op or partial.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LifecycleOutcome {
    /// Whether the operation performed a git mutation (vs a clean no-op).
    pub performed: bool,
    /// A human-readable note (no-op reason, conflict summary, branch landed).
    pub note: Option<String>,
    /// When a merge surfaced genuine agent-content conflicts, the paths.
    pub conflict_paths: Vec<String>,
    /// The branch the conflict is in-tree on (for the recovery to target).
    pub conflict_branch: Option<String>,
    /// Which downstream-sync step conflicted (mechanic #5), when applicable.
    pub conflict_step: Option<SyncConflictStep>,
}

impl LifecycleOutcome {
    fn noop(note: impl Into<String>) -> Self {
        Self {
            performed: false,
            note: Some(note.into()),
            ..Default::default()
        }
    }
    fn done(note: impl Into<String>) -> Self {
        Self {
            performed: true,
            note: Some(note.into()),
            ..Default::default()
        }
    }
    /// Whether this outcome carries unresolved agent-content conflicts.
    pub fn has_conflicts(&self) -> bool {
        !self.conflict_paths.is_empty()
    }
}

/// The repo root for a store — the parent of its `.darkrun` root (matching
/// `position::cascade_repo_root`). Worktrees and branches resolve against it.
fn repo_root(store: &StateStore) -> PathBuf {
    store
        .root()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| store.root().to_path_buf())
}

/// Open the repo as a [`Git`], or `None` when the store's repo root is not a git
/// repository (every lifecycle fn then no-ops).
fn open_git(store: &StateStore) -> Option<(Git, PathBuf)> {
    let root = repo_root(store);
    Git::open(&root).ok().map(|g| (g, root))
}

/// Fork `darkrun/<slug>/main` off the resolved base at run start. Idempotent:
/// a no-op when run-main already exists. No-op outside a git repo.
pub fn ensure_run_main(store: &StateStore, slug: &str) -> LifecycleOutcome {
    let Some((git, _root)) = open_git(store) else {
        return LifecycleOutcome::noop("not a git repo");
    };
    let run_main = run_main_branch(slug);
    if git.branch_exists(&run_main).unwrap_or(false) {
        return LifecycleOutcome::noop(format!("{run_main} already exists"));
    }
    let base = resolve_base_branch(store);
    // The base must resolve (an empty repo with no commit on `main` can't fork).
    if !git.branch_exists(&base).unwrap_or(false) {
        return LifecycleOutcome::noop(format!("base branch '{base}' not found; skipping fork"));
    }
    match git.create_branch(&run_main, &base) {
        Ok(()) => LifecycleOutcome::done(format!("forked {run_main} off {base}")),
        Err(e) => LifecycleOutcome::noop(format!("create {run_main} failed: {e}")),
    }
}

/// Enter a station: fork `darkrun/<slug>/<station>` off run-main and create a
/// worktree on it. Idempotent — skips the fork / worktree when they already
/// exist (crash-recovery: a re-entered station reuses its branch + worktree).
/// No-op outside a git repo.
///
/// Returns the station branch name (when on a git repo) in `note` so the caller
/// can stamp `Station.branch`.
pub fn enter_station(store: &StateStore, slug: &str, station: &str) -> LifecycleOutcome {
    let Some((git, root)) = open_git(store) else {
        return LifecycleOutcome::noop("not a git repo");
    };
    // run-main must exist to fork from; ensure it (idempotent) so a station can
    // never fork off the wrong base if run_start's ensure was skipped.
    let run_main = run_main_branch(slug);
    if !git.branch_exists(&run_main).unwrap_or(false) {
        let _ = ensure_run_main(store, slug);
    }
    if !git.branch_exists(&run_main).unwrap_or(false) {
        return LifecycleOutcome::noop(format!("{run_main} unavailable; cannot enter station"));
    }

    let branch = station_branch(slug, station);
    let wt_path = station_worktree_path(&root, slug, station);
    let outcome = fork_and_worktree(&git, &run_main, &branch, &wt_path, &format!("{slug}-{station}"));
    if outcome.performed {
        // Publish the freshly-forked station branch immediately (early and
        // often): origin carries the run's full hierarchy from birth, so the
        // web browse can read the current station off its own branch.
        let _ = crate::hosting::push_head_with_nff_recovery(&git, &wt_path, &branch);
    }
    outcome
}

/// Fork `child` off `parent` and attach a worktree at `wt_path` (registered as
/// `wt_name`). Idempotent: `create_branch` is a no-op when `child` exists, and
/// the worktree is created only when not already registered — so a re-entered
/// unit/fix/station reuses its branch + worktree (crash recovery). The caller
/// guarantees `parent` exists and `git` is open on the repo. Returns the child
/// branch name in `note` so the caller can stamp it onto state.
fn fork_and_worktree(
    git: &Git,
    parent: &str,
    child: &str,
    wt_path: &Path,
    wt_name: &str,
) -> LifecycleOutcome {
    let _ = git.create_branch(child, parent);
    let already = git
        .list_worktrees()
        .map(|ws| ws.iter().any(|w| w.path == *wt_path))
        .unwrap_or(false);
    if !already {
        if let Some(parent_dir) = wt_path.parent() {
            let _ = std::fs::create_dir_all(parent_dir);
        }
        let opts = darkrun_git::CreateOptions {
            reference: Some(child.to_string()),
            new_branch: None,
        };
        let _ = git.create_worktree(wt_name, wt_path, &opts);
    }
    LifecycleOutcome::done(child.to_string())
}

/// Enter a unit: fork `darkrun/<slug>/<station>/<unit>` off the station branch
/// and create its worktree. Idempotent (a re-entered unit reuses both). No-op
/// outside a git repo, or when the station branch doesn't exist yet (the station
/// hasn't been entered — the unit has no parent to fork from).
///
/// Returns the unit branch name in `note` on success so the caller can stamp
/// `Unit.branch`.
pub fn enter_unit(store: &StateStore, slug: &str, station: &str, unit: &str) -> LifecycleOutcome {
    let Some((git, root)) = open_git(store) else {
        return LifecycleOutcome::noop("not a git repo");
    };
    let parent = station_branch(slug, station);
    if !git.branch_exists(&parent).unwrap_or(false) {
        return LifecycleOutcome::noop(format!("{parent} unavailable; cannot enter unit"));
    }
    let branch = unit_branch(slug, station, unit);
    let wt_path = unit_worktree_path(&root, slug, station, unit);
    fork_and_worktree(
        &git,
        &parent,
        &branch,
        &wt_path,
        &format!("{slug}-{station}-{unit}"),
    )
}

/// Enter a fix: fork `darkrun/<slug>/<station>/fix-<id>` off the station branch
/// and create its worktree, so a drift/feedback repair's diff is isolated. Same
/// idempotency + no-op rules as [`enter_unit`].
///
/// Returns the fix branch name in `note` on success.
pub fn enter_fix(store: &StateStore, slug: &str, station: &str, fix_id: &str) -> LifecycleOutcome {
    let Some((git, root)) = open_git(store) else {
        return LifecycleOutcome::noop("not a git repo");
    };
    // A run-scope fix (`_run`) forks off run-main; a station fix off its branch.
    let parent = if is_run_scope_station(station) {
        run_main_branch(slug)
    } else {
        station_branch(slug, station)
    };
    if !git.branch_exists(&parent).unwrap_or(false) {
        return LifecycleOutcome::noop(format!("{parent} unavailable; cannot enter fix"));
    }
    let branch = fix_branch(slug, station, fix_id);
    let wt_path = fix_worktree_path(&root, slug, station, fix_id);
    let label = fix_station_component(station);
    fork_and_worktree(
        &git,
        &parent,
        &branch,
        &wt_path,
        &format!("{slug}-{label}-fix-{fix_id}"),
    )
}

/// Land a completed station: engine-protected merge `darkrun/<slug>/<station>`
/// -> run-main, then remove the station worktree + branch. Best-effort and
/// crash-tolerant (a missing worktree but unmerged branch still merges the
/// branch). No-op outside a git repo.
pub fn land_station(store: &StateStore, slug: &str, station: &str) -> LifecycleOutcome {
    let Some((git, root)) = open_git(store) else {
        return LifecycleOutcome::noop("not a git repo");
    };
    let branch = station_branch(slug, station);
    let run_main = run_main_branch(slug);
    let wt_path = station_worktree_path(&root, slug, station);
    land_child(
        store,
        &git,
        &root,
        slug,
        &run_main,
        &branch,
        &wt_path,
        &format!("{slug}-{station}"),
        &format!("darkrun: land station '{station}' -> {run_main}"),
    )
}

/// Retire a DROPPED station's branch + worktree without landing anything —
/// the keep-or-drop offer guarantees the station never started, so its fork
/// carries no work (it sits at run-main's commit). Best-effort; no-op outside
/// a git repo or when the branch never existed.
pub fn drop_station_branch(store: &StateStore, slug: &str, station: &str) -> LifecycleOutcome {
    let Some((git, root)) = open_git(store) else {
        return LifecycleOutcome::noop("not a git repo");
    };
    let branch = station_branch(slug, station);
    let wt_path = station_worktree_path(&root, slug, station);
    let _ = git.remove_worktree(&format!("{slug}-{station}"), true);
    let _ = std::fs::remove_dir_all(&wt_path);
    if git.branch_exists(&branch).unwrap_or(false) {
        let _ = git.delete_branch(&branch);
        LifecycleOutcome::done(format!("dropped {branch}"))
    } else {
        LifecycleOutcome::noop(format!("{branch} not found; nothing to drop"))
    }
}

/// Land a completed unit: engine-protected merge the unit branch -> its station
/// branch, then remove the unit worktree + branch. Same crash-tolerance as
/// [`land_station`], one level down (the parent is the station branch, not
/// run-main). No-op outside a git repo.
pub fn land_unit(store: &StateStore, slug: &str, station: &str, unit: &str) -> LifecycleOutcome {
    let Some((git, root)) = open_git(store) else {
        return LifecycleOutcome::noop("not a git repo");
    };
    let branch = unit_branch(slug, station, unit);
    let parent = station_branch(slug, station);
    let wt_path = unit_worktree_path(&root, slug, station, unit);
    land_child(
        store,
        &git,
        &root,
        slug,
        &parent,
        &branch,
        &wt_path,
        &format!("{slug}-{station}-{unit}"),
        &format!("darkrun: land unit '{unit}' -> {parent}"),
    )
}

/// Land a completed fix: engine-protected merge the fix branch -> its station
/// branch, then remove the fix worktree + branch. Mirrors [`land_unit`]. No-op
/// outside a git repo.
pub fn land_fix(store: &StateStore, slug: &str, station: &str, fix_id: &str) -> LifecycleOutcome {
    let Some((git, root)) = open_git(store) else {
        return LifecycleOutcome::noop("not a git repo");
    };
    let branch = fix_branch(slug, station, fix_id);
    // A RUN-SCOPE feedback (the `_run` sentinel) is a run-level / closeout
    // finding — its fix lands on run-main, not a station branch.
    let parent = if is_run_scope_station(station) {
        run_main_branch(slug)
    } else {
        station_branch(slug, station)
    };
    let wt_path = fix_worktree_path(&root, slug, station, fix_id);
    let label = fix_station_component(station);
    land_child(
        store,
        &git,
        &root,
        slug,
        &parent,
        &branch,
        &wt_path,
        &format!("{slug}-{label}-fix-{fix_id}"),
        &format!("darkrun: land fix '{fix_id}' -> {parent}"),
    )
}

/// The shared land discipline: engine-protected merge `child` -> `parent`, then
/// retire the child's worktree (always — its work is captured on the branch) and
/// the child branch (only on a clean land, so a conflicted child stays
/// recoverable). Crash-tolerant: a present branch with unmerged commits whose
/// worktree is gone still merges; an absent branch or an already-merged ancestor
/// short-circuits to a no-op after sweeping any leftover worktree/branch.
#[allow(clippy::too_many_arguments)]
fn land_child(
    store: &StateStore,
    git: &Git,
    root: &Path,
    slug: &str,
    parent: &str,
    branch: &str,
    wt_path: &Path,
    wt_name: &str,
    message: &str,
) -> LifecycleOutcome {
    // #8 "complete but never merged": short-circuit to a no-op ONLY when there
    // is genuinely nothing to merge — the branch is absent, OR it's already an
    // ancestor of the parent (its commits already landed). A present branch with
    // unmerged commits whose worktree happens to be gone must STILL merge below.
    if !git.branch_exists(branch).unwrap_or(false) {
        return LifecycleOutcome::noop(format!("{branch} not found; nothing to land"));
    }
    if !git.branch_exists(parent).unwrap_or(false) {
        return LifecycleOutcome::noop(format!("{parent} not found; cannot land"));
    }
    if git.is_ancestor(branch, parent).unwrap_or(false) {
        // Already merged (or an empty fork at the parent's commit) — truly
        // nothing to land. Retire any leftover worktree, then no-op.
        let _ = git.remove_worktree(wt_name, true);
        let _ = std::fs::remove_dir_all(wt_path);
        let _ = git.delete_branch(branch);
        return LifecycleOutcome::noop(format!("{branch} already in {parent}; nothing to land"));
    }

    // Merge through a worktree checked out on the parent so the primary checkout
    // is never touched.
    let outcome = merge_into_branch(store, git, root, parent, branch, slug, message);

    // Whether the merge succeeded or not, retire the child worktree (its work is
    // captured on the branch). Remove the branch only on a clean land so a
    // conflicted child is left recoverable.
    let _ = git.remove_worktree(wt_name, true);
    let _ = std::fs::remove_dir_all(wt_path);

    if outcome.performed || git.is_ancestor(branch, parent).unwrap_or(false) {
        let _ = git.delete_branch(branch);
    }
    outcome
}

/// Land the run: engine-protected merge run-main -> base at run completion (the
/// run-main -> base PR/merge). No-op outside a git repo.
pub fn land_run(store: &StateStore, slug: &str) -> LifecycleOutcome {
    let Some((git, root)) = open_git(store) else {
        return LifecycleOutcome::noop("not a git repo");
    };
    let run_main = run_main_branch(slug);
    let state = store.read_state(slug).ok().flatten();
    let base = state
        .and_then(|s| s.base_branch)
        .unwrap_or_else(|| resolve_base_branch(store));
    if !git.branch_exists(&run_main).unwrap_or(false) {
        return LifecycleOutcome::noop(format!("{run_main} not found; nothing to land"));
    }
    if !git.branch_exists(&base).unwrap_or(false) {
        return LifecycleOutcome::noop(format!("base '{base}' not found; cannot land run"));
    }
    merge_into_branch(
        store,
        &git,
        &root,
        &base,
        &run_main,
        slug,
        &format!("darkrun: land run '{slug}' -> {base}"),
    )
}

/// Where a merge should run: in-place on the primary/existing checkout of the
/// target, or in a detached temp worktree so the agent's tree is untouched.
enum MergeSite {
    /// Merge in-place at `path` (the engine is already on `target` there). When
    /// `path` is the repo root this is the primary checkout; otherwise it's a
    /// foreign clean worktree already on `target`. Either way `branch -f` would
    /// refuse, so we merge directly in that tree.
    InPlace { path: PathBuf },
    /// Merge in a detached temp worktree at `path`, then fast-update `target`.
    Temp { path: PathBuf },
    /// The target checkout is dirty and can't be reused — `note` names the fix.
    Refused { note: String },
}

/// Pick the merge site for `target` (mechanic #6).
///
/// - If a worktree (primary or foreign) is already on `target`, merge IN-PLACE
///   there — a detached temp worktree can't fast-update a branch ref that's
///   checked out elsewhere (`branch -f` refuses it). Refuse a dirty such tree,
///   naming the remediation.
/// - Otherwise no checkout holds `target`, so make a detached temp worktree at
///   `target`'s commit and merge there, leaving the agent's tree alone.
fn merge_site(git: &Git, root: &Path, target: &str, slug: &str) -> MergeSite {
    // Find any worktree already on `target` (primary or station/foreign).
    let existing = git
        .list_worktrees()
        .ok()
        .and_then(|ws| ws.into_iter().find(|w| w.branch.as_deref() == Some(target)));
    if let Some(wt) = existing {
        if worktree_is_dirty(&wt.path) {
            return MergeSite::Refused {
                note: format!(
                    "branch '{target}' is checked out at '{}' with uncommitted or untracked \
                     changes — commit or stash them (and add or clean untracked files) so the \
                     engine can merge into it",
                    wt.path.display()
                ),
            };
        }
        return MergeSite::InPlace { path: wt.path };
    }
    let merge_wt = root
        .join(".darkrun")
        .join("worktrees")
        .join(slug)
        .join(format!("_merge-{}", sanitize(target)));
    MergeSite::Temp { path: merge_wt }
}

/// Merge `source` into `target` (mechanics #4 + #6), guarded by the
/// engine-protected merge.
///
/// #4: short-circuit when there is no merge debt (identical trees OR `source`
/// already an ancestor of `target`) so a `--no-ff` no-op can never mint an
/// empty commit that triggers the alternating-sync loop.
///
/// #6: pick the merge site via [`merge_site`] — in-place when the engine is
/// already on `target`, else a detached temp worktree so the agent's tree is
/// never disturbed; the temp worktree's merge result fast-updates `target`.
fn merge_into_branch(
    _store: &StateStore,
    git: &Git,
    root: &Path,
    target: &str,
    source: &str,
    slug: &str,
    message: &str,
) -> LifecycleOutcome {
    // #4: no merge debt → clean no-op (identical trees OR already an ancestor).
    if has_no_merge_debt(git, source, target) {
        return LifecycleOutcome::noop(format!("{source} already in {target} (no merge debt)"));
    }

    match merge_site(git, root, target, slug) {
        MergeSite::Refused { note } => LifecycleOutcome::noop(note),
        MergeSite::InPlace { path } => {
            // Merge directly in the tree that holds `target` (primary or a
            // foreign clean checkout). No ref fast-update needed — the merge
            // commit advances `target` in place.
            let result = engine_protected_merge(git, &path, source, slug, message);
            let outcome = merge_result_outcome(result, source, target);
            if outcome.performed {
                // Push every landed target — unit→station, station→run-main,
                // run→base — so origin tracks the hierarchy as it advances.
                // Best-effort with NFF recovery; a failure never undoes a land.
                let _ = crate::hosting::push_head_with_nff_recovery(git, &path, target);
            }
            outcome
        }
        MergeSite::Temp { path: merge_wt } => {
            if let Some(parent) = merge_wt.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let wt_name = worktree_name_of(&merge_wt);
            // Detached worktree at the target's commit — works even when the
            // target branch is checked out elsewhere.
            if git
                .create_worktree_detached(&wt_name, &merge_wt, target)
                .is_err()
            {
                return LifecycleOutcome::noop(format!(
                    "could not create merge worktree for {target}"
                ));
            }

            let result = engine_protected_merge(git, &merge_wt, source, slug, message);
            // #3: a genuine conflict must be left IN-TREE for resolution — do
            // NOT tear down the temp worktree, or the agent has nothing to
            // resolve. The mid-merge guard suspension lets the agent write the
            // conflicted files there; the next land re-uses this same worktree.
            let conflicted = matches!(&result, Ok(o) if !o.ok && !o.conflict_paths.is_empty());
            let outcome = match &result {
                Ok(o) if o.ok && o.performed => {
                    // Advance the target branch ref to the merge commit.
                    match git.head_oid(&merge_wt) {
                        Ok(head) => {
                            let _ = git.set_branch_to(target, &head);
                            // Push the landed target (best-effort, NFF-recovering)
                            // so origin tracks the hierarchy as it advances.
                            let _ = crate::hosting::push_head_with_nff_recovery(
                                git, &merge_wt, target,
                            );
                            LifecycleOutcome::done(format!("merged {source} -> {target}"))
                        }
                        Err(e) => LifecycleOutcome::noop(format!(
                            "merged {source} -> {target} but could not resolve HEAD: {e}"
                        )),
                    }
                }
                _ => merge_result_outcome(result, source, target),
            };

            if !conflicted {
                // Tear down the temp worktree only on a clean / no-op outcome.
                let _ = git.remove_worktree(&wt_name, true);
                let _ = std::fs::remove_dir_all(&merge_wt);
            }
            outcome
        }
    }
}

/// Fold a raw [`engine_protected_merge`] result into a [`LifecycleOutcome`],
/// carrying conflict paths + the conflict branch (`target`) when the merge left
/// genuine agent-content conflicts in-tree.
fn merge_result_outcome(
    result: darkrun_git::Result<darkrun_git::MergeOutcome>,
    source: &str,
    target: &str,
) -> LifecycleOutcome {
    match result {
        Ok(o) if o.ok && o.performed => LifecycleOutcome::done(format!("merged {source} -> {target}")),
        Ok(o) if o.ok => {
            LifecycleOutcome::noop(format!("{source} already up to date with {target}"))
        }
        Ok(o) => LifecycleOutcome {
            performed: false,
            note: o
                .message
                .or_else(|| Some(format!("merge {source} -> {target} left conflicts"))),
            conflict_paths: o.conflict_paths,
            conflict_branch: Some(target.to_string()),
            conflict_step: None,
        },
        Err(e) => LifecycleOutcome::noop(format!("merge {source} -> {target} failed: {e}")),
    }
}

/// Run `fn` with a checkout of `branch` available (mechanic #6): reuse an
/// existing clean worktree on `branch` if one is registered, refuse a dirty one
/// with a named remediation, else create a detached temp worktree at `branch`'s
/// commit. The closure receives the worktree path. Cleans up a temp worktree.
///
/// Returned to mirror the reference `withWorktreeOnBranch`; the land/sync paths
/// route their merges through [`merge_into_branch`] which inlines the same
/// in-place-vs-temp choice via [`merge_site`]. This is the standalone helper for
/// callers that need the path directly.
pub fn with_worktree_on_branch<T>(
    git: &Git,
    root: &Path,
    branch: &str,
    slug: &str,
    f: impl FnOnce(&Path) -> T,
) -> std::result::Result<T, String> {
    // Reuse a registered checkout of `branch` when it's clean.
    let existing = git
        .list_worktrees()
        .ok()
        .and_then(|ws| ws.into_iter().find(|w| w.branch.as_deref() == Some(branch)));
    if let Some(wt) = existing {
        // Inspect that specific worktree's cleanliness in its own dir.
        if worktree_is_dirty(&wt.path) {
            return Err(format!(
                "branch '{branch}' is checked out at '{}' with uncommitted or untracked \
                 changes — commit or stash them (and add or clean untracked files) so the \
                 engine can merge into it",
                wt.path.display()
            ));
        }
        return Ok(f(&wt.path));
    }

    // No existing checkout → detached temp worktree at the branch commit.
    let temp = root
        .join(".darkrun")
        .join("worktrees")
        .join(slug)
        .join(format!("_wt-{}", sanitize(branch)));
    if let Some(parent) = temp.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let wt_name = worktree_name_of(&temp);
    if git.create_worktree_detached(&wt_name, &temp, branch).is_err() {
        return Err(format!("could not create a worktree on '{branch}'"));
    }
    let out = f(&temp);
    let _ = git.remove_worktree(&wt_name, true);
    let _ = std::fs::remove_dir_all(&temp);
    Ok(out)
}

/// Downstream sync before merging up (mechanic #5): keep the station fresh by
/// merging DOWN first, in two debt-gated, engine-protected steps:
///
/// 1. base/mainline -> run-main, then
/// 2. run-main -> the active station branch.
///
/// Run each tick before a land so land-time conflicts shrink. Each step is
/// gated on [`has_no_merge_debt`] (#4) and goes through the engine-protected
/// merge (#1). On a conflict the outcome carries `conflict_step` (which step)
/// and `conflict_branch` (where the merge is left in-tree for resolution).
/// No-op outside a git repo or when there's nothing to sync.
pub fn sync_branch_downstream(store: &StateStore, slug: &str, station: &str) -> LifecycleOutcome {
    let Some((git, root)) = open_git(store) else {
        return LifecycleOutcome::noop("not a git repo");
    };
    let run_main = run_main_branch(slug);
    let branch = station_branch(slug, station);
    let base = store
        .read_state(slug)
        .ok()
        .flatten()
        .and_then(|s| s.base_branch)
        .unwrap_or_else(|| resolve_base_branch(store));

    let mut performed = false;

    // Pretick remote reconciliation (#5): when an `origin` remote is configured,
    // FETCH it so teammate work pushed to the default branch can be folded into
    // the run before any land — without this, run-main silently falls behind
    // origin and accumulates non-fast-forward divergence. Best-effort: offline /
    // no-remote simply skips, and the origin merge below is debt-gated.
    let has_origin = git.remote_url("origin").ok().flatten().is_some();
    if has_origin {
        let _ = git.fetch(&root, &base);
        // Also refresh THIS run's branches — the cross-machine half. Without
        // these, a run advanced from another machine never reaches this clone
        // and every push rejects non-fast-forward forever.
        let _ = git.fetch(&root, &run_main);
        let _ = git.fetch(&root, &branch);
    }

    // Step 0: origin/run-main -> run-main, by PURE FAST-FORWARD only. When
    // another machine advanced the run, its `.darkrun` state on origin is the
    // NEWER truth — an engine-protected merge here would hold the local (stale)
    // state authoritative and clobber the remote's progress back. A genuine
    // divergence is left for the push path's NFF recovery instead.
    if has_origin && git.branch_exists(&run_main).unwrap_or(false) {
        let origin_rm = format!("origin/{run_main}");
        let strictly_behind = git.is_ancestor(&run_main, &origin_rm).unwrap_or(false)
            && !git.is_ancestor(&origin_rm, &run_main).unwrap_or(true);
        if strictly_behind {
            let on_run_main =
                git.current_branch().ok().flatten().as_deref() == Some(run_main.as_str());
            // Never move the ref under a dirty primary checkout of it.
            if !on_run_main || git.is_clean().unwrap_or(false) {
                let _ = git.set_branch_to(&run_main, &origin_rm);
                if on_run_main {
                    // Refresh the checked-out tree + index to the new tip.
                    let _ = git.checkout_branch(&run_main);
                }
                performed = true;
            }
        }
    }

    // Step 1: base -> run-main (only when both exist and there's debt).
    if git.branch_exists(&run_main).unwrap_or(false)
        && git.branch_exists(&base).unwrap_or(false)
        && !has_no_merge_debt(&git, &base, &run_main)
    {
        let out = merge_into_branch(
            store,
            &git,
            &root,
            &run_main,
            &base,
            slug,
            &format!("darkrun: sync {base} -> {run_main} (pre-land)"),
        );
        if out.has_conflicts() {
            return LifecycleOutcome {
                performed,
                note: out.note,
                conflict_paths: out.conflict_paths,
                conflict_branch: Some(run_main.clone()),
                conflict_step: Some(SyncConflictStep::MainlineToRunMain),
            };
        }
        performed |= out.performed;
    }

    // Step 1b: origin/<base> -> run-main — incorporate work other clones pushed
    // to the default branch (the predecessor's `run-main <- origin/<default>`
    // reconciliation). Debt-gated; the merge resolves `origin/<base>` from the
    // tracking ref the fetch above advanced, and no-ops when it's absent.
    if has_origin && git.branch_exists(&run_main).unwrap_or(false) {
        let origin_base = format!("origin/{base}");
        if !has_no_merge_debt(&git, &origin_base, &run_main) {
            let out = merge_into_branch(
                store,
                &git,
                &root,
                &run_main,
                &origin_base,
                slug,
                &format!("darkrun: sync {origin_base} -> {run_main} (pre-land)"),
            );
            if out.has_conflicts() {
                return LifecycleOutcome {
                    performed,
                    note: out.note,
                    conflict_paths: out.conflict_paths,
                    conflict_branch: Some(run_main.clone()),
                    conflict_step: Some(SyncConflictStep::MainlineToRunMain),
                };
            }
            performed |= out.performed;
        }
    }

    // Step 2: run-main -> station branch (only when both exist and there's debt).
    if git.branch_exists(&run_main).unwrap_or(false)
        && git.branch_exists(&branch).unwrap_or(false)
        && !has_no_merge_debt(&git, &run_main, &branch)
    {
        let out = merge_into_branch(
            store,
            &git,
            &root,
            &branch,
            &run_main,
            slug,
            &format!("darkrun: sync {run_main} -> {branch} (pre-land)"),
        );
        if out.has_conflicts() {
            return LifecycleOutcome {
                performed,
                note: out.note,
                conflict_paths: out.conflict_paths,
                conflict_branch: Some(branch.clone()),
                conflict_step: Some(SyncConflictStep::RunMainToStation),
            };
        }
        performed |= out.performed;
    }

    if performed {
        LifecycleOutcome::done(format!("synced {base} -> {run_main} -> {branch}"))
    } else {
        LifecycleOutcome::noop("nothing to sync (branches fresh)")
    }
}

/// Whether the worktree checked out at `path` has uncommitted or untracked
/// changes. Opens the worktree in-process (pure-Rust gix); any error is treated
/// as dirty so the engine never merges into a tree it couldn't inspect.
fn worktree_is_dirty(path: &Path) -> bool {
    Git::open(path)
        .and_then(|g| g.is_clean())
        .map(|clean| !clean)
        .unwrap_or(true)
}

/// The worktree admin name for a path — its final component, matching how
/// `git worktree add <path>` derives the name. Used to pair create + remove.
fn worktree_name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Make a branch name filesystem-safe for a worktree directory component.
fn sanitize(name: &str) -> String {
    name.replace('/', "-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    fn init_repo() -> (TempDir, PathBuf, StateStore) {
        let dir = TempDir::new().expect("tmp");
        let root = dir.path().to_path_buf();
        let git = |args: &[&str]| {
            let ok = Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .status()
                .expect("git")
                .success();
            assert!(ok, "git {args:?} failed");
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["config", "user.email", "test@darkrun.ai"]);
        git(&["config", "user.name", "darkrun test"]);
        // Gitignore .darkrun so the nested worktrees don't pollute commits.
        std::fs::write(root.join(".gitignore"), ".darkrun/\n").unwrap();
        std::fs::write(root.join("README.md"), "# smoke\n").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "init"]);
        let store = StateStore::new(&root);
        (dir, root, store)
    }

    fn branch_exists(root: &Path, branch: &str) -> bool {
        Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["rev-parse", "--verify", &format!("refs/heads/{branch}")])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    #[test]
    fn branch_name_helpers() {
        assert_eq!(run_main_branch("r"), "darkrun/r/main");
        assert_eq!(station_branch("r", "build"), "darkrun/r/build");
        // A unit forks off its station, in a parallel `units/` namespace that
        // dodges the git directory/file ref conflict under the station ref.
        assert_eq!(unit_branch("r", "build", "u1"), "darkrun/r/units/build/u1");
        // A fix forks off its station too, in the parallel `fixes/` namespace.
        assert_eq!(fix_branch("r", "build", "fb-7"), "darkrun/r/fixes/build/fb-7");
    }

    #[test]
    fn resolve_base_reads_settings_default() {
        let (_d, root, store) = init_repo();
        std::fs::create_dir_all(store.root()).unwrap();
        std::fs::write(store.root().join("settings.yml"), "default_branch: trunk\n").unwrap();
        assert_eq!(resolve_base_branch(&store), "trunk");
        // Absent → main.
        std::fs::remove_file(store.root().join("settings.yml")).unwrap();
        assert_eq!(resolve_base_branch(&store), "main");
        let _ = root;
    }

    #[test]
    fn lifecycle_is_clean_noop_outside_git() {
        let dir = TempDir::new().unwrap();
        let store = StateStore::new(dir.path());
        assert!(!ensure_run_main(&store, "r").performed);
        assert!(!enter_station(&store, "r", "build").performed);
        assert!(!land_station(&store, "r", "build").performed);
        assert!(!enter_unit(&store, "r", "build", "u1").performed);
        assert!(!land_unit(&store, "r", "build", "u1").performed);
        assert!(!enter_fix(&store, "r", "build", "fb-1").performed);
        assert!(!land_fix(&store, "r", "build", "fb-1").performed);
        assert!(!land_run(&store, "r").performed);
    }

    #[test]
    fn ensure_run_main_forks_off_base_idempotently() {
        let (_d, root, store) = init_repo();
        let out = ensure_run_main(&store, "r");
        assert!(out.performed, "first fork should perform: {out:?}");
        assert!(branch_exists(&root, "darkrun/r/main"));
        // Second call is a clean no-op.
        assert!(!ensure_run_main(&store, "r").performed);
    }

    #[test]
    fn enter_station_creates_branch_and_worktree_then_land_removes_them() {
        let (_d, root, store) = init_repo();
        ensure_run_main(&store, "r");

        let enter = enter_station(&store, "r", "build");
        assert!(enter.performed, "enter should perform: {enter:?}");
        assert!(branch_exists(&root, "darkrun/r/build"));
        let wt = station_worktree_path(&root, "r", "build");
        assert!(wt.exists(), "station worktree should exist on disk");

        // Do some station work on the worktree branch.
        std::fs::write(wt.join("feature.txt"), "built\n").unwrap();
        let git = |args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(&wt)
                .args(args)
                .status()
                .expect("git")
                .success()
        };
        assert!(git(&["add", "-A"]));
        assert!(git(&["commit", "-q", "-m", "build work"]));

        // Land it onto run-main.
        let land = land_station(&store, "r", "build");
        assert!(land.performed, "land should perform: {land:?}");

        // Station branch + worktree are gone…
        assert!(!branch_exists(&root, "darkrun/r/build"));
        assert!(!wt.exists(), "station worktree should be removed");

        // …and run-main now carries the station's work.
        let out = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["show", "darkrun/r/main:feature.txt"])
            .output()
            .unwrap();
        assert!(out.status.success(), "feature.txt should be on run-main");
        assert_eq!(String::from_utf8_lossy(&out.stdout), "built\n");
    }

    /// Commit `file=content` on the worktree at `wt`.
    fn commit_in(wt: &Path, file: &str, content: &str, msg: &str) {
        std::fs::write(wt.join(file), content).unwrap();
        for args in [
            vec!["add", "-A"],
            vec!["commit", "-q", "-m", msg],
        ] {
            assert!(Command::new("git")
                .arg("-C")
                .arg(wt)
                .args(&args)
                .status()
                .unwrap()
                .success());
        }
    }

    #[test]
    fn enter_unit_forks_off_station_then_land_merges_back_to_station() {
        let (_d, root, store) = init_repo();
        ensure_run_main(&store, "r");
        enter_station(&store, "r", "build");

        // The unit forks off the station branch onto its own worktree.
        let enter = enter_unit(&store, "r", "build", "u1");
        assert!(enter.performed, "enter_unit should perform: {enter:?}");
        assert_eq!(enter.note.as_deref(), Some("darkrun/r/units/build/u1"));
        assert!(branch_exists(&root, "darkrun/r/units/build/u1"));
        let wt = unit_worktree_path(&root, "r", "build", "u1");
        assert!(wt.exists(), "unit worktree should exist on disk");

        // Re-entering is idempotent (crash recovery).
        assert!(enter_unit(&store, "r", "build", "u1").performed);

        // Do the unit's work, then land it back onto the STATION branch (not
        // run-main — the unit lands one level down).
        commit_in(&wt, "u1.txt", "unit one\n", "u1 work");
        let land = land_unit(&store, "r", "build", "u1");
        assert!(land.performed, "land_unit should perform: {land:?}");

        // Unit branch + worktree gone; the work is on the station branch, NOT yet
        // on run-main (the station hasn't landed).
        assert!(!branch_exists(&root, "darkrun/r/units/build/u1"));
        assert!(!wt.exists(), "unit worktree should be removed");
        let on_station = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["show", "darkrun/r/build:u1.txt"])
            .output()
            .unwrap();
        assert!(on_station.status.success(), "u1.txt should be on the station branch");
        let on_main = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["show", "darkrun/r/main:u1.txt"])
            .output()
            .unwrap();
        assert!(!on_main.status.success(), "u1.txt must NOT be on run-main yet");
    }

    #[test]
    fn enter_unit_noops_when_station_branch_absent() {
        let (_d, _root, store) = init_repo();
        ensure_run_main(&store, "r");
        // No enter_station → no parent branch to fork from.
        let out = enter_unit(&store, "r", "build", "u1");
        assert!(!out.performed, "a unit cannot fork off a station that wasn't entered");
        assert!(out.note.unwrap().contains("cannot enter unit"));
    }

    #[test]
    fn two_units_isolate_then_both_land_onto_the_station() {
        let (_d, root, store) = init_repo();
        ensure_run_main(&store, "r");
        enter_station(&store, "r", "build");

        enter_unit(&store, "r", "build", "u1");
        enter_unit(&store, "r", "build", "u2");
        let wt1 = unit_worktree_path(&root, "r", "build", "u1");
        let wt2 = unit_worktree_path(&root, "r", "build", "u2");
        commit_in(&wt1, "a.txt", "from u1\n", "u1");
        commit_in(&wt2, "b.txt", "from u2\n", "u2");

        assert!(land_unit(&store, "r", "build", "u1").performed);
        assert!(land_unit(&store, "r", "build", "u2").performed);

        // Both units' work is on the station branch.
        for f in ["a.txt", "b.txt"] {
            let out = Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(["show", &format!("darkrun/r/build:{f}")])
                .output()
                .unwrap();
            assert!(out.status.success(), "{f} should be on the station branch");
        }
    }

    #[test]
    fn enter_fix_forks_off_station_then_land_merges_back() {
        let (_d, root, store) = init_repo();
        ensure_run_main(&store, "r");
        enter_station(&store, "r", "build");

        let enter = enter_fix(&store, "r", "build", "fb-1");
        assert!(enter.performed, "enter_fix should perform: {enter:?}");
        assert_eq!(enter.note.as_deref(), Some("darkrun/r/fixes/build/fb-1"));
        assert!(branch_exists(&root, "darkrun/r/fixes/build/fb-1"));
        let wt = fix_worktree_path(&root, "r", "build", "fb-1");
        assert!(wt.exists(), "fix worktree should exist on disk");

        commit_in(&wt, "fix.txt", "repaired\n", "fix fb-1");
        let land = land_fix(&store, "r", "build", "fb-1");
        assert!(land.performed, "land_fix should perform: {land:?}");

        assert!(!branch_exists(&root, "darkrun/r/fixes/build/fb-1"));
        assert!(!wt.exists(), "fix worktree should be removed");
        let on_station = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["show", "darkrun/r/build:fix.txt"])
            .output()
            .unwrap();
        assert!(on_station.status.success(), "the fix should land on the station branch");
    }

    /// Gap #2: a RUN-SCOPE fix (`_run`) forks off run-main and lands back onto
    /// run-main — not a station branch.
    #[test]
    fn run_scope_fix_forks_and_lands_on_run_main() {
        let (_d, root, store) = init_repo();
        ensure_run_main(&store, "r");

        let enter = enter_fix(&store, "r", "_run", "fb-9");
        assert!(enter.performed, "enter_fix run-scope should perform: {enter:?}");
        assert_eq!(enter.note.as_deref(), Some("darkrun/r/fixes/_run/fb-9"));
        let wt = fix_worktree_path(&root, "r", "_run", "fb-9");
        assert!(wt.exists(), "run-scope fix worktree exists");

        commit_in(&wt, "runfix.txt", "run repair\n", "fix fb-9");
        let land = land_fix(&store, "r", "_run", "fb-9");
        assert!(land.performed, "land_fix run-scope should perform: {land:?}");

        // The fix landed on RUN-MAIN, not any station branch.
        let on_run_main = Command::new("git")
            .arg("-C").arg(&root)
            .args(["show", "darkrun/r/main:runfix.txt"])
            .output().unwrap();
        assert!(on_run_main.status.success(), "the run-scope fix lands on run-main");
    }

    #[test]
    fn land_run_merges_run_main_into_base() {
        let (_d, root, store) = init_repo();
        ensure_run_main(&store, "r");
        enter_station(&store, "r", "build");
        let wt = station_worktree_path(&root, "r", "build");
        std::fs::write(wt.join("shipped.txt"), "ship\n").unwrap();
        let git_wt = |args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(&wt)
                .args(args)
                .status()
                .unwrap()
                .success()
        };
        assert!(git_wt(&["add", "-A"]));
        assert!(git_wt(&["commit", "-q", "-m", "ship work"]));
        land_station(&store, "r", "build");

        let land = land_run(&store, "r");
        assert!(land.performed, "land_run should merge run-main -> base: {land:?}");
        let out = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["show", "main:shipped.txt"])
            .output()
            .unwrap();
        assert!(out.status.success(), "shipped.txt should be on base (main)");
    }

    /// G4b: run_main_status tracks where run-main stands vs the default branch
    /// across its lifecycle — not_forked → up_to_date → ahead → merged.
    #[test]
    fn run_main_status_tracks_run_main_vs_default() {
        let (_d, root, store) = init_repo();
        // Not forked yet.
        assert_eq!(run_main_status(&store, "r"), RunMainStatus::NotForked);

        // Forked at the base tip → up to date (same commit).
        ensure_run_main(&store, "r");
        assert_eq!(run_main_status(&store, "r"), RunMainStatus::UpToDate);

        // Do verified station work and land it onto run-main → run-main is ahead
        // of the default branch.
        enter_station(&store, "r", "build");
        let wt = station_worktree_path(&root, "r", "build");
        commit_in(&wt, "feature.txt", "built\n", "build work");
        land_station(&store, "r", "build");
        assert_eq!(run_main_status(&store, "r"), RunMainStatus::Ahead);

        // Land run-main onto the default branch → merged (base now contains it
        // and has advanced past the original fork point).
        land_run(&store, "r");
        assert_eq!(run_main_status(&store, "r"), RunMainStatus::Merged);
    }

    /// #4: a land with no merge debt (identical trees) is a clean no-op — no new
    /// commit minted on run-main, so the alternating no-op-merge loop can't start.
    #[test]
    fn land_with_no_merge_debt_is_noop() {
        let (_d, root, store) = init_repo();
        ensure_run_main(&store, "r");
        // Enter a station but do NO work: the station branch's tree is identical
        // to run-main. A land must short-circuit (no debt) rather than commit.
        enter_station(&store, "r", "build");
        let before = git_rev(&root, "darkrun/r/main");
        let land = land_station(&store, "r", "build");
        assert!(!land.performed, "no-debt land must not perform: {land:?}");
        let after = git_rev(&root, "darkrun/r/main");
        assert_eq!(before, after, "run-main HEAD must be unchanged (no empty commit)");
    }

    /// land_run no-ops when run-main carries no debt against base (identical
    /// trees) — the merge_into_branch debt short-circuit, not the land_child one.
    #[test]
    fn land_run_with_no_debt_is_a_clean_noop() {
        let (_d, _root, store) = init_repo();
        ensure_run_main(&store, "r");
        // run-main was just forked off main and has no new commits → no debt.
        let out = land_run(&store, "r");
        assert!(!out.performed, "a no-debt run land must not mint a commit: {out:?}");
        assert!(out.note.unwrap_or_default().contains("no merge debt"));
    }

    /// land_run refuses when the recorded base branch doesn't exist.
    #[test]
    fn land_run_noops_when_the_base_branch_is_missing() {
        let (_d, _root, store) = init_repo();
        ensure_run_main(&store, "r");
        // Point the run at a base branch that doesn't exist.
        let mut state = store.read_state("r").ok().flatten().unwrap_or_default();
        state.base_branch = Some("ghost-base".into());
        store.write_state("r", &state).unwrap();
        let out = land_run(&store, "r");
        assert!(!out.performed);
        assert!(out.note.unwrap_or_default().contains("cannot land run"));
    }

    /// #8: the station worktree is gone but its branch still carries unmerged
    /// commits — land_station must merge the durable branch, not report done.
    #[test]
    fn land_station_merges_when_worktree_gone_but_branch_unmerged() {
        let (_d, root, store) = init_repo();
        ensure_run_main(&store, "r");
        enter_station(&store, "r", "build");
        let wt = station_worktree_path(&root, "r", "build");
        std::fs::write(wt.join("late.txt"), "shipped\n").unwrap();
        let git_wt = |args: &[&str]| {
            Command::new("git").arg("-C").arg(&wt).args(args).status().unwrap().success()
        };
        assert!(git_wt(&["add", "-A"]));
        assert!(git_wt(&["commit", "-q", "-m", "late station work"]));

        // Remove ONLY the worktree (simulate a crash) — the branch + its
        // unmerged commit survive.
        Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["worktree", "remove", "--force", wt.to_str().unwrap()])
            .status()
            .unwrap();
        assert!(!wt.exists());
        assert!(branch_exists(&root, "darkrun/r/build"), "branch survives the crash");

        let land = land_station(&store, "r", "build");
        assert!(land.performed, "must still merge the durable branch: {land:?}");
        let out = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["show", "darkrun/r/main:late.txt"])
            .output()
            .unwrap();
        assert!(out.status.success(), "late.txt must reach run-main");
        assert_eq!(String::from_utf8_lossy(&out.stdout), "shipped\n");
    }

    /// #6: when the engine is NOT on the target, the merge runs in a detached
    /// temp worktree and the primary checkout is never disturbed.
    #[test]
    fn merge_isolation_leaves_primary_tree_untouched() {
        let (_d, root, store) = init_repo();
        // Primary tree stays on `main` the whole time.
        ensure_run_main(&store, "r");
        enter_station(&store, "r", "build");
        let wt = station_worktree_path(&root, "r", "build");
        std::fs::write(wt.join("iso.txt"), "iso\n").unwrap();
        let git_wt = |args: &[&str]| {
            Command::new("git").arg("-C").arg(&wt).args(args).status().unwrap().success()
        };
        assert!(git_wt(&["add", "-A"]));
        assert!(git_wt(&["commit", "-q", "-m", "iso work"]));

        // Land build -> run-main. Primary is on `main` (not run-main), so the
        // merge happens in a temp worktree.
        let land = land_station(&store, "r", "build");
        assert!(land.performed, "{land:?}");
        // Primary tree is still on main and clean — never touched.
        let head = git_rev_branch(&root);
        assert_eq!(head.as_deref(), Some("main"), "primary checkout untouched");
        let clean = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["status", "--porcelain"])
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&clean.stdout).trim().is_empty(), "primary clean");
        // No `iso.txt` in the primary working tree.
        assert!(!root.join("iso.txt").exists(), "merge did not leak into primary tree");
    }

    /// #5: sync_branch_downstream merges base -> run-main -> station, fresh.
    #[test]
    fn sync_downstream_freshens_station_before_land() {
        let (_d, root, store) = init_repo();
        ensure_run_main(&store, "r");
        enter_station(&store, "r", "build");

        // Advance base (main) with a commit the station hasn't seen.
        std::fs::write(root.join("base-new.txt"), "from base\n").unwrap();
        let git_root = |args: &[&str]| {
            Command::new("git").arg("-C").arg(&root).args(args).status().unwrap().success()
        };
        assert!(git_root(&["add", "-A"]));
        assert!(git_root(&["commit", "-q", "-m", "base advanced"]));

        let sync = sync_branch_downstream(&store, "r", "build");
        assert!(sync.performed, "should merge base down to the station: {sync:?}");
        assert!(sync.conflict_step.is_none(), "clean sync has no conflict step");

        // The station branch now carries the base's new file.
        let out = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["show", "darkrun/r/build:base-new.txt"])
            .output()
            .unwrap();
        assert!(out.status.success(), "base-new.txt must reach the station branch");

        // A second sync is a clean no-op (branches now fresh).
        let again = sync_branch_downstream(&store, "r", "build");
        assert!(!again.performed, "second sync is a no-op: {again:?}");
    }

    fn git_rev(root: &Path, refname: &str) -> String {
        String::from_utf8_lossy(
            &Command::new("git")
                .arg("-C")
                .arg(root)
                .args(["rev-parse", refname])
                .output()
                .unwrap()
                .stdout,
        )
        .trim()
        .to_string()
    }

    fn git_rev_branch(root: &Path) -> Option<String> {
        let out = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .unwrap();
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if s.is_empty() || s == "HEAD" { None } else { Some(s) }
    }

    /// Run `git -C <dir> <args>`, asserting success.
    fn git_in(dir: &Path, args: &[&str]) {
        let ok = Command::new("git").arg("-C").arg(dir).args(args).status().unwrap().success();
        assert!(ok, "git {args:?} in {dir:?}");
    }

    /// Commit a divergent edit to `file` on `branch` via a throwaway worktree
    /// (so a branch checked out nowhere can still gain a commit).
    fn commit_on_branch(root: &Path, branch: &str, file: &str, body: &str, msg: &str) {
        let wt = root.parent().unwrap().join(format!("dr-edit-{}", msg.replace(' ', "-")));
        git_in(root, &["worktree", "add", "--force", &wt.to_string_lossy(), branch]);
        std::fs::write(wt.join(file), body).unwrap();
        git_in(&wt, &["add", "-A"]);
        git_in(&wt, &["commit", "-qm", msg]);
        git_in(root, &["worktree", "remove", "--force", &wt.to_string_lossy()]);
    }

    #[test]
    fn land_station_with_a_genuine_conflict_leaves_it_in_tree() {
        let (_d, root, store) = init_repo();
        // A shared file in the base both sides will diverge on.
        std::fs::write(root.join("shared.txt"), "base\n").unwrap();
        git_in(&root, &["add", "-A"]);
        git_in(&root, &["commit", "-qm", "add shared"]);

        ensure_run_main(&store, "r");
        enter_station(&store, "r", "build");
        let wt = station_worktree_path(&root, "r", "build");

        // Build edits shared.txt one way…
        std::fs::write(wt.join("shared.txt"), "build version\n").unwrap();
        git_in(&wt, &["add", "-A"]);
        git_in(&wt, &["commit", "-qm", "build edit"]);
        // …and run-main edits it the other way.
        commit_on_branch(&root, "darkrun/r/main", "shared.txt", "main version\n", "main edit");

        // Landing build -> run-main surfaces a genuine agent-content conflict,
        // left IN-TREE (not torn down) for resolution.
        let land = land_station(&store, "r", "build");
        assert!(
            land.has_conflicts() || land.conflict_branch.is_some(),
            "expected a conflict outcome, got {land:?}"
        );
    }

    #[test]
    fn sync_branch_downstream_freshens_run_main_and_station_from_base() {
        let (_d, root, store) = init_repo();
        ensure_run_main(&store, "r");
        enter_station(&store, "r", "build");

        // The base branch (`main`) advances with a new, non-conflicting file
        // after the run forked — sync pulls it down base -> run-main -> station.
        commit_on_branch(&root, "main", "base-new.txt", "fresh\n", "base advance");

        let out = sync_branch_downstream(&store, "r", "build");
        // Either it performed a downstream merge or cleanly no-op'd — never errors;
        // the new base file is now reachable from the station branch.
        assert!(out.conflict_step.is_none(), "non-conflicting sync: {out:?}");
        let reachable = Command::new("git")
            .arg("-C").arg(&root)
            .args(["cat-file", "-e", "darkrun/r/build:base-new.txt"])
            .status().unwrap().success();
        assert!(reachable, "base advance should reach the station branch after sync");
    }

    /// Gap #5: the pre-tick sync FETCHES origin and folds work another clone
    /// pushed to the default branch into run-main — without it, run-main silently
    /// falls behind origin.
    #[test]
    fn sync_downstream_pulls_origin_into_run_main() {
        let (_d, root, store) = init_repo();
        let git = |args: &[&str]| {
            assert!(Command::new("git").arg("-C").arg(&root).args(args).status().unwrap().success(), "git {args:?}");
        };
        // A bare origin seeded from this repo's `main`.
        let bare = TempDir::new().unwrap();
        assert!(Command::new("git").args(["init", "-q", "--bare", &bare.path().to_string_lossy()]).status().unwrap().success());
        git(&["remote", "add", "origin", &bare.path().to_string_lossy()]);
        git(&["push", "-q", "origin", "main"]);

        // The run forks run-main + a station off the current base.
        ensure_run_main(&store, "r");
        enter_station(&store, "r", "build");

        // Another clone pushes a new commit to origin/main (teammate work).
        let other = TempDir::new().unwrap();
        let o = other.path().join("o");
        assert!(Command::new("git").args(["clone", "-q", &bare.path().to_string_lossy(), &o.to_string_lossy()]).status().unwrap().success());
        let og = |args: &[&str]| { assert!(Command::new("git").arg("-C").arg(&o).args(args).status().unwrap().success(), "git {args:?}"); };
        og(&["config", "user.email", "o@x.io"]); og(&["config", "user.name", "O"]);
        std::fs::write(o.join("teammate.txt"), "pushed\n").unwrap();
        og(&["add", "-A"]); og(&["commit", "-qm", "teammate work"]); og(&["push", "-q", "origin", "main"]);

        // Sync fetches origin and merges origin/main into run-main → the teammate
        // file becomes reachable from run-main (and the station) even though the
        // LOCAL `main` was never advanced.
        let out = sync_branch_downstream(&store, "r", "build");
        assert!(out.conflict_step.is_none(), "non-conflicting origin sync: {out:?}");
        let reachable = Command::new("git")
            .arg("-C").arg(&root)
            .args(["cat-file", "-e", "darkrun/r/main:teammate.txt"])
            .status().unwrap().success();
        assert!(reachable, "origin/main's teammate commit should reach run-main after sync");
    }

    #[test]
    fn with_worktree_on_branch_runs_a_closure_against_the_branch_tree() {
        let (_d, _root, store) = init_repo();
        ensure_run_main(&store, "r");
        let (git, root2) = open_git(&store).unwrap();
        let saw = with_worktree_on_branch(&git, &root2, &run_main_branch("r"), "r", |wt| {
            // README from the base commit is present in run-main's tree.
            wt.join("README.md").exists()
        })
        .expect("closure runs on a fresh worktree");
        assert!(saw, "the run-main worktree carries the base README");
    }

    #[test]
    fn sync_conflict_step_labels_are_stable() {
        assert_eq!(SyncConflictStep::MainlineToRunMain.as_str(), "mainline_to_run_main");
        assert_eq!(SyncConflictStep::RunMainToStation.as_str(), "run_main_to_station");
    }

    #[test]
    fn run_main_status_reports_diverged_and_unforked_base() {
        let (_d, root, store) = init_repo();
        let git = |args: &[&str]| {
            assert!(Command::new("git").arg("-C").arg(&root).args(args).status().unwrap().success(), "git {args:?}");
        };
        ensure_run_main(&store, "r");
        // Advance BOTH branches independently → neither is an ancestor → Diverged.
        std::fs::write(root.join("base.txt"), "on main\n").unwrap();
        git(&["add", "-A"]); git(&["commit", "-q", "-m", "main advance"]);
        git(&["checkout", "-q", "darkrun/r/main"]);
        std::fs::write(root.join("rm.txt"), "on run-main\n").unwrap();
        git(&["add", "-A"]); git(&["commit", "-q", "-m", "run-main advance"]);
        git(&["checkout", "-q", "main"]);
        assert_eq!(run_main_status(&store, "r"), RunMainStatus::Diverged);

        // A run whose recorded base branch no longer exists reads as not-forked.
        let (_d2, _root2, store2) = init_repo();
        ensure_run_main(&store2, "r2");
        let mut state = store2.read_state("r2").unwrap().unwrap_or_default();
        state.base_branch = Some("ghost-base".into());
        store2.write_state("r2", &state).unwrap();
        assert_eq!(run_main_status(&store2, "r2"), RunMainStatus::NotForked);
    }

    #[test]
    fn sync_downstream_surfaces_a_mainline_conflict() {
        let (_d, root, store) = init_repo();
        ensure_run_main(&store, "r");
        let git = |args: &[&str]| {
            assert!(Command::new("git").arg("-C").arg(&root).args(args).status().unwrap().success(), "git {args:?}");
        };
        // Conflicting edits to README on base (main) vs run-main.
        std::fs::write(root.join("README.md"), "BASE EDIT\n").unwrap();
        git(&["add", "-A"]); git(&["commit", "-qm", "base edit"]);
        git(&["checkout", "-q", "darkrun/r/main"]);
        std::fs::write(root.join("README.md"), "RUN-MAIN EDIT\n").unwrap();
        git(&["add", "-A"]); git(&["commit", "-qm", "run-main edit"]);
        git(&["checkout", "-q", "main"]);
        // base -> run-main conflicts on README → the mainline-to-run-main step.
        let out = sync_branch_downstream(&store, "r", "build");
        assert_eq!(out.conflict_step, Some(SyncConflictStep::MainlineToRunMain));
        assert!(!out.conflict_paths.is_empty(), "the conflicting path is surfaced");
    }

    #[test]
    fn sync_downstream_surfaces_a_run_main_to_station_conflict() {
        let (_d, root, store) = init_repo();
        ensure_run_main(&store, "r");
        enter_station(&store, "r", "build");
        let git = |args: &[&str]| {
            assert!(Command::new("git").arg("-C").arg(&root).args(args).status().unwrap().success(), "git {args:?}");
        };
        // Conflicting edits on run-main vs the station branch. run-main has no
        // checkout (edit via root); the station branch lives in its worktree.
        git(&["checkout", "-q", "darkrun/r/main"]);
        std::fs::write(root.join("README.md"), "RUN-MAIN SIDE\n").unwrap();
        git(&["commit", "-aqm", "run-main edit"]);
        git(&["checkout", "-q", "main"]);
        let wt = station_worktree_path(&root, "r", "build");
        commit_in(&wt, "README.md", "STATION SIDE\n", "station edit");
        // No base→run-main debt, but run-main→station conflicts → step 2.
        let out = sync_branch_downstream(&store, "r", "build");
        assert_eq!(out.conflict_step, Some(SyncConflictStep::RunMainToStation));
        assert!(!out.conflict_paths.is_empty());
    }

    #[test]
    fn lifecycle_ops_noop_when_their_branch_is_absent() {
        let (_d, _root, store) = init_repo();
        // No run-main forked yet → land_run is a clean no-op.
        assert!(!land_run(&store, "r").performed);
        // The station branch isn't forked → a fix on it can't enter.
        assert!(!enter_fix(&store, "r", "build", "fix-1").performed);
        // land_station on a never-entered station is a no-op too.
        assert!(!land_station(&store, "r", "build").performed);
    }

    #[test]
    fn landing_refuses_a_dirty_checkout_of_the_target_branch() {
        let (_d, root, store) = init_repo();
        ensure_run_main(&store, "r");
        enter_station(&store, "r", "build");
        // Real station work so there's genuine merge debt (the no-debt
        // short-circuit doesn't fire).
        let wt = station_worktree_path(&root, "r", "build");
        commit_in(&wt, "feature.txt", "built\n", "build work");

        // Check run-main out in a separate worktree and leave it dirty.
        let rm_wt = root.join("rm-checkout");
        assert!(Command::new("git")
            .arg("-C").arg(&root)
            .args(["worktree", "add", "-q", rm_wt.to_str().unwrap(), "darkrun/r/main"])
            .status().unwrap().success());
        std::fs::write(rm_wt.join("dirty.txt"), "wip").unwrap();

        // The land finds the dirty target checkout and refuses with a remediation
        // note rather than merging into an unclean tree.
        let out = land_station(&store, "r", "build");
        assert!(!out.performed, "a dirty target checkout blocks the land");
        let note = out.note.unwrap_or_default();
        assert!(note.contains("uncommitted") || note.contains("untracked"), "remediation note: {note}");
    }

    #[test]
    fn landing_noops_when_the_parent_branch_is_missing() {
        let (_d, root, store) = init_repo();
        ensure_run_main(&store, "r");
        enter_station(&store, "r", "build");
        let wt = station_worktree_path(&root, "r", "build");
        commit_in(&wt, "feature.txt", "built\n", "build work");

        // Delete run-main so the child branch exists but its parent does not.
        assert!(Command::new("git")
            .arg("-C").arg(&root)
            .args(["branch", "-D", "darkrun/r/main"])
            .status().unwrap().success());

        let out = land_station(&store, "r", "build");
        assert!(!out.performed, "a missing parent cannot be landed into");
        assert!(out.note.unwrap_or_default().contains("cannot land"));
    }

    #[test]
    fn with_worktree_on_branch_handles_clean_dirty_and_temp() {
        let (_d, root, store) = init_repo();
        ensure_run_main(&store, "r");
        enter_station(&store, "r", "build");
        let git = darkrun_git::Git::open(&root).unwrap();
        let branch = station_branch("r", "build");

        // A clean registered worktree runs the closure on its path.
        let ran = with_worktree_on_branch(&git, &root, &branch, "r", |_| 42).unwrap();
        assert_eq!(ran, 42);

        // A dirty worktree on that branch is refused.
        let wt = station_worktree_path(&root, "r", "build");
        std::fs::write(wt.join("uncommitted.txt"), "wip").unwrap();
        assert!(with_worktree_on_branch(&git, &root, &branch, "r", |_| 1).is_err());

        // run-main has no checkout → a detached temp worktree is created + cleaned.
        let run_main = run_main_branch("r");
        let saw = with_worktree_on_branch(&git, &root, &run_main, "r", |p| p.exists()).unwrap();
        assert!(saw, "the temp worktree path exists while the closure runs");
    }
}
