//! Pure-Rust [`GitBackend`] over **gitoxide** (`gix`) — no C, no `git` CLI.
//!
//! Built incrementally behind the [`GitBackend`](crate::GitBackend) trait and
//! validated against the same real-git conformance fixtures as the libgit2 and
//! shell backends (they must AGREE). Operations gitoxide doesn't yet provide
//! (push/rebase/merge/worktree-create) return [`GitError::Unsupported`] until
//! their build-out phase lands, so the migration flips on one operation at a
//! time without ever breaking the working engine.

use std::path::{Path, PathBuf};

use crate::backend::{CreateOptions, GitBackend, MergeOutcome, WorktreeInfo};
use crate::error::{GitError, Result};

/// A pure-Rust git backend driven by gitoxide.
pub struct GixBackend {
    repo_root: PathBuf,
}

/// Map any gix error into our crate error.
fn gix_err(e: impl std::fmt::Display) -> GitError {
    GitError::Gix(e.to_string())
}

impl GixBackend {
    /// Open the git repository that contains `repo_root` (walks up, like
    /// `Repository::discover` / running `git` from a subdirectory).
    pub fn open(repo_root: impl AsRef<Path>) -> Result<Self> {
        let root = repo_root.as_ref();
        gix::discover(root).map_err(|_| GitError::NotARepo(root.to_path_buf()))?;
        Ok(Self {
            repo_root: root.to_path_buf(),
        })
    }

    /// The repository root this backend was opened against.
    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    /// Open the gix repository rooted at this backend's path.
    fn repo(&self) -> Result<gix::Repository> {
        gix::discover(&self.repo_root).map_err(|_| GitError::NotARepo(self.repo_root.clone()))
    }
}

impl GitBackend for GixBackend {
    // ── Reads (native in gitoxide) ───────────────────────────────────────────

    fn current_branch(&self) -> Result<Option<String>> {
        let repo = self.repo()?;
        // `head_name()` is `None` when HEAD is detached.
        let name = repo.head_name().map_err(gix_err)?;
        Ok(name.map(|n| n.shorten().to_string()))
    }

    fn is_clean(&self) -> Result<bool> {
        // `is_dirty()` is tracked-only; the engine (like `git status --porcelain`
        // / libgit2) treats UNTRACKED non-ignored files as dirty too. Drive the
        // full status with untracked files included and stop at the first change.
        let repo = self.repo()?;
        let iter = repo
            .status(gix::progress::Discard)
            .map_err(gix_err)?
            .untracked_files(gix::status::UntrackedFiles::Files)
            .into_iter(None)
            .map_err(gix_err)?;
        for item in iter {
            item.map_err(gix_err)?;
            return Ok(false);
        }
        Ok(true)
    }

    fn branch_exists(&self, name: &str) -> Result<bool> {
        let repo = self.repo()?;
        let full = format!("refs/heads/{name}");
        Ok(repo.try_find_reference(&full).map_err(gix_err)?.is_some())
    }

    // ── Not yet built (later phases) ─────────────────────────────────────────

    fn create_worktree(
        &self,
        _name: &str,
        _path: &Path,
        _opts: &CreateOptions,
    ) -> Result<WorktreeInfo> {
        Err(GitError::Unsupported("create_worktree"))
    }

    fn list_worktrees(&self) -> Result<Vec<WorktreeInfo>> {
        Err(GitError::Unsupported("list_worktrees"))
    }

    fn remove_worktree(&self, _name: &str, _force: bool) -> Result<()> {
        Err(GitError::Unsupported("remove_worktree"))
    }

    fn create_branch(&self, _name: &str, _from_ref: &str) -> Result<()> {
        Err(GitError::Unsupported("create_branch"))
    }

    fn is_ancestor(&self, maybe_ancestor: &str, descendant: &str) -> Result<bool> {
        let repo = self.repo()?;
        let a = repo
            .rev_parse_single(maybe_ancestor)
            .map_err(gix_err)?
            .object()
            .map_err(gix_err)?
            .peel_to_commit()
            .map_err(gix_err)?
            .id;
        let b = repo
            .rev_parse_single(descendant)
            .map_err(gix_err)?
            .object()
            .map_err(gix_err)?
            .peel_to_commit()
            .map_err(gix_err)?
            .id;
        if a == b {
            return Ok(true);
        }
        // `a` is an ancestor of `b` iff their merge base is `a` itself.
        let base = repo.merge_base(a, b).map_err(gix_err)?;
        Ok(base.detach() == a)
    }

    fn merge_no_commit(&self, _worktree_path: &Path, _source_ref: &str) -> Result<MergeOutcome> {
        Err(GitError::Unsupported("merge_no_commit"))
    }

    fn merge_in_progress(&self, worktree_path: &Path) -> Result<bool> {
        // A merge is in progress when MERGE_HEAD exists in the (worktree's) git
        // dir. `gix::open` resolves the correct git dir whether `worktree_path`
        // is the primary checkout or a linked worktree.
        let repo = gix::open(worktree_path).map_err(gix_err)?;
        Ok(repo.git_dir().join("MERGE_HEAD").exists())
    }

    fn checkout_paths(&self, _worktree_path: &Path, _from_ref: &str, _paths: &[String]) -> Result<()> {
        Err(GitError::Unsupported("checkout_paths"))
    }

    fn add_paths(&self, _worktree_path: &Path, _paths: &[String]) -> Result<()> {
        Err(GitError::Unsupported("add_paths"))
    }

    fn commit(&self, _worktree_path: &Path, _message: &str) -> Result<()> {
        Err(GitError::Unsupported("commit"))
    }

    fn ls_tree(&self, _worktree_path: &Path, _from_ref: &str, _prefix: &str) -> Result<Vec<String>> {
        Err(GitError::Unsupported("ls_tree"))
    }

    fn unresolved_paths(&self, _worktree_path: &Path) -> Result<Vec<String>> {
        Err(GitError::Unsupported("unresolved_paths"))
    }

    fn refs_have_identical_trees(&self, ref_a: &str, ref_b: &str) -> Result<bool> {
        // Contract: never errors — any resolution failure is `false` (the merge-
        // debt short-circuit is an optimization, not a correctness gate).
        let Ok(repo) = self.repo() else {
            return Ok(false);
        };
        let tree_of = |spec: &str| -> Option<gix::ObjectId> {
            Some(
                repo.rev_parse_single(spec)
                    .ok()?
                    .object()
                    .ok()?
                    .peel_to_commit()
                    .ok()?
                    .tree_id()
                    .ok()?
                    .detach(),
            )
        };
        match (tree_of(ref_a), tree_of(ref_b)) {
            (Some(a), Some(b)) => Ok(a == b),
            _ => Ok(false),
        }
    }

    fn push(&self, _worktree_path: &Path, _branch: &str) -> Result<()> {
        Err(GitError::Unsupported("push"))
    }

    fn fetch(&self, _worktree_path: &Path, _branch: &str) -> Result<()> {
        Err(GitError::Unsupported("fetch"))
    }

    fn rebase_onto(&self, _worktree_path: &Path, _upstream: &str) -> Result<()> {
        Err(GitError::Unsupported("rebase_onto"))
    }

    fn rebase_abort(&self, _worktree_path: &Path) -> Result<()> {
        Err(GitError::Unsupported("rebase_abort"))
    }
}
