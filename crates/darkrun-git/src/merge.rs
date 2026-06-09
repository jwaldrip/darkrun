//! The engine-protected merge.
//!
//! The branch hierarchy (`darkrun/<slug>/main` accumulating fully-verified
//! per-station branches) only holds if a merge can never silently revert the
//! engine's own workflow state. A 3-way merge auto-resolves a file that changed
//! on only one side with NO conflict marker — so a station branch carrying a
//! frozen-at-fork snapshot of the `.darkrun/<run>` tree would silently clobber
//! the run-main's authoritative state on every land.
//!
//! The guard, ported from the reference's `engineProtectedMergeInCwd` +
//! `restoreEngineStateFromBase`:
//!
//! 1. merge `source_ref` with `--no-ff --no-commit` (the merge is staged, not
//!    committed);
//! 2. if no merge started, report an up-to-date no-op (or a hard refusal);
//! 3. re-assert EVERY engine-owned `.darkrun/<run>` path from the target's
//!    pre-merge `HEAD` — force-holding the merge-INTO side regardless of whether
//!    a conflict marker was left or the path was silently auto-resolved;
//! 4. re-scan unresolved paths — any remaining are genuine conflicts on agent
//!    (non-engine) content, which we surface;
//! 5. commit.
//!
//! Crate-level guarantee: `darkrun/<slug>/main` only ever carries fully-verified,
//! state-consistent stations.

use std::path::Path;

use crate::backend::{GitBackend, MergeOutcome};
use crate::error::Result;

/// The engine-owned state prefix. Everything under `.darkrun/<run>/` is workflow
/// state the merge must hold to the target (merge-INTO) side.
pub const ENGINE_STATE_PREFIX: &str = ".darkrun";

/// Whether `rel` is an engine-owned workflow-state path for `run_slug`.
///
/// The run document, derived `state.json`, units, feedback, reflections, drift,
/// witnesses, and proof all live under `.darkrun/<run>/` and are owned by the
/// engine on the target branch. Holding the WHOLE run subtree is deliberately
/// broad — a station branch never legitimately authors run state (that's the
/// manager writing on run-main / the worktree's own state path), so any
/// divergence the worktree side carries is stale and must lose.
pub fn is_engine_owned_state_path(rel: &str, run_slug: &str) -> bool {
    let base = format!("{ENGINE_STATE_PREFIX}/{run_slug}/");
    rel.starts_with(&base)
}

/// Merge `source_ref` into the branch checked out at `target_worktree`,
/// holding `.darkrun/<run_slug>` state to the target side, and commit with
/// `message`. See the module docs for the staged sequence.
///
/// Non-fatal contract: any unresolved agent-content conflict is reported as a
/// non-ok [`MergeOutcome`] rather than an error; the caller routes it. A clean
/// no-op (already up to date) returns `ok = true, performed = false`.
pub fn engine_protected_merge(
    backend: &dyn GitBackend,
    target_worktree: &Path,
    source_ref: &str,
    run_slug: &str,
    message: &str,
) -> Result<MergeOutcome> {
    // (1) stage the merge.
    let outcome = backend.merge_no_commit(target_worktree, source_ref)?;
    // (2) no merge started → up-to-date no-op or hard pre-merge refusal.
    if !outcome.performed {
        return Ok(outcome);
    }

    // (3) re-assert the target's authoritative engine state over any silent
    // auto-resolve to the source side. `HEAD` is still the pre-merge target
    // (the merge is `--no-commit`).
    restore_engine_state_from_base(backend, target_worktree, "HEAD", run_slug)?;

    // (4) any remaining unresolved paths are genuine conflicts on agent content.
    let conflicts = backend.unresolved_paths(target_worktree)?;
    if !conflicts.is_empty() {
        let summary = format!(
            "Merge {source_ref} left conflicts in {} file(s): {}.",
            conflicts.len(),
            conflicts.join(", ")
        );
        return Ok(MergeOutcome {
            ok: false,
            performed: false,
            conflict_paths: conflicts,
            message: Some(summary),
        });
    }

    // (5) commit the engine-settled merge.
    backend.commit(target_worktree, message)?;
    Ok(MergeOutcome {
        ok: true,
        performed: true,
        conflict_paths: Vec::new(),
        message: None,
    })
}

/// Re-assert `base_ref` as the source of truth for every engine-owned
/// `.darkrun/<run_slug>` state path after a `--no-commit` merge.
///
/// Enumerates from `base_ref` (the merge's "ours") so a file the source only
/// MODIFIED or DELETED is restored, then force-checks-out + stages each
/// engine-owned path — closing the silent-auto-resolve hole a conflict-only
/// `checkout --ours` would leave open. Best-effort per path.
fn restore_engine_state_from_base(
    backend: &dyn GitBackend,
    target_worktree: &Path,
    base_ref: &str,
    run_slug: &str,
) -> Result<()> {
    let prefix = format!("{ENGINE_STATE_PREFIX}/{run_slug}");
    let tracked = backend.ls_tree(target_worktree, base_ref, &prefix)?;
    let owned: Vec<String> = tracked
        .into_iter()
        .filter(|rel| is_engine_owned_state_path(rel, run_slug))
        .collect();
    if owned.is_empty() {
        return Ok(());
    }
    backend.checkout_paths(target_worktree, base_ref, &owned)?;
    backend.add_paths(target_worktree, &owned)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{has_no_merge_debt, Git, GitBackend};
    use std::path::PathBuf;
    use std::process::Command;
    use tempfile::TempDir;

    /// Build a throwaway repo with one commit on `main`.
    fn init_repo() -> (TempDir, PathBuf) {
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path().to_path_buf();
        let git = |args: &[&str]| {
            let status = Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .status()
                .expect("run git");
            assert!(status.success(), "git {args:?} failed");
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["config", "user.email", "test@darkrun.ai"]);
        git(&["config", "user.name", "darkrun test"]);
        std::fs::write(root.join("README.md"), "# smoke\n").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "init"]);
        (dir, root)
    }

    fn git_in(root: &std::path::Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .status()
            .expect("git");
        assert!(status.success(), "git {args:?} failed");
    }

    /// Identical-tree branches compare equal; a diverged branch does not.
    fn refs_identical_trees(open: fn(&std::path::Path) -> Result<Git>) {
        let (_d, root) = init_repo();
        // Two branches forked at the SAME commit → identical trees.
        git_in(&root, &["branch", "a", "main"]);
        git_in(&root, &["branch", "b", "main"]);
        let git = open(&root).expect("open");
        assert!(git.refs_have_identical_trees("a", "b").unwrap());
        assert!(git.refs_have_identical_trees("a", "main").unwrap());

        // Advance `b` with a new commit → trees diverge.
        git_in(&root, &["checkout", "-q", "b"]);
        std::fs::write(root.join("new.txt"), "diverge\n").unwrap();
        git_in(&root, &["add", "-A"]);
        git_in(&root, &["commit", "-q", "-m", "b work"]);
        assert!(!git.refs_have_identical_trees("a", "b").unwrap());

        // A missing ref resolves to "not identical" (false), never an error.
        assert!(!git.refs_have_identical_trees("a", "does-not-exist").unwrap());
    }

    #[test]
    fn gix_refs_identical_trees() {
        refs_identical_trees(|p| Git::open(p));
    }

    /// `has_no_merge_debt` is true for identical trees, true for an ancestor,
    /// and false for a genuine divergence (where a real merge has work to do).
    #[test]
    fn merge_debt_predicate() {
        let (_d, root) = init_repo();
        // identical-tree fork.
        git_in(&root, &["branch", "twin", "main"]);
        // ancestor: main is an ancestor of `ahead` once `ahead` advances.
        git_in(&root, &["checkout", "-q", "-b", "ahead"]);
        std::fs::write(root.join("a.txt"), "a\n").unwrap();
        git_in(&root, &["add", "-A"]);
        git_in(&root, &["commit", "-q", "-m", "ahead work"]);
        // diverged: a separate branch off main with its own commit.
        git_in(&root, &["checkout", "-q", "-b", "side", "main"]);
        std::fs::write(root.join("s.txt"), "s\n").unwrap();
        git_in(&root, &["add", "-A"]);
        git_in(&root, &["commit", "-q", "-m", "side work"]);

        let git = Git::open(&root).expect("open");

        // Identical trees → no debt.
        assert!(has_no_merge_debt(&git, "twin", "main"));
        // main is an ancestor of `ahead` → merging main into ahead is a no-op.
        assert!(has_no_merge_debt(&git, "main", "ahead"));
        // `side` diverged from `ahead` → there IS debt (a real merge).
        assert!(!has_no_merge_debt(&git, "side", "ahead"));
    }

    #[test]
    fn merge_in_progress_marker_set() {
        use crate::is_merge_in_progress;
        let (_d, root) = init_repo();
        // No merge in flight on a clean repo.
        assert!(!is_merge_in_progress(&root));

        // Plant each marker in $GIT_DIR and confirm the broad predicate fires.
        let git_dir = root.join(".git");
        for marker in ["MERGE_HEAD", "REBASE_HEAD", "CHERRY_PICK_HEAD", "REVERT_HEAD"] {
            std::fs::write(git_dir.join(marker), "ref\n").unwrap();
            assert!(is_merge_in_progress(&root), "{marker} should register");
            std::fs::remove_file(git_dir.join(marker)).unwrap();
        }
        for dir_marker in ["rebase-merge", "rebase-apply"] {
            std::fs::create_dir_all(git_dir.join(dir_marker)).unwrap();
            assert!(is_merge_in_progress(&root), "{dir_marker} should register");
            std::fs::remove_dir_all(git_dir.join(dir_marker)).unwrap();
        }
        // Cleared again.
        assert!(!is_merge_in_progress(&root));
    }

    #[test]
    fn engine_owned_predicate_scopes_to_run() {
        assert!(is_engine_owned_state_path(".darkrun/r/state.json", "r"));
        assert!(is_engine_owned_state_path(
            ".darkrun/r/units/u1.md",
            "r"
        ));
        // A different run's state is not owned by this merge.
        assert!(!is_engine_owned_state_path(".darkrun/other/state.json", "r"));
        // Agent content outside .darkrun is never engine-owned.
        assert!(!is_engine_owned_state_path("src/main.rs", "r"));
        // The bare prefix (no trailing run) is not a per-run state path.
        assert!(!is_engine_owned_state_path(".darkrun/settings.yml", "r"));
    }
}
