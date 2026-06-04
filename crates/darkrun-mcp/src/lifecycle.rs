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
    format!("{BRANCH_PREFIX}/{slug}/fixes/{station}/{fix_id}")
}

/// The on-disk worktree path for a fix's branch:
/// `<repo>/.darkrun/worktrees/<slug>/fixes/<station>/<id>`.
pub fn fix_worktree_path(repo_root: &Path, slug: &str, station: &str, fix_id: &str) -> PathBuf {
    repo_root
        .join(".darkrun")
        .join("worktrees")
        .join(slug)
        .join("fixes")
        .join(station)
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
    fork_and_worktree(&git, &run_main, &branch, &wt_path, &format!("{slug}-{station}"))
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
    let parent = station_branch(slug, station);
    if !git.branch_exists(&parent).unwrap_or(false) {
        return LifecycleOutcome::noop(format!("{parent} unavailable; cannot enter fix"));
    }
    let branch = fix_branch(slug, station, fix_id);
    let wt_path = fix_worktree_path(&root, slug, station, fix_id);
    fork_and_worktree(
        &git,
        &parent,
        &branch,
        &wt_path,
        &format!("{slug}-{station}-fix-{fix_id}"),
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
    let parent = station_branch(slug, station);
    let wt_path = fix_worktree_path(&root, slug, station, fix_id);
    land_child(
        store,
        &git,
        &root,
        slug,
        &parent,
        &branch,
        &wt_path,
        &format!("{slug}-{station}-fix-{fix_id}"),
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
        let _ = delete_branch(root, branch);
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
        let _ = delete_branch(root, branch);
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
        let dirty = git_at(&wt.path, &["status", "--porcelain"])
            .map(|o| !o.trim().is_empty())
            .unwrap_or(true);
        if dirty {
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
            merge_result_outcome(result, source, target)
        }
        MergeSite::Temp { path: merge_wt } => {
            let merge_wt_str = merge_wt.to_string_lossy().to_string();
            if let Some(parent) = merge_wt.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            // Detached worktree at the target's commit — works even when the
            // target branch is checked out elsewhere.
            if git_at(root, &["worktree", "add", "--detach", &merge_wt_str, target]).is_err() {
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
                    match git_at(&merge_wt, &["rev-parse", "HEAD"]) {
                        Ok(head) => {
                            let _ = git_at(root, &["branch", "-f", target, head.trim()]);
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
                let _ = git_at(root, &["worktree", "remove", "--force", &merge_wt_str]);
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
        // Inspect that specific worktree's cleanliness via status in its dir.
        let dirty = git_at(&wt.path, &["status", "--porcelain"])
            .map(|o| !o.trim().is_empty())
            .unwrap_or(true);
        if dirty {
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
    let temp_str = temp.to_string_lossy().to_string();
    if let Some(parent) = temp.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if git_at(root, &["worktree", "add", "--detach", &temp_str, branch]).is_err() {
        return Err(format!("could not create a worktree on '{branch}'"));
    }
    let out = f(&temp);
    let _ = git_at(root, &["worktree", "remove", "--force", &temp_str]);
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

/// Run `git -C <dir> <args>`, returning trimmed stdout on success. Used for the
/// handful of merge-worktree plumbing commands not on the GitBackend trait.
fn git_at(dir: &Path, args: &[&str]) -> std::io::Result<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()?;
    if !output.status.success() {
        return Err(std::io::Error::other(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Delete a local branch via the shell (no GitBackend primitive for it; the
/// merge worktree on `target` already holds the merged result).
fn delete_branch(root: &Path, branch: &str) -> std::io::Result<()> {
    std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["branch", "-D", branch])
        .output()
        .map(|_| ())
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
}
