//! The branch + worktree lifecycle the manager hooks into.
//!
//! darkrun drives a **universal** branch hierarchy — the same for every mode
//! (continuous, autopilot, quick, bugfix, refactor, full, and discrete):
//!
//! ```text
//! <base>  <  darkrun/<slug>/main  <  darkrun/<slug>/<station>
//! ```
//!
//! - **run-main** (`darkrun/<slug>/main`) is the stable per-run base. It only
//!   ever accumulates *fully-verified* stations.
//! - **station branches** (`darkrun/<slug>/<station>`) fork off run-main when a
//!   station is entered, and carry that station's work on an isolated worktree.
//! - On station completion the station branch lands onto run-main through the
//!   [`engine_protected_merge`](darkrun_git::engine_protected_merge), and the
//!   station worktree + branch are removed.
//! - At run completion run-main lands onto the repo base (the run-completion
//!   PR/merge).
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
use darkrun_git::{engine_protected_merge, GitBackend, Git};

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
}

impl LifecycleOutcome {
    fn noop(note: impl Into<String>) -> Self {
        Self {
            performed: false,
            note: Some(note.into()),
            conflict_paths: Vec::new(),
        }
    }
    fn done(note: impl Into<String>) -> Self {
        Self {
            performed: true,
            note: Some(note.into()),
            conflict_paths: Vec::new(),
        }
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
    let _ = git.create_branch(&branch, &run_main);

    // Create the station worktree if it isn't already registered.
    let wt_path = station_worktree_path(&root, slug, station);
    let already = git
        .list_worktrees()
        .map(|ws| ws.iter().any(|w| w.path == wt_path))
        .unwrap_or(false);
    if !already {
        if let Some(parent) = wt_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Attach the worktree to the already-created station branch.
        let opts = darkrun_git::CreateOptions {
            reference: Some(branch.clone()),
            new_branch: None,
        };
        let name = format!("{slug}-{station}");
        let _ = git.create_worktree(&name, &wt_path, &opts);
    }
    LifecycleOutcome::done(branch)
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
    if !git.branch_exists(&branch).unwrap_or(false) {
        return LifecycleOutcome::noop(format!("{branch} not found; nothing to land"));
    }
    if !git.branch_exists(&run_main).unwrap_or(false) {
        return LifecycleOutcome::noop(format!("{run_main} not found; cannot land"));
    }

    let wt_path = station_worktree_path(&root, slug, station);
    let wt_name = format!("{slug}-{station}");

    // Merge through a worktree checked out on run-main so the primary checkout
    // is never touched. Reuse a temporary run-main worktree as the merge target.
    let outcome = merge_into_branch(store, &git, &root, &run_main, &branch, slug, &format!(
        "darkrun: land station '{station}' -> {run_main}"
    ));

    // Whether the merge succeeded or not, retire the station worktree (its work
    // is captured on the branch). Remove the branch only on a clean land so a
    // conflicted station is left recoverable.
    let _ = git.remove_worktree(&wt_name, true);
    let _ = std::fs::remove_dir_all(&wt_path);

    if outcome.performed || git.is_ancestor(&branch, &run_main).unwrap_or(false) {
        // Landed (or already merged) → drop the station branch.
        let _ = delete_branch(&root, &branch);
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

/// Merge `source` into `target` through a temporary DETACHED worktree at
/// `target`'s commit, guarded by the engine-protected merge, then fast-update
/// the `target` branch ref to the merge result.
///
/// The worktree is detached (not checked out on `target`) so this works even
/// when `target` is the branch checked out in the primary working tree (the
/// run-main -> base case) — the primary checkout is never touched, mirroring the
/// reference's ephemeral-worktree merge. Cleans up the temporary worktree after.
fn merge_into_branch(
    _store: &StateStore,
    git: &Git,
    root: &Path,
    target: &str,
    source: &str,
    slug: &str,
    message: &str,
) -> LifecycleOutcome {
    // Already merged → clean no-op.
    if git.is_ancestor(source, target).unwrap_or(false) {
        return LifecycleOutcome::noop(format!("{source} already in {target}"));
    }

    // When `target` is the branch checked out in the PRIMARY tree (the run-main
    // -> base case where base is the operator's working branch), merge into the
    // primary checkout directly — a detached worktree can't fast-update the
    // checked-out branch ref (`branch -f` refuses the current branch). This is
    // the one place the working tree is intentionally advanced: the run is
    // landing on the base the operator asked for. The engine guard still holds
    // `.darkrun` state to the target side.
    let primary_branch = git.current_branch().ok().flatten();
    if primary_branch.as_deref() == Some(target) {
        // Refuse to clobber a dirty primary tree.
        if !git.is_clean().unwrap_or(false) {
            return LifecycleOutcome::noop(format!(
                "primary tree on {target} is dirty; skipping in-process land"
            ));
        }
        let result = engine_protected_merge(git, root, source, slug, message);
        return match result {
            Ok(o) if o.ok && o.performed => {
                LifecycleOutcome::done(format!("merged {source} -> {target}"))
            }
            Ok(o) if o.ok => {
                LifecycleOutcome::noop(format!("{source} already up to date with {target}"))
            }
            Ok(o) => LifecycleOutcome {
                performed: false,
                note: o
                    .message
                    .or_else(|| Some(format!("merge {source} -> {target} left conflicts"))),
                conflict_paths: o.conflict_paths,
            },
            Err(e) => LifecycleOutcome::noop(format!("merge {source} -> {target} failed: {e}")),
        };
    }

    let merge_wt = root
        .join(".darkrun")
        .join("worktrees")
        .join(slug)
        .join(format!("_merge-{}", sanitize(target)));
    let merge_wt_str = merge_wt.to_string_lossy().to_string();

    if let Some(parent) = merge_wt.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // Detached worktree at the target's current commit — works even when the
    // target branch is checked out in the primary tree.
    if git_at(root, &["worktree", "add", "--detach", &merge_wt_str, target]).is_err() {
        return LifecycleOutcome::noop(format!("could not create merge worktree for {target}"));
    }

    let result = engine_protected_merge(git, &merge_wt, source, slug, message);

    // On a clean merge, advance the target branch ref to the merge commit so the
    // land is durable, then tear down the temp worktree.
    let outcome = match result {
        Ok(o) if o.ok && o.performed => {
            // Resolve the merge worktree's HEAD and point `target` at it.
            match git_at(&merge_wt, &["rev-parse", "HEAD"]) {
                Ok(head) => {
                    let head = head.trim();
                    let _ = git_at(root, &["branch", "-f", target, head]);
                    LifecycleOutcome::done(format!("merged {source} -> {target}"))
                }
                Err(e) => LifecycleOutcome::noop(format!(
                    "merged {source} -> {target} but could not resolve HEAD: {e}"
                )),
            }
        }
        Ok(o) if o.ok => {
            LifecycleOutcome::noop(format!("{source} already up to date with {target}"))
        }
        Ok(o) => LifecycleOutcome {
            performed: false,
            note: o
                .message
                .or_else(|| Some(format!("merge {source} -> {target} left conflicts"))),
            conflict_paths: o.conflict_paths,
        },
        Err(e) => LifecycleOutcome::noop(format!("merge {source} -> {target} failed: {e}")),
    };

    // Tear down the temp worktree regardless of outcome.
    let _ = git_at(root, &["worktree", "remove", "--force", &merge_wt_str]);
    let _ = std::fs::remove_dir_all(&merge_wt);
    outcome
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
}
