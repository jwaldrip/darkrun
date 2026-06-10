//! The [`GitBackend`] abstraction.
//!
//! darkrun talks to git through a trait, implemented by the pure-Rust
//! [`GixBackend`](crate::gix_backend::GixBackend) — gitoxide-backed, in-process,
//! with no C dependency and no `git`/`gh`/`glab` CLI shell-out. The trait
//! boundary keeps the engine decoupled from the git implementation and is the
//! seam the conformance suite drives against real `git`.

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

    /// Stage EVERY pending change under `prefix` — additions (untracked,
    /// gitignore-respected), modifications, and deletions — relative to the
    /// working tree at `worktree_path`. An empty prefix stages the whole tree.
    /// The status-driven `git add -A -- <prefix>` the engine's commit-state
    /// spine runs on every state mutation.
    fn add_all_under(&self, worktree_path: &Path, prefix: &str) -> Result<()>;

    /// Whether any pending change (tracked or untracked, gitignore-respected)
    /// exists under `prefix` (`git status --porcelain -- <prefix>` non-empty).
    /// Empty prefix = the whole tree. The dirty gate that keeps the engine's
    /// if-dirty state commit from minting phantom empty commits. Read-only.
    fn status_dirty_under(&self, worktree_path: &Path, prefix: &str) -> Result<bool>;

    /// Switch the MAIN working tree to `branch` (`git checkout <branch>`).
    ///
    /// The caller must ensure the tree is clean ([`GitBackend::is_clean`]):
    /// tracked files are replaced/removed to match the target, the index is
    /// rebuilt from the target tree, and `HEAD` becomes a symbolic ref to the
    /// branch. Never force-clobbers a dirty tree — callers surface that to the
    /// operator with a clear error instead.
    fn checkout_branch(&self, branch: &str) -> Result<()>;

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

    // ── Merge-debt / no-op loop guard (mechanic #4) ───────────────────────

    /// Whether `ref_a` and `ref_b` resolve to the **identical tree**
    /// (`<ref>^{tree}` equal for both). Read-only.
    ///
    /// The backing query for the merge-debt short-circuit: a `--no-ff` merge of
    /// two refs with identical trees still mints an empty commit, which makes
    /// the *other* side look "behind" and triggers an alternating no-op merge
    /// loop. Returns `false` on any git failure (missing ref, not-a-repo) — the
    /// equality check is an optimization, never a correctness gate, so callers
    /// fall through to the normal merge path on error.
    fn refs_have_identical_trees(&self, ref_a: &str, ref_b: &str) -> Result<bool>;

    // ── Remote push + NFF recovery primitives (mechanic #7) ───────────────
    //
    // Network ops over the pure-Rust transport (reqwest + rustls); push is the
    // hand-built send-pack. They run non-interactively so a missing credential
    // fails fast instead of hanging.

    /// Push `HEAD` of the working tree at `worktree_path` to `origin` as
    /// `refs/heads/<branch>`. A rejected ref surfaces as an error whose message
    /// preserves the remote's reason, so the caller can narrowly match a
    /// non-fast-forward rejection.
    fn push(&self, worktree_path: &Path, branch: &str) -> Result<()>;

    /// Fetch `branch` from `origin` into `refs/remotes/origin/<branch>`.
    fn fetch(&self, worktree_path: &Path, branch: &str) -> Result<()>;

    /// Rebase the working tree at `worktree_path` onto `upstream` (replay
    /// `upstream..HEAD`). The NFF-recovery rebase target is `origin/<branch>`.
    fn rebase_onto(&self, worktree_path: &Path, upstream: &str) -> Result<()>;

    /// Abort an in-progress rebase in the working tree at `worktree_path`,
    /// restoring the branch to `ORIG_HEAD`. Best-effort recovery after a failed
    /// NFF rebase.
    fn rebase_abort(&self, worktree_path: &Path) -> Result<()>;

    // ── Plumbing for the lifecycle / setup / gate read paths ──────────────

    /// Create a worktree named `name` at `path` with a DETACHED `HEAD` at
    /// `committish`'s commit (`git worktree add --detach <path> <committish>`).
    /// Always detached — even when `committish` is a branch — so it works when
    /// that branch is checked out elsewhere (the engine's merge-site worktree).
    fn create_worktree_detached(
        &self,
        name: &str,
        path: &Path,
        committish: &str,
    ) -> Result<WorktreeInfo>;

    /// The full commit id `HEAD` resolves to in the working tree at
    /// `worktree_path` (`git -C <wt> rev-parse HEAD`). Read-only.
    fn head_oid(&self, worktree_path: &Path) -> Result<String>;

    /// Force-update (or create) the local branch `name` to point at `committish`
    /// (`git branch -f <name> <committish>`). Used to fast-update a target branch
    /// to a merge commit produced in a detached worktree.
    fn set_branch_to(&self, name: &str, committish: &str) -> Result<()>;

    /// Delete the local branch `name` (`git branch -D <name>`). A no-op when the
    /// branch does not exist.
    fn delete_branch(&self, name: &str) -> Result<()>;

    /// The configured URL of remote `name` (`git remote get-url <name>`), or
    /// `None` when the remote is not configured. Read-only.
    fn remote_url(&self, name: &str) -> Result<Option<String>>;

    /// The default branch's short name, resolved from `origin/HEAD`
    /// (`git symbolic-ref refs/remotes/origin/HEAD`). `None` when unset.
    /// Read-only.
    fn default_branch(&self) -> Result<Option<String>>;

    /// A `git diff --stat <reference>` summary of the working tree against
    /// `reference` (the changed-files overview). Read-only.
    fn diff_stat(&self, reference: &str) -> Result<String>;

    /// The unified `git diff <reference>` of the working tree against
    /// `reference`. Read-only.
    fn diff(&self, reference: &str) -> Result<String>;
}
