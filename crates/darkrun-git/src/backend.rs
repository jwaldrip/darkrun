//! The [`GitBackend`] abstraction.
//!
//! darkrun talks to git through a trait so the implementation can be swapped:
//! the default [`Libgit2Backend`](crate::libgit2::Libgit2Backend) drives
//! everything in-process via libgit2, while the
//! [`ShellBackend`](crate::shell::ShellBackend) shells out to the `git`
//! executable. The shell backend exists as a fallback for the handful of
//! worktree operations libgit2 historically handles awkwardly across versions,
//! and as an escape hatch in environments where linking libgit2 is undesirable.

use std::path::{Path, PathBuf};

use crate::error::Result;

/// A registered git worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeInfo {
    /// The worktree's logical name (the directory name git registers it under).
    pub name: String,
    /// The absolute path to the worktree's working directory.
    pub path: PathBuf,
    /// The branch checked out in the worktree, if any (`None` when detached).
    pub branch: Option<String>,
    /// Whether the worktree is locked.
    pub locked: bool,
}

/// The outcome of an [`engine-protected merge`](crate::merge::engine_protected_merge)
/// or its underlying [`merge_no_commit`](GitBackend::merge_no_commit) primitive.
///
/// Mirrors the reference's `{ ok, performed, conflictFiles }` shape:
///
/// - `ok` — the merge resolved cleanly (or was a clean no-op). `false` only when
///   genuine, unresolved conflicts remain on agent (non-engine) content, or a
///   pre-merge refusal (dirty tree) blocked it.
/// - `performed` — a merge commit was actually minted. `false` for an
///   "already up to date" no-op.
/// - `conflict_paths` — the unresolved paths when `ok == false`; empty otherwise.
/// - `message` — a human-readable note (the git error or conflict summary) when
///   `ok == false`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MergeOutcome {
    /// Whether the merge resolved cleanly (no genuine conflicts, no hard refusal).
    pub ok: bool,
    /// Whether a merge commit was actually created (vs an up-to-date no-op).
    pub performed: bool,
    /// The unresolved (agent-content) conflict paths, when `ok == false`.
    pub conflict_paths: Vec<String>,
    /// A human-readable note when the merge did not resolve cleanly.
    pub message: Option<String>,
}

/// Options controlling how a worktree is created.
#[derive(Debug, Clone, Default)]
pub struct CreateOptions {
    /// The committish (branch, tag, or revision) to fork the worktree from.
    /// When `None`, the worktree forks from the repository `HEAD`.
    pub reference: Option<String>,
    /// When set, create (and check out) a new branch with this name in the
    /// worktree. When `None`, the worktree checks out `reference`/`HEAD`
    /// directly (detached when the reference is not a branch).
    pub new_branch: Option<String>,
}

/// The set of git worktree operations darkrun depends on.
///
/// Implementations MUST treat read-only queries (`list_worktrees`,
/// `current_branch`, `is_clean`) as non-mutating and side-effect free.
pub trait GitBackend {
    /// Create a worktree named `name` at `path`. See [`CreateOptions`].
    fn create_worktree(
        &self,
        name: &str,
        path: &Path,
        opts: &CreateOptions,
    ) -> Result<WorktreeInfo>;

    /// List every registered worktree (including the primary working tree).
    fn list_worktrees(&self) -> Result<Vec<WorktreeInfo>>;

    /// Remove the worktree named `name`. When `force` is true, remove it even
    /// if it contains uncommitted or untracked changes.
    fn remove_worktree(&self, name: &str, force: bool) -> Result<()>;

    /// The branch currently checked out in the repository's main working tree,
    /// or `None` when `HEAD` is detached.
    fn current_branch(&self) -> Result<Option<String>>;

    /// Whether the repository's working tree has no pending changes (no
    /// modified, staged, or untracked-but-not-ignored files).
    fn is_clean(&self) -> Result<bool>;

    // ── Branch + merge primitives ─────────────────────────────────────────
    //
    // The branch hierarchy (`darkrun/<slug>/main` + per-station
    // `darkrun/<slug>/<station>`) and its staged fan-in merges build on these.
    // `branch_exists` and `is_ancestor` are read-only queries; the rest mutate.

    /// Whether a local branch named `name` exists. Read-only.
    fn branch_exists(&self, name: &str) -> Result<bool>;

    /// Create the local branch `name` at `from_ref` (a branch, tag, or revision).
    /// Idempotent: a no-op when `name` already exists. Does not check it out.
    fn create_branch(&self, name: &str, from_ref: &str) -> Result<()>;

    /// Whether `maybe_ancestor` is an ancestor of (or equal to) `descendant` —
    /// the backing query for "is this branch already merged". Read-only.
    fn is_ancestor(&self, maybe_ancestor: &str, descendant: &str) -> Result<bool>;

    /// Merge `source_ref` into the branch checked out at `worktree_path` with
    /// `--no-ff --no-commit`, leaving the merge staged (and `MERGE_HEAD` set)
    /// for the engine-protected restore + commit to follow. An "already up to
    /// date" source resolves to a clean no-op ([`MergeOutcome::performed`] =
    /// false). A pre-merge refusal or conflict leaves the merge in progress for
    /// the caller to inspect / restore.
    fn merge_no_commit(&self, worktree_path: &Path, source_ref: &str) -> Result<MergeOutcome>;

    /// Whether a merge is in progress in the working tree at `worktree_path`
    /// (i.e. `MERGE_HEAD` is set). Read-only.
    fn merge_in_progress(&self, worktree_path: &Path) -> Result<bool>;

    /// Restore `paths` in the working tree at `worktree_path` from `from_ref`
    /// (`git checkout <from_ref> -- <paths>`), overwriting the index + worktree
    /// regardless of conflict state — the engine-protected restore primitive.
    fn checkout_paths(&self, worktree_path: &Path, from_ref: &str, paths: &[String]) -> Result<()>;

    /// Stage `paths` in the working tree at `worktree_path` (`git add -- <paths>`).
    fn add_paths(&self, worktree_path: &Path, paths: &[String]) -> Result<()>;

    /// Commit the staged state in the working tree at `worktree_path` with
    /// `message` (`git commit --no-edit -m <message>`), finishing a merge.
    fn commit(&self, worktree_path: &Path, message: &str) -> Result<()>;

    /// The tracked paths under `prefix` at `from_ref`
    /// (`git ls-tree -r --name-only <from_ref> -- <prefix>`) — the enumeration
    /// the engine-protected restore walks. Read-only.
    fn ls_tree(&self, worktree_path: &Path, from_ref: &str, prefix: &str) -> Result<Vec<String>>;

    /// The unresolved (conflicted) paths in the working tree at `worktree_path`
    /// (`git diff --name-only --diff-filter=U`). Read-only.
    fn unresolved_paths(&self, worktree_path: &Path) -> Result<Vec<String>>;
}
