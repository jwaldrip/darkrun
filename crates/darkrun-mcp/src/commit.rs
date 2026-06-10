//! The commit-and-push spine — runs commit and push **early and often**.
//!
//! `.darkrun/<run>/` state is TRACKED content: every state mutation commits on
//! whatever branch the engine is on and pushes it, so a run's progress is on
//! origin continuously — browseable from the website, durable across a restart,
//! and resumable from another machine. This module ports the predecessor's
//! hardest-won git machinery:
//!
//! - `commit_state` / `commit_state_if_dirty` — stage **only `.darkrun/`**,
//!   commit on the current branch, plain `git push` (best-effort, bounded,
//!   non-interactive). The narrow stage keeps engine commits free of unrelated
//!   user-code changes; the if-dirty gate avoids phantom empty commits that
//!   manufacture merge debt.
//! - `commit_all` — stage the WHOLE tree (`git add -A`), commit, push. For
//!   call sites that have already validated the dirty user code is in scope
//!   (a pass-beat advance commits the unit's work).
//! - `checkpoint_worktree` — commit + push a unit/fix worktree's branch
//!   mid-loop (refspec push with non-fast-forward recovery), so an in-progress
//!   loop survives a restart or a cross-machine pickup. Silent best-effort.
//! - `ensure_worktrees_gitignored` — `.darkrun/worktrees/` (the engine's
//!   worktree pool) is ignored **before any worktree exists**; everything else
//!   under `.darkrun/` stays tracked. We deliberately never write a broader
//!   ignore: state must commit.
//!
//! Every function here is **best-effort and non-fatal**: a push failure (no
//! remote, offline, rejected) reports in the outcome but never fails the
//! engine operation that triggered it.

use std::path::Path;

use darkrun_core::StateStore;
use darkrun_git::{Git, GitBackend};

/// What a commit-and-push attempt did. Mirrors the predecessor's
/// `{ committed, pushed, pushError }` result shape.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommitOutcome {
    /// Whether a commit was created.
    pub committed: bool,
    /// Whether the branch reached origin.
    pub pushed: bool,
    /// The push failure, when `pushed` is false but a push was attempted.
    pub push_error: Option<String>,
}

impl CommitOutcome {
    fn noop() -> Self {
        Self::default()
    }
}

/// The engine-state prefix staged by [`commit_state`].
const STATE_PREFIX: &str = ".darkrun";

/// The gitignore line that keeps the engine's worktree POOL (and only the
/// pool) out of the tracked state tree.
const WORKTREES_IGNORE: &str = ".darkrun/worktrees/";

/// Ensure `.darkrun/worktrees/` is gitignored at `repo_root` — appended once,
/// idempotent, BEFORE any worktree is created. Without this a bare
/// `git add -A` would try to stage the nested checkouts (and git would step on
/// its own linked worktrees). The rest of `.darkrun/` deliberately stays
/// tracked — state is published content.
pub fn ensure_worktrees_gitignored(repo_root: &Path) {
    let path = repo_root.join(".gitignore");
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let covered = existing.lines().any(|l| {
        let l = l.trim();
        l == WORKTREES_IGNORE
            || l == ".darkrun/worktrees"
            || l == "/.darkrun/worktrees/"
            || l == "/.darkrun/worktrees"
            // A user who ignored ALL of .darkrun covers the pool too (their
            // choice; state then degrades to local-only).
            || l == ".darkrun/"
            || l == ".darkrun"
            || l == "/.darkrun/"
            || l == "/.darkrun"
    });
    if covered {
        return;
    }
    let mut out = existing;
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(WORKTREES_IGNORE);
    out.push('\n');
    let _ = std::fs::write(&path, out);
}

/// The repo root for `store` (the parent of `.darkrun/`).
fn repo_root(store: &StateStore) -> std::path::PathBuf {
    store
        .root()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| store.root().to_path_buf())
}

/// Stage `.darkrun/` + commit on the current branch + push it. Use when the
/// caller just mutated engine state and the surrounding user code may
/// legitimately be dirty — the narrow stage keeps the commit clean of it.
/// No-op outside a git repo OR when state has no pending change: empty
/// commits move HEAD and manufacture phantom merge debt (the predecessor's
/// `--allow-empty` bug its IfDirty variant was built to fix), so EVERY state
/// commit is dirty-gated.
pub fn commit_state(store: &StateStore, message: &str) -> CommitOutcome {
    commit_and_push(store, STATE_PREFIX, message, true)
}

/// Alias of [`commit_state`] kept for call-site intent: use at idempotent
/// write sites where a no-op re-run is the EXPECTED common case.
pub fn commit_state_if_dirty(store: &StateStore, message: &str) -> CommitOutcome {
    commit_state(store, message)
}

/// Stage EVERYTHING (`git add -A`) + commit + push the current branch. Only
/// for call sites that have already validated the dirty user code belongs in
/// this commit (e.g. a pass-beat advance committing the unit's work).
pub fn commit_all(store: &StateStore, message: &str) -> CommitOutcome {
    commit_and_push(store, "", message, true)
}

/// The shared body: ensure the worktree-pool ignore, stage `prefix`, commit on
/// the CURRENT branch, push it.
fn commit_and_push(
    store: &StateStore,
    prefix: &str,
    message: &str,
    only_if_dirty: bool,
) -> CommitOutcome {
    let root = repo_root(store);
    let Ok(git) = Git::open(&root) else {
        return CommitOutcome::noop(); // not a git repo — filesystem mode
    };
    // The dirty gate runs BEFORE the ignore write: a clean tree stays a true
    // no-op (writing the ignore here would itself dirty the tree and mint a
    // phantom commit on every idle if-dirty call).
    if only_if_dirty
        && !git.status_dirty_under(&root, prefix).unwrap_or(false)
        && !git.status_dirty_under(&root, ".gitignore").unwrap_or(false)
    {
        return CommitOutcome::noop();
    }
    ensure_worktrees_gitignored(&root);
    if git.add_all_under(&root, prefix).is_err() {
        return CommitOutcome::noop();
    }
    // The worktree-pool ignore line (written above) rides along with state
    // commits so the repo's first state commit also publishes the ignore.
    let _ = git.add_all_under(&root, ".gitignore");
    if git.commit(&root, message).is_err() {
        return CommitOutcome::noop();
    }

    // Push the branch the engine is on. Detached HEAD (mid-recovery) skips the
    // push — there is no branch to publish.
    let Some(branch) = git.current_branch().ok().flatten() else {
        return CommitOutcome {
            committed: true,
            pushed: false,
            push_error: Some("detached HEAD — nothing to push".into()),
        };
    };
    match git.push(&root, &branch) {
        Ok(()) => CommitOutcome {
            committed: true,
            pushed: true,
            push_error: None,
        },
        Err(e) => CommitOutcome {
            committed: true,
            pushed: false,
            push_error: Some(e.to_string()),
        },
    }
}

/// Checkpoint an in-progress unit/fix worktree: commit any pending work on its
/// branch and push it to origin (refspec push with non-fast-forward recovery).
/// Restart / cross-machine durability — the loop's work isn't on the station
/// branch until the terminal land, so without this a restart on another
/// machine loses it. Silent best-effort: no worktree, no remote, and push
/// failures all degrade to the local worktree surviving a same-machine restart.
pub fn checkpoint_worktree(store: &StateStore, worktree: &Path, branch: &str, message: &str) {
    if !worktree.exists() {
        return;
    }
    let root = repo_root(store);
    let Ok(git) = Git::open(&root) else {
        return;
    };
    if git.status_dirty_under(worktree, "").unwrap_or(false)
        && git.add_all_under(worktree, "").is_ok()
    {
        let _ = git.commit(worktree, message);
    }
    let _ = crate::hosting::push_head_with_nff_recovery(&git, worktree, branch);
}

/// Push `branch` to origin from the repo root, best-effort with NFF recovery —
/// the `pushBranchToOrigin` analog for run-create / run-completion points.
pub fn push_branch(store: &StateStore, branch: &str) -> Option<String> {
    let root = repo_root(store);
    let Ok(git) = Git::open(&root) else {
        return Some("not a git repo".into());
    };
    match crate::hosting::push_head_with_nff_recovery(&git, &root, branch) {
        crate::hosting::PushOutcome::Pushed => None,
        crate::hosting::PushOutcome::Failed { note } => Some(note),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn sh_git(root: &Path, args: &[&str]) {
        let st = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .expect("git");
        assert!(st.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&st.stderr));
    }

    fn out_git(root: &Path, args: &[&str]) -> String {
        let st = Command::new("git").arg("-C").arg(root).args(args).output().expect("git");
        String::from_utf8_lossy(&st.stdout).trim().to_string()
    }

    /// A repo with one commit on `main`, plus a bare `origin` it can push to.
    /// Returns `(tempdir, work_root)`; the bare origin lives at
    /// `tempdir/origin.git` (assert remote state THERE — the pure-Rust
    /// send-pack updates the remote, not the local tracking ref).
    fn repo_with_origin() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("work");
        std::fs::create_dir_all(&root).unwrap();
        sh_git(&root, &["init", "-q", "-b", "main"]);
        sh_git(&root, &["config", "user.email", "t@darkrun.ai"]);
        sh_git(&root, &["config", "user.name", "t"]);
        std::fs::write(root.join("README.md"), "# t\n").unwrap();
        sh_git(&root, &["add", "-A"]);
        sh_git(&root, &["commit", "-q", "-m", "init"]);
        let bare = dir.path().join("origin.git");
        let st = Command::new("git")
            .args(["init", "-q", "--bare", bare.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(st.status.success());
        sh_git(&root, &["remote", "add", "origin", bare.to_str().unwrap()]);
        sh_git(&root, &["push", "-q", "-u", "origin", "main"]);
        (dir, root)
    }

    #[test]
    fn gitignore_gains_the_worktree_pool_once() {
        let dir = tempfile::tempdir().unwrap();
        ensure_worktrees_gitignored(dir.path());
        ensure_worktrees_gitignored(dir.path());
        let content = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert_eq!(content.matches(".darkrun/worktrees/").count(), 1);
        // A broader user ignore is respected, not duplicated or fought.
        std::fs::write(dir.path().join(".gitignore"), ".darkrun/\n").unwrap();
        ensure_worktrees_gitignored(dir.path());
        let content = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert_eq!(content.trim(), ".darkrun/");
    }

    #[test]
    fn commit_state_stages_only_state_and_pushes_the_current_branch() {
        let (_d, root) = repo_with_origin();
        let store = StateStore::new(&root);
        // Engine state + an UNRELATED dirty user file.
        std::fs::create_dir_all(root.join(".darkrun/r")).unwrap();
        std::fs::write(root.join(".darkrun/r/run.md"), "---\nfactory: software\n---\n# r\n").unwrap();
        std::fs::write(root.join("user-code.txt"), "untouched\n").unwrap();

        let out = commit_state(&store, "darkrun: state");
        assert!(out.committed, "state committed");
        assert!(out.pushed, "pushed to the bare origin: {:?}", out.push_error);

        // The commit carries the state file but NOT the user file.
        let files = out_git(&root, &["show", "--name-only", "--format=", "HEAD"]);
        assert!(files.contains(".darkrun/r/run.md"), "{files}");
        assert!(!files.contains("user-code.txt"), "{files}");
        // The BARE ORIGIN's main has the commit (the send-pack updates the
        // remote itself; the local `origin/main` tracking ref isn't the test).
        let bare = root.parent().unwrap().join("origin.git");
        assert_eq!(
            out_git(&root, &["rev-parse", "HEAD"]),
            out_git(&bare, &["rev-parse", "main"]),
        );
        // The user file is still dirty in the tree (not lost, not committed).
        let status = out_git(&root, &["status", "--porcelain"]);
        assert!(status.contains("user-code.txt"), "{status}");
    }

    #[test]
    fn commit_state_if_dirty_noops_on_a_clean_state_tree() {
        let (_d, root) = repo_with_origin();
        let store = StateStore::new(&root);
        let head_before = out_git(&root, &["rev-parse", "HEAD"]);
        let out = commit_state_if_dirty(&store, "darkrun: nothing");
        assert!(!out.committed, "clean tree → no phantom commit");
        assert_eq!(out_git(&root, &["rev-parse", "HEAD"]), head_before);
        // Dirty state → commits.
        std::fs::create_dir_all(root.join(".darkrun/r")).unwrap();
        std::fs::write(root.join(".darkrun/r/state.json"), "{}\n").unwrap();
        let out = commit_state_if_dirty(&store, "darkrun: state");
        assert!(out.committed);
    }

    #[test]
    fn commit_state_excludes_the_worktree_pool() {
        let (_d, root) = repo_with_origin();
        let store = StateStore::new(&root);
        std::fs::create_dir_all(root.join(".darkrun/worktrees/r/frame")).unwrap();
        std::fs::write(root.join(".darkrun/worktrees/r/frame/file.txt"), "x\n").unwrap();
        std::fs::create_dir_all(root.join(".darkrun/r")).unwrap();
        std::fs::write(root.join(".darkrun/r/run.md"), "# r\n").unwrap();

        let out = commit_state(&store, "darkrun: state");
        assert!(out.committed);
        let files = out_git(&root, &["show", "--name-only", "--format=", "HEAD"]);
        assert!(files.contains(".darkrun/r/run.md"), "{files}");
        assert!(!files.contains("worktrees"), "the pool never commits: {files}");
    }

    #[test]
    fn push_failure_reports_but_never_fails_the_commit() {
        // A repo with NO remote: the commit lands, the push reports.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        sh_git(&root, &["init", "-q", "-b", "main"]);
        sh_git(&root, &["config", "user.email", "t@darkrun.ai"]);
        sh_git(&root, &["config", "user.name", "t"]);
        std::fs::write(root.join("README.md"), "# t\n").unwrap();
        sh_git(&root, &["add", "-A"]);
        sh_git(&root, &["commit", "-q", "-m", "init"]);
        let store = StateStore::new(&root);
        std::fs::create_dir_all(root.join(".darkrun/r")).unwrap();
        std::fs::write(root.join(".darkrun/r/run.md"), "# r\n").unwrap();

        let out = commit_state(&store, "darkrun: state");
        assert!(out.committed, "the commit still lands");
        assert!(!out.pushed);
        assert!(out.push_error.is_some(), "the failure is reported");
    }

    #[test]
    fn run_start_publishes_the_run_to_origin_end_to_end() {
        use darkrun_core::domain::Mode;
        let (_d, root) = repo_with_origin();
        let store = StateStore::new(&root);
        crate::position::run_start(&store, "r", "software", None, Mode::Solo, "full")
            .expect("run starts");

        // The engine switched the main tree onto the run's branch…
        assert_eq!(out_git(&root, &["rev-parse", "--abbrev-ref", "HEAD"]), "darkrun/r/main");
        // …committed the run state on it…
        let files = out_git(&root, &["ls-tree", "-r", "--name-only", "darkrun/r/main"]);
        assert!(files.contains(".darkrun/r/run.md"), "{files}");
        assert!(files.contains(".darkrun/r/state.json"), "{files}");
        assert!(!files.contains("worktrees"), "the pool never publishes: {files}");
        // …and ORIGIN has the run: run-main with the state, plus the first
        // station's branch — the whole hierarchy is browseable remotely.
        let bare = root.parent().unwrap().join("origin.git");
        let origin_branches = out_git(&bare, &["branch", "--list"]);
        assert!(origin_branches.contains("darkrun/r/main"), "{origin_branches}");
        assert!(origin_branches.contains("darkrun/r/frame"), "{origin_branches}");
        let origin_files = out_git(&bare, &["ls-tree", "-r", "--name-only", "darkrun/r/main"]);
        assert!(origin_files.contains(".darkrun/r/state.json"), "{origin_files}");

        // A tick's writes publish too (commit early, commit often).
        let _ = crate::position::run_tick(&store, "r");
        let local = out_git(&root, &["rev-parse", "darkrun/r/main"]);
        let remote = out_git(&bare, &["rev-parse", "darkrun/r/main"]);
        assert_eq!(local, remote, "origin tracks every tick's state commit");
    }

    #[test]
    fn non_git_dirs_noop_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new(dir.path());
        let out = commit_state(&store, "darkrun: state");
        assert_eq!(out, CommitOutcome::default());
    }
}
