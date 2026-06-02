//! The libgit2-backed [`GitBackend`] implementation (the default).

use std::path::{Path, PathBuf};

use git2::{Repository, StatusOptions, WorktreeAddOptions};

use crate::backend::{CreateOptions, GitBackend, MergeOutcome, WorktreeInfo};
use crate::error::{GitError, Result};
use crate::shell::ShellBackend;

/// A [`GitBackend`] driven entirely in-process by libgit2.
pub struct Libgit2Backend {
    repo_root: PathBuf,
}

impl Libgit2Backend {
    /// Open the git repository that contains `repo_root`. Errors with
    /// [`GitError::NotARepo`] when no repository can be discovered.
    pub fn open(repo_root: impl AsRef<Path>) -> Result<Self> {
        let root = repo_root.as_ref();
        // `discover` walks up to find the enclosing repo, matching what a user
        // running `git` from a subdirectory would expect.
        Repository::discover(root).map_err(|_| GitError::NotARepo(root.to_path_buf()))?;
        Ok(Self {
            repo_root: root.to_path_buf(),
        })
    }

    /// The repository root this backend was opened against.
    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    fn repo(&self) -> Result<Repository> {
        Repository::discover(&self.repo_root)
            .map_err(|_| GitError::NotARepo(self.repo_root.clone()))
    }

    /// The shell backend used for the engine-protected merge trio.
    ///
    /// The merge / restore / commit primitives operate inside an ephemeral
    /// station-or-run worktree and must match the reference's exact, battle-
    /// tested `git` semantics (a `--no-ff --no-commit` merge, a `checkout <ref>
    /// -- <path>` restore, a `commit --no-edit`). Driving that trio through
    /// libgit2's index/merge machinery is both fiddly and version-sensitive
    /// across worktrees, so we route it through the shell path — the libgit2
    /// backend stays authoritative only for the read-side branch/ancestor
    /// queries it does cleanly in-process.
    fn shell(&self) -> Result<ShellBackend> {
        ShellBackend::open(&self.repo_root)
    }
}

impl GitBackend for Libgit2Backend {
    fn create_worktree(
        &self,
        name: &str,
        path: &Path,
        opts: &CreateOptions,
    ) -> Result<WorktreeInfo> {
        let repo = self.repo()?;

        if repo.find_worktree(name).is_ok() {
            return Err(GitError::WorktreeExists(name.to_string()));
        }

        // Resolve the reference to fork from. libgit2's worktree-add checks out
        // whatever reference we point it at; when the caller wants a brand-new
        // branch we create it first (off the resolved base) and point the
        // worktree at that, mirroring `git worktree add -b <new> <path> <base>`.
        let base = match &opts.reference {
            Some(r) => repo.revparse_single(r)?,
            None => repo.head()?.peel(git2::ObjectType::Any)?,
        };

        // Resolve the branch reference to attach the worktree to (if any). It
        // must outlive the `WorktreeAddOptions` borrow, so bind it here.
        let (reference, created_branch) = if let Some(new_branch) = &opts.new_branch {
            let commit = base.peel_to_commit()?;
            let branch = repo.branch(new_branch, &commit, false)?;
            (Some(branch.into_reference()), Some(new_branch.clone()))
        } else {
            // No new branch: if the resolved base is itself a local branch,
            // check it out so the worktree is attached rather than detached.
            match &opts.reference {
                Some(r) => match repo.find_branch(r, git2::BranchType::Local) {
                    Ok(branch) => (Some(branch.into_reference()), Some(r.clone())),
                    Err(_) => (None, None),
                },
                None => (None, None),
            }
        };

        let mut add_opts = WorktreeAddOptions::new();
        if let Some(reference) = &reference {
            add_opts.reference(Some(reference));
        }

        let worktree = repo.worktree(name, path, Some(&add_opts))?;

        Ok(WorktreeInfo {
            name: name.to_string(),
            path: worktree.path().to_path_buf(),
            branch: created_branch,
            locked: false,
        })
    }

    fn list_worktrees(&self) -> Result<Vec<WorktreeInfo>> {
        let repo = self.repo()?;
        let mut out = Vec::new();

        // The primary working tree is not enumerated by `repo.worktrees()`;
        // include it explicitly so callers see the complete picture.
        if let Some(workdir) = repo.workdir() {
            out.push(WorktreeInfo {
                name: "(main)".to_string(),
                path: workdir.to_path_buf(),
                branch: head_branch(&repo),
                locked: false,
            });
        }

        for name in repo.worktrees()?.iter().flatten() {
            let wt = match repo.find_worktree(name) {
                Ok(wt) => wt,
                Err(_) => continue,
            };
            let path = wt.path().to_path_buf();
            // Open the linked worktree to read its checked-out branch.
            let branch = Repository::open(&path).ok().and_then(|r| head_branch(&r));
            let locked = matches!(wt.is_locked(), Ok(git2::WorktreeLockStatus::Locked(_)));
            out.push(WorktreeInfo {
                name: name.to_string(),
                path,
                branch,
                locked,
            });
        }

        Ok(out)
    }

    fn remove_worktree(&self, name: &str, force: bool) -> Result<()> {
        let repo = self.repo()?;
        let worktree = repo
            .find_worktree(name)
            .map_err(|_| GitError::WorktreeNotFound(name.to_string()))?;

        // Remove the working directory on disk, then prune the admin entry.
        let path = worktree.path().to_path_buf();
        if path.exists() {
            std::fs::remove_dir_all(&path).map_err(|source| GitError::Io {
                path: path.clone(),
                source,
            })?;
        }

        let mut prune = git2::WorktreePruneOptions::new();
        prune.valid(true).working_tree(true);
        if force {
            prune.locked(true);
        }
        worktree.prune(Some(&mut prune))?;
        Ok(())
    }

    fn current_branch(&self) -> Result<Option<String>> {
        let repo = self.repo()?;
        Ok(head_branch(&repo))
    }

    fn is_clean(&self) -> Result<bool> {
        let repo = self.repo()?;
        let mut opts = StatusOptions::new();
        opts.include_untracked(true)
            .include_ignored(false)
            .exclude_submodules(true);
        let statuses = repo.statuses(Some(&mut opts))?;
        Ok(statuses.is_empty())
    }

    fn branch_exists(&self, name: &str) -> Result<bool> {
        let repo = self.repo()?;
        let exists = repo.find_branch(name, git2::BranchType::Local).is_ok();
        Ok(exists)
    }

    fn create_branch(&self, name: &str, from_ref: &str) -> Result<()> {
        let repo = self.repo()?;
        // Idempotent: leave an existing branch in place.
        if repo.find_branch(name, git2::BranchType::Local).is_ok() {
            return Ok(());
        }
        let commit = repo.revparse_single(from_ref)?.peel_to_commit()?;
        repo.branch(name, &commit, false)?;
        Ok(())
    }

    fn is_ancestor(&self, maybe_ancestor: &str, descendant: &str) -> Result<bool> {
        let repo = self.repo()?;
        let anc = repo.revparse_single(maybe_ancestor)?.peel_to_commit()?.id();
        let desc = repo.revparse_single(descendant)?.peel_to_commit()?.id();
        if anc == desc {
            return Ok(true);
        }
        // `graph_descendant_of(desc, anc)` is true iff anc is a strict ancestor
        // of desc — the equality case above covers the inclusive end.
        Ok(repo.graph_descendant_of(desc, anc).unwrap_or(false))
    }

    // The mutating merge trio routes through the shell backend (see `shell()`).

    fn merge_no_commit(&self, worktree_path: &Path, source_ref: &str) -> Result<MergeOutcome> {
        self.shell()?.merge_no_commit(worktree_path, source_ref)
    }

    fn merge_in_progress(&self, worktree_path: &Path) -> Result<bool> {
        self.shell()?.merge_in_progress(worktree_path)
    }

    fn checkout_paths(&self, worktree_path: &Path, from_ref: &str, paths: &[String]) -> Result<()> {
        self.shell()?.checkout_paths(worktree_path, from_ref, paths)
    }

    fn add_paths(&self, worktree_path: &Path, paths: &[String]) -> Result<()> {
        self.shell()?.add_paths(worktree_path, paths)
    }

    fn commit(&self, worktree_path: &Path, message: &str) -> Result<()> {
        self.shell()?.commit(worktree_path, message)
    }

    fn ls_tree(&self, worktree_path: &Path, from_ref: &str, prefix: &str) -> Result<Vec<String>> {
        self.shell()?.ls_tree(worktree_path, from_ref, prefix)
    }

    fn unresolved_paths(&self, worktree_path: &Path) -> Result<Vec<String>> {
        self.shell()?.unresolved_paths(worktree_path)
    }
}

/// The short branch name of `repo`'s `HEAD`, or `None` when detached.
fn head_branch(repo: &Repository) -> Option<String> {
    let head = repo.head().ok()?;
    if head.is_branch() {
        head.shorthand().map(|s| s.to_string())
    } else {
        None
    }
}
